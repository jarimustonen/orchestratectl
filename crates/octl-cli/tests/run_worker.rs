//! Integration tests for the `run-worker` launcher shim (design.md §2.1 / A1,
//! issue `thin-exit-status-launcher`).
//!
//! The shim wraps an autonomous worker: it launches the wrapped command, waits
//! on it, records the true exit status as a durable `worker.exited` event under
//! the run lock, and exits with the worker's OWN status. These tests drive the
//! real binary end-to-end against a seeded run dir (no live supervisor), so they
//! assert both the propagated exit code and the recorded event.

use std::path::Path;
use std::process::Command;

use octl_core::{append_and_apply_event, ensure_root, NodeId, RunPaths};
use serde_json::{json, Value};
use tempfile::TempDir;

const RUN_ID: &str = "01jxwd0000000000000000000w";

fn bin(home: &TempDir) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_orchestratectl"));
    c.env("ORCHESTRATECTL_HOME", home.path());
    c
}

/// Seed a minimal autonomous run (manifest + one `n-0001` node) directly via the
/// core append path — no `run create`, so no supervisor is spawned and the test
/// stays deterministic.
fn seed_run(home: &Path) -> RunPaths {
    ensure_root(home).unwrap();
    let dir = home.join("runs").join(RUN_ID);
    std::fs::create_dir_all(&dir).unwrap();
    let paths = RunPaths::new(dir, RUN_ID).unwrap();
    append_and_apply_event(
        &paths,
        "run.created",
        None,
        None,
        json!({ "kind": "spinoff", "lifecycle": "autonomous", "title": "t" }),
    )
    .unwrap();
    append_and_apply_event(
        &paths,
        "node.created",
        Some(&NodeId::parse_str("n-0001").unwrap()),
        None,
        json!({ "kind": "spinoff" }),
    )
    .unwrap();
    paths
}

/// The one `worker.exited` event on the run's log, parsed.
fn worker_exited_event(paths: &RunPaths) -> Value {
    let log = std::fs::read_to_string(paths.events()).unwrap();
    let line = log
        .lines()
        .find(|l| l.contains("\"worker.exited\""))
        .expect("a worker.exited event must be recorded");
    serde_json::from_str(line).unwrap()
}

#[test]
fn shim_records_clean_exit_and_propagates_zero() {
    let home = TempDir::new().unwrap();
    let paths = seed_run(home.path());

    let status = bin(&home)
        .args(["run-worker", RUN_ID, "n-0001", "--", "sh", "-c", "exit 0"])
        .status()
        .expect("spawn shim");
    assert_eq!(
        status.code(),
        Some(0),
        "shim propagates the worker's exit 0"
    );

    let ev = worker_exited_event(&paths);
    assert_eq!(ev["node_id"], "n-0001");
    assert_eq!(ev["data"]["exit_code"], 0);
    assert!(ev["data"].get("signal").is_none());
}

#[test]
fn shim_records_nonzero_exit_and_propagates_it() {
    let home = TempDir::new().unwrap();
    let paths = seed_run(home.path());

    let status = bin(&home)
        .args(["run-worker", RUN_ID, "n-0001", "--", "sh", "-c", "exit 7"])
        .status()
        .expect("spawn shim");
    assert_eq!(
        status.code(),
        Some(7),
        "shim propagates the worker's non-zero code"
    );

    let ev = worker_exited_event(&paths);
    assert_eq!(ev["data"]["exit_code"], 7);
}

#[test]
fn shim_records_signal_death() {
    let home = TempDir::new().unwrap();
    let paths = seed_run(home.path());

    // The wrapped worker kills itself with SIGKILL (9).
    let status = bin(&home)
        .args([
            "run-worker",
            RUN_ID,
            "n-0001",
            "--",
            "sh",
            "-c",
            "kill -9 $$",
        ])
        .status()
        .expect("spawn shim");
    // Shell convention: a signal death propagates as 128 + signal.
    assert_eq!(status.code(), Some(128 + 9));

    let ev = worker_exited_event(&paths);
    assert_eq!(ev["data"]["signal"], 9);
    assert!(ev["data"].get("exit_code").is_none());
}

#[test]
fn shim_rejects_unknown_run() {
    let home = TempDir::new().unwrap();
    // A valid-shaped but nonexistent run id.
    let out = bin(&home)
        .args(["run-worker", RUN_ID, "n-0001", "--", "sh", "-c", "exit 0"])
        .output()
        .expect("spawn shim");
    assert!(
        !out.status.success(),
        "an unknown run must fail, not launch"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("run_not_found"),
        "expected run_not_found, got: {stderr}"
    );
}

#[test]
fn shim_rejects_unknown_node() {
    let home = TempDir::new().unwrap();
    let _paths = seed_run(home.path());
    // The run exists but the node does not — the shim must refuse, not launch a
    // worker whose exit would fold to nothing (a `worker.exited` for an unknown
    // node is a silent reducer no-op).
    let out = bin(&home)
        .args(["run-worker", RUN_ID, "n-9999", "--", "sh", "-c", "exit 0"])
        .output()
        .expect("spawn shim");
    assert!(
        !out.status.success(),
        "an unknown node must fail, not launch"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("node_not_found"),
        "expected node_not_found, got: {stderr}"
    );
}

#[test]
fn shim_records_a_told_failure_when_the_worker_cannot_launch() {
    let home = TempDir::new().unwrap();
    let paths = seed_run(home.path());
    // The wrapped program does not exist → the worker cannot be launched at all.
    let out = bin(&home)
        .args([
            "run-worker",
            RUN_ID,
            "n-0001",
            "--",
            "/nonexistent/orchestratectl-no-such-worker",
        ])
        .output()
        .expect("spawn shim");
    assert!(!out.status.success(), "a spawn failure exits non-zero");

    // Crucially, a told FAILURE fact is still recorded — the supervisor must not
    // have to fall back to pid-guessing for a worker that never started.
    let ev = worker_exited_event(&paths);
    assert_eq!(ev["data"]["exit_code"], 127);
}

#[test]
fn shim_survives_sigterm_and_records_the_childs_true_exit() {
    let home = TempDir::new().unwrap();
    let paths = seed_run(home.path());

    // The shim wraps a child that sleeps, then exits cleanly. We SIGTERM the SHIM
    // while the child is still running: the shim must ignore it, keep waiting, and
    // record the child's real clean exit (design.md §2.1 — the told fact must
    // survive a foreground-group / teardown signal).
    let mut child = bin(&home)
        .args([
            "run-worker",
            RUN_ID,
            "n-0001",
            "--",
            "sh",
            "-c",
            "sleep 2; exit 0",
        ])
        .spawn()
        .expect("spawn shim");
    let shim_pid = child.id().to_string();

    // Give the shim time to install its signal-ignore and spawn the child.
    std::thread::sleep(std::time::Duration::from_millis(400));
    let killed = Command::new("kill")
        .args(["-TERM", &shim_pid])
        .status()
        .expect("send SIGTERM")
        .success();
    assert!(killed, "SIGTERM to the shim should be delivered");

    let status = child.wait().expect("await shim");
    assert_eq!(
        status.code(),
        Some(0),
        "the shim ignored SIGTERM, waited for the child, and propagated its clean exit"
    );
    let ev = worker_exited_event(&paths);
    assert_eq!(ev["data"]["exit_code"], 0);
}
