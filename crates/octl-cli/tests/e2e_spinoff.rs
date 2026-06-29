//! End-to-end harness for the autonomous-spinoff lifecycle (issue
//! `spinoff-e2e-harness`).
//!
//! This drives ONE real round-trip of the production spawn → supervise →
//! merge → teardown loop on every `cargo test` run, so the cleanup / merge /
//! supervisor paths get a CI gate instead of relying on hand-crafted live
//! smokes:
//!
//!   `run create --kind spinoff --headless`  (real create.sh stub + real
//!       detached supervisor) → a minimal agent process stays alive →
//!   `run merge <id>`  (real merge.sh stub) submits the terminal
//!       `node.report` → the supervisor rolls the run up to `done`, runs
//!       teardown, and exits emitting `supervisor.exited`.
//!
//! The two shell-out boundaries are stubbed through the SAME binary-override
//! hooks production already exposes — `OCTL_CREATE_SH` (see
//! `run::spawn::create_sh_path`) and `OCTL_MERGE_SH` (see
//! `run::merge::materialize_merge_sh`) — so no real tmux/workmux/git is
//! touched. The supervisor's own tmux/git teardown is neutralized by pointing
//! `TMUX_BIN`/`GIT_BIN` at nonexistent paths: a spawn error makes the tmux
//! liveness probe `Unknown` (so PID liveness governs and the live agent stays
//! `Alive` until the merge terminalizes it) and makes every cleanup step a
//! lenient no-op.
//!
//! Per CLAUDE.md the test isolates `~/.orchestratectl` with the `TestHome`
//! fixture (which reaps the detached supervisor on drop) and never leaks the
//! stub agent process (killed by `AgentGuard` on drop, panic-safe).

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde_json::Value;
use serial_test::file_serial;
use tempfile::TempDir;

mod common;
use common::TestHome;

/// A real OS process the stub create.sh spawned as the "agent". Killed on drop
/// so a panicking assertion (or an early return) never leaks the `sleep` past
/// the test — the `TestHome` reaper only knows about supervisor pids.
struct AgentGuard {
    pid: i32,
}

impl Drop for AgentGuard {
    fn drop(&mut self) {
        if self.pid > 0 {
            // SIGKILL is fine: the stub agent is an inert `sleep`, nothing to
            // flush. ESRCH (already gone) is harmless.
            unsafe { libc::kill(self.pid, libc::SIGKILL) };
        }
    }
}

/// Write `body` to `path` and mark it executable (0o755).
fn write_exec(path: &Path, body: &str) {
    std::fs::write(path, body).unwrap();
    let mut perms = std::fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).unwrap();
}

/// A stub `create.sh` that materializes a minimal lifecycle: it spawns a
/// long-lived agent (`sleep`, detached from create.sh's stdout pipe so
/// `run create` does not block on EOF), records that agent's pid to
/// `agent_pid_file` for the test's reaper, and emits the structured
/// [`SpawnOutcome`] envelope production parses on exit 0.
///
/// The agent sleeps far longer than the test needs — it only has to outlive
/// the merge so the watchdog keeps the node `Alive` and the *merge* (not a
/// synthesized agent-death report) is what terminalizes the node as `Done`.
fn write_create_sh(
    scratch: &Path,
    worktree: &Path,
    agent_pid_file: &Path,
    branch: &str,
) -> PathBuf {
    let p = scratch.join("fake-create.sh");
    let body = format!(
        r#"#!/bin/bash
# E2E stub create.sh — spawn a minimal agent, record its pid, emit envelope.
# Redirect every std fd away from create.sh's stdout pipe so `run create`
# reads our envelope and then sees EOF immediately (the agent does NOT hold
# the pipe open for its whole sleep).
bash -c 'echo done; exec sleep 120' </dev/null >/dev/null 2>&1 &
agent_pid=$!
echo "$agent_pid" > '{pidfile}'
cat <<EOF
{{"schema_version":1,"type":"spinoff","branch":"{branch}","worktree_path":"{worktree}","tmux_window":"{branch}","agent_pid_hint":$agent_pid,"workmux_session":"e2e","tmux_socket":null,"tmux_session":"e2e","tmux_window_id":"@1"}}
EOF
"#,
        pidfile = agent_pid_file.display(),
        branch = branch,
        worktree = worktree.display(),
    );
    write_exec(&p, &body);
    p
}

/// A stub `merge.sh` that records its argv (one line) to `<dir>/merge.log` and
/// exits 0 — the merge mechanics themselves are out of scope here; we only
/// need a clean exit so `run merge` submits the terminal `node.report`.
fn write_merge_sh(dir: &Path) -> PathBuf {
    let p = dir.join("fake-merge.sh");
    let log = dir.join("merge.log");
    let body = format!(
        "#!/bin/bash\nprintf '%s ' \"$@\" >> '{log}'\nprintf '\\n' >> '{log}'\nexit 0\n",
        log = log.display(),
    );
    write_exec(&p, &body);
    p
}

/// Parse `events.jsonl`, tolerating a torn final line the supervisor may be
/// mid-write on (we read this file while a live supervisor appends to it).
fn read_events(events: &Path) -> Vec<Value> {
    std::fs::read_to_string(events)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .collect()
}

/// The ordered list of event `kind`s recorded so far.
fn event_kinds(events: &Path) -> Vec<String> {
    read_events(events)
        .into_iter()
        .filter_map(|v| v["kind"].as_str().map(str::to_string))
        .collect()
}

/// Poll `events.jsonl` until an event of `kind` appears, or `timeout` elapses.
/// Returns true if it appeared.
fn wait_for_event(events: &Path, kind: &str, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if event_kinds(events).iter().any(|k| k == kind) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn run_ok(cmd: &mut Command) -> Value {
    let out = cmd.output().expect("spawn");
    assert!(
        out.status.success(),
        "exit={:?} stderr={}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("stdout is valid JSON")
}

/// Drive the full autonomous-spinoff round-trip and assert the canonical event
/// sequence, terminal manifest, and projection counts — then prove no
/// supervisor or agent process leaked.
#[test]
#[file_serial(key, path => "/tmp/octl-test-supervise.lock")]
fn spinoff_round_trip_reaches_done_and_tears_down() {
    let home = TestHome::new();
    let scratch = TempDir::new().unwrap();
    let worktree = scratch.path().join("worktree");
    std::fs::create_dir_all(&worktree).unwrap();
    let agent_pid_file = scratch.path().join("agent.pid");
    let branch = "wt/e2e-spinoff";

    let create_sh = write_create_sh(scratch.path(), &worktree, &agent_pid_file, branch);
    let merge_sh = write_merge_sh(scratch.path());
    // Nonexistent binaries: the supervisor inherits these from `run create`, so
    // its tmux liveness probe is `Unknown` (PID liveness governs → live agent
    // stays Alive until the merge) and every teardown step is a lenient no-op.
    let no_tmux = scratch.path().join("no-such-tmux");
    let no_git = scratch.path().join("no-such-git");

    // 1. Create the spinoff: real materialization (fake create.sh) + a real
    //    detached supervisor. Headless so no foreground PTY is consumed.
    let created = run_ok(
        Command::new(env!("CARGO_BIN_EXE_orchestratectl"))
            .env("ORCHESTRATECTL_HOME", home.path())
            .env("OCTL_CREATE_SH", &create_sh)
            .env("TMUX_BIN", &no_tmux)
            .env("GIT_BIN", &no_git)
            .args([
                "--output",
                "json",
                "run",
                "create",
                "--kind",
                "spinoff",
                "--headless",
                "--title",
                "e2e",
                "--task",
                "echo done",
            ]),
    );
    let run_id = created["data"]["run_id"].as_str().unwrap().to_string();
    assert_eq!(created["data"]["kind"], "spinoff");
    assert_eq!(created["data"]["node_id"], "n-0001");
    assert_eq!(created["data"]["lifecycle"], "autonomous");

    // Adopt the stub agent so it is reaped even if an assertion below panics.
    let agent_pid: i32 = std::fs::read_to_string(&agent_pid_file)
        .expect("create.sh recorded the agent pid")
        .trim()
        .parse()
        .expect("agent pid is an integer");
    let _agent = AgentGuard { pid: agent_pid };

    let events = home.path().join("runs").join(&run_id).join("events.jsonl");

    // 2. Wait (bounded) for the detached supervisor to boot and announce itself
    //    before we merge — this fixes the canonical ordering (supervisor.started
    //    precedes node.report) and proves the supervisor is live to roll up.
    assert!(
        wait_for_event(&events, "supervisor.started", Duration::from_secs(15)),
        "supervisor never emitted supervisor.started; events: {:?}",
        event_kinds(&events)
    );

    // 3. Merge to close: the fake merge.sh exits 0, so `run merge` appends the
    //    terminal `node.report` (via: explicit-merge) that completes the node.
    let merged = run_ok(
        Command::new(env!("CARGO_BIN_EXE_orchestratectl"))
            .env("ORCHESTRATECTL_HOME", home.path())
            .env("OCTL_MERGE_SH", &merge_sh)
            .args(["--output", "json", "run", "merge", &run_id]),
    );
    assert_eq!(merged["data"]["merged"], true);
    assert_eq!(merged["data"]["branch"], branch);

    // 4. The supervisor sees the node terminal, rolls the run up to `done`,
    //    tears down (lenient no-ops here), and exits cleanly.
    assert!(
        wait_for_event(&events, "supervisor.exited", Duration::from_secs(30)),
        "supervisor never exited; events: {:?}",
        event_kinds(&events)
    );

    // ---- Assert the canonical lifecycle event sequence (as a subsequence:
    // the supervisor may interleave watchdog/state housekeeping). ----
    let kinds = event_kinds(&events);
    let expected = [
        "run.created",
        "node.created",
        "supervisor.started",
        "node.report",
        "run.status",
        "supervisor.exited",
    ];
    let mut idx = 0usize;
    for k in &kinds {
        if idx < expected.len() && k == expected[idx] {
            idx += 1;
        }
    }
    assert_eq!(
        idx,
        expected.len(),
        "events did not contain the canonical lifecycle sequence {expected:?} in order; got {kinds:?}"
    );

    // The rolled-up run.status carried `done`.
    let run_status_done = read_events(&events)
        .into_iter()
        .any(|v| v["kind"] == "run.status" && v["data"]["status"] == "done");
    assert!(
        run_status_done,
        "run.status was not `done`; events: {kinds:?}"
    );

    // The terminal report was the explicit-merge one.
    let report_via_merge = read_events(&events)
        .into_iter()
        .any(|v| v["kind"] == "node.report" && v["data"]["via"] == "explicit-merge");
    assert!(report_via_merge, "node.report was not via explicit-merge");

    // ---- Manifest reached Done; projection counts match. ----
    let manifest: Value = serde_json::from_slice(
        &std::fs::read(home.path().join("runs").join(&run_id).join("manifest.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["status"], "done", "manifest status: {manifest}");
    assert_eq!(manifest["kind"], "spinoff");
    assert_eq!(manifest["lifecycle"], "autonomous");
    assert_eq!(
        manifest["node_count"].as_u64(),
        Some(1),
        "exactly one node: {manifest}"
    );

    // The node projection itself is terminal Done.
    let node: Value = serde_json::from_slice(
        &std::fs::read(
            home.path()
                .join("runs")
                .join(&run_id)
                .join("nodes")
                .join("n-0001.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(node["status"], "done", "node status: {node}");

    // ---- No leaked supervisor: on clean exit it removes its own pid file
    // immediately after appending `supervisor.exited`. Poll briefly for the
    // removal rather than racing that microsecond gap. (`TestHome::drop` would
    // reap a survivor, but a clean run must leave nothing for it to reap.) ----
    let pid_file = home
        .path()
        .join("runs")
        .join(&run_id)
        .join("supervisor.pid");
    let deadline = Instant::now() + Duration::from_secs(5);
    while pid_file.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        !pid_file.exists(),
        "supervisor.pid should be removed on clean exit"
    );
}
