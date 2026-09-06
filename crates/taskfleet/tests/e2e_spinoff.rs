//! One complete native materialization → supervise → merge → teardown roundtrip.

use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use serde_json::Value;
use tempfile::TempDir;

mod common;
use common::{NativeSpawnTools, TestHome};

fn executable(path: &Path, body: &str) {
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn run_ok(command: &mut Command) -> Value {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn wait_event(path: &Path, kind: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if std::fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .any(|line| {
                serde_json::from_str::<Value>(line).is_ok_and(|event| event["kind"] == kind)
            })
        {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!(
        "timed out waiting for {kind}: {}",
        std::fs::read_to_string(path).unwrap_or_default()
    );
}

#[test]
fn native_spinoff_round_trip_reaches_done_and_tears_down() {
    let home = TestHome::new();
    let scratch = TempDir::new().unwrap();
    let tools = NativeSpawnTools::new();
    let worker = scratch.path().join("worker.sh");
    executable(&worker, "#!/bin/sh\nexec /bin/sleep 120\n");
    std::fs::write(
        home.path().join("config.toml"),
        format!(
            r#"[profiles.e2e]
description="e2e"
capability="fast"
residency="local"
agents=[{{harness="pi",command=["{}"],telemetry="worker-v1"}}]
[profile]
default="e2e"
"#,
            worker.display()
        ),
    )
    .unwrap();
    let merge = scratch.path().join("merge.sh");
    executable(&merge, "#!/bin/sh\nexit 0\n");
    let worktree = tools.worktree("worktree");

    let mut create = Command::new(env!("CARGO_BIN_EXE_taskfleet"));
    create
        .env("TASKFLEET_HOME", home.path())
        .env("HOME", home.path());
    tools.configure(&mut create, &worktree, "headless");
    let created = run_ok(create.args([
        "--output",
        "json",
        "run",
        "create",
        "--kind",
        "spinoff",
        "--headless",
        "--title",
        "native e2e",
        "--task",
        "do work",
    ]));
    let run_id = created["data"]["run_id"].as_str().unwrap();
    let run_dir = home.path().join("runs").join(run_id);
    let events = run_dir.join("events.jsonl");
    wait_event(&events, "supervisor.started");

    let merged = run_ok(
        Command::new(env!("CARGO_BIN_EXE_taskfleet"))
            .env("TASKFLEET_HOME", home.path())
            .env("HOME", home.path())
            .env("TASKFLEET_MERGE_SH", &merge)
            .args(["--output", "json", "run", "merge", run_id]),
    );
    assert_eq!(merged["data"]["merged"], true);
    wait_event(&events, "supervisor.exited");
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(run_dir.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["status"], "done");
    assert!(
        !worktree.exists(),
        "supervisor teardown removed the worktree"
    );
    let kinds: Vec<String> = std::fs::read_to_string(events)
        .unwrap()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|event| event["kind"].as_str().map(str::to_owned))
        .collect();
    for required in [
        "run.created",
        "node.created",
        "supervisor.started",
        "node.report",
        "run.status",
        "supervisor.exited",
    ] {
        assert!(
            kinds.iter().any(|kind| kind == required),
            "missing {required}: {kinds:?}"
        );
    }
}
