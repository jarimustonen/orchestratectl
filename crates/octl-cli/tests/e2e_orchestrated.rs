//! End-to-end harness for the autonomous-**orchestrated** lifecycle (issue
//! `orchestrated-children-hang-pending`).
//!
//! Sibling of `e2e_spinoff.rs`, but for a parent-pointed `--kind orchestrated`
//! child of an `--kind orchestrate` driver. This is the regression gate for the
//! bug where orchestrated children stayed `status: pending` with no teardown
//! because the orchestrate driver spawned no supervisor to adopt them.
//!
//! The driver run now spawns a detached **driver supervisor**; it adopts the
//! child (sees `child.spawned`), forks the child's own supervisor, and that
//! child supervisor rolls the child up to a terminal status and tears down its
//! worktree — the same guarantee spinoff already had.
//!
//! Two round-trips are driven:
//!   1. `orchestrated_child_merge_round_trip_reaches_done_and_tears_down` — the
//!      child closes via `run merge`; assert it reaches `done`, the worker node
//!      is registered, and the worktree dir is gone.
//!   2. `orchestrated_child_cancel_tears_down_worktree` — the reporter's
//!      regression: `run cancel` on the child must ALSO tear down the worktree.
//!
//! Unlike `e2e_spinoff` (which neutralizes git with a nonexistent `GIT_BIN`),
//! these assert the worktree is physically removed, so they drive a REAL git
//! repo + linked worktree through real `git` and only stub the two shell-out
//! boundaries (`OCTL_CREATE_SH`, `OCTL_MERGE_SH`) plus a nonexistent `TMUX_BIN`
//! (tmux teardown is a lenient no-op; the watchdog tmux probe reads `Unknown`
//! so PID liveness keeps the stub agent `Alive` until the close terminalizes
//! the node).

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
/// so a panicking assertion never leaks the `sleep` past the test.
struct AgentGuard {
    pid: i32,
}

impl Drop for AgentGuard {
    fn drop(&mut self) {
        if self.pid > 0 {
            unsafe { libc::kill(self.pid, libc::SIGKILL) };
        }
    }
}

fn write_exec(path: &Path, body: &str) {
    std::fs::write(path, body).unwrap();
    let mut perms = std::fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).unwrap();
}

/// Run `git <args>` in `cwd`, asserting success.
fn git(cwd: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .is_ok_and(|o| o.status.success());
    assert!(ok, "git {args:?} failed in {}", cwd.display());
}

/// Init a real repo with one commit on `main`, branch `integration` off it, and
/// a linked worktree on a fresh `child_branch` forked from `integration`.
/// Returns `(repo, worktree)`. The cleanup path resolves the main worktree from
/// the linked one, so this is enough for a real `worktree remove` / `branch -D`.
fn init_git_worktree(scratch: &Path, integration: &str, child_branch: &str) -> (PathBuf, PathBuf) {
    let repo = scratch.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.email", "t@example.com"]);
    git(&repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join("README"), "x").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-qm", "init"]);
    git(&repo, &["branch", integration, "main"]);
    let wt = scratch.join("worktree");
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            child_branch,
            wt.to_str().unwrap(),
            integration,
        ],
    );
    (repo, wt)
}

/// A stub `create.sh` that spawns a long-lived agent (`sleep`), records its pid,
/// and emits the [`SpawnOutcome`] envelope pointing `worktree_path` at the real
/// linked worktree and `branch` at the real child branch (so `run merge` and the
/// supervisor's teardown act on a genuine git worktree).
fn write_create_sh(
    scratch: &Path,
    worktree: &Path,
    agent_pid_file: &Path,
    branch: &str,
) -> PathBuf {
    let p = scratch.join("fake-create.sh");
    let body = format!(
        r#"#!/bin/bash
bash -c 'echo done; exec sleep 120' </dev/null >/dev/null 2>&1 &
agent_pid=$!
echo "$agent_pid" > '{pidfile}'
cat <<EOF
{{"schema_version":1,"type":"orchestrated","branch":"{branch}","worktree_path":"{worktree}","tmux_window":"{branch}","agent_pid_hint":$agent_pid,"workmux_session":"e2e","tmux_socket":null,"tmux_session":"e2e","tmux_window_id":"@1"}}
EOF
"#,
        pidfile = agent_pid_file.display(),
        branch = branch,
        worktree = worktree.display(),
    );
    write_exec(&p, &body);
    p
}

/// A stub `merge.sh` that records its argv and exits 0 — enough for `run merge`
/// to submit the terminal `node.report`; the merge mechanics are out of scope.
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

fn read_events(events: &Path) -> Vec<Value> {
    std::fs::read_to_string(events)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .collect()
}

fn event_kinds(events: &Path) -> Vec<String> {
    read_events(events)
        .into_iter()
        .filter_map(|v| v["kind"].as_str().map(str::to_string))
        .collect()
}

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

/// Poll until `path` no longer exists, or `timeout` elapses. Returns true if it
/// was removed in time.
fn wait_for_gone(path: &Path, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if !path.exists() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Poll a run's `manifest.json` until `status` reaches one of `wants`, or
/// `timeout` elapses. Returns the final observed status string (or whatever the
/// last read produced).
fn wait_for_status(manifest: &Path, wants: &[&str], timeout: Duration) -> String {
    let deadline = Instant::now() + timeout;
    loop {
        let status = std::fs::read(manifest)
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|v| v["status"].as_str().map(str::to_string))
            .unwrap_or_default();
        if wants.contains(&status.as_str()) || Instant::now() >= deadline {
            return status;
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

/// Spawn an orchestrate driver (with a real driver supervisor) and one
/// orchestrated child pointed at it, materialized against a real git worktree.
/// Blocks until the child's own supervisor has booted, so the caller can close
/// the child (merge or cancel) knowing a supervisor is live to tear it down.
///
/// Returns the child run id, the child's events path + manifest path, the real
/// worktree path, and the adopted stub agent guard.
struct Spawned {
    child_id: String,
    child_events: PathBuf,
    child_manifest: PathBuf,
    worktree: PathBuf,
    _agent: AgentGuard,
}

fn spawn_driver_and_child(home: &TestHome, scratch: &Path, no_tmux: &Path) -> Spawned {
    let integration = "orchestrate/e2e-integration";
    let child_branch = "wt/e2e-orchestrated";
    let (_repo, worktree) = init_git_worktree(scratch, integration, child_branch);
    let agent_pid_file = scratch.join("agent.pid");
    let create_sh = write_create_sh(scratch, &worktree, &agent_pid_file, child_branch);

    // 1. Driver run. `--kind orchestrate` is skip-materialize (no create.sh)
    //    but now boots a detached driver supervisor. TMUX_BIN is nonexistent so
    //    the inherited-down probes are no-ops; GIT_BIN defaults to real `git`
    //    so the child supervisor's teardown actually removes the worktree.
    let driver = run_ok(
        Command::new(env!("CARGO_BIN_EXE_orchestratectl"))
            .env("ORCHESTRATECTL_HOME", home.path())
            .env("TMUX_BIN", no_tmux)
            .args([
                "--output",
                "json",
                "run",
                "create",
                "--kind",
                "orchestrate",
                "--title",
                "e2e-campaign",
                "--source-branch",
                "main",
            ]),
    );
    let driver_id = driver["data"]["run_id"].as_str().unwrap().to_string();
    assert_eq!(driver["data"]["node_id"], "n-0001");
    // The driver now reports a real supervisor PID, not the old
    // "orchestrator-in-main-conversation" note.
    assert!(
        driver["data"]["supervisor"].as_u64().is_some(),
        "orchestrate driver must spawn a supervisor; got {}",
        driver["data"]["supervisor"]
    );

    let driver_events = home
        .path()
        .join("runs")
        .join(&driver_id)
        .join("events.jsonl");
    assert!(
        wait_for_event(
            &driver_events,
            "supervisor.started",
            Duration::from_secs(20)
        ),
        "driver supervisor never started; events: {:?}",
        event_kinds(&driver_events)
    );

    // 2. Orchestrated child pointed at the driver. Real materialization (stub
    //    create.sh) → emits child.spawned on the driver log + node.created on
    //    the child log. The driver supervisor adopts it and forks the child
    //    supervisor.
    let child = run_ok(
        Command::new(env!("CARGO_BIN_EXE_orchestratectl"))
            .env("ORCHESTRATECTL_HOME", home.path())
            .env("OCTL_CREATE_SH", &create_sh)
            .env("TMUX_BIN", no_tmux)
            .args([
                "--output",
                "json",
                "run",
                "create",
                "--kind",
                "orchestrated",
                "--title",
                "c1",
                "--task",
                "trivial change",
                "--source-branch",
                integration,
                "--parent-run-id",
                &driver_id,
                "--parent-node-id",
                "n-0001",
            ]),
    );
    let child_id = child["data"]["run_id"].as_str().unwrap().to_string();
    assert_eq!(child["data"]["kind"], "orchestrated");
    assert_eq!(child["data"]["node_id"], "n-0001");
    assert_eq!(child["data"]["lifecycle"], "autonomous");
    assert_eq!(
        child["data"]["supervisor"], "delegated-to-parent-supervisor",
        "child delegates supervisor creation to the driver supervisor"
    );

    let agent_pid: i32 = std::fs::read_to_string(&agent_pid_file)
        .expect("create.sh recorded the agent pid")
        .trim()
        .parse()
        .expect("agent pid is an integer");
    let agent = AgentGuard { pid: agent_pid };

    let child_events = home
        .path()
        .join("runs")
        .join(&child_id)
        .join("events.jsonl");
    let child_manifest = home
        .path()
        .join("runs")
        .join(&child_id)
        .join("manifest.json");

    // 3. Wait for the driver supervisor to fork the child's own supervisor — its
    //    boot is the proof a teardown actor is live before we close the child.
    assert!(
        wait_for_event(&child_events, "supervisor.started", Duration::from_secs(20)),
        "child supervisor never started (driver failed to adopt the child); \
         driver events: {:?}; child events: {:?}",
        event_kinds(&driver_events),
        event_kinds(&child_events),
    );

    Spawned {
        child_id,
        child_events,
        child_manifest,
        worktree,
        _agent: agent,
    }
}

/// The happy path: the orchestrated child closes via `run merge`, the child
/// supervisor rolls it up to `done`, the worker node stays registered, and the
/// worktree is physically torn down.
#[test]
#[file_serial(key, path => "/tmp/octl-test-supervise.lock")]
fn orchestrated_child_merge_round_trip_reaches_done_and_tears_down() {
    let home = TestHome::new();
    let scratch = TempDir::new().unwrap();
    let no_tmux = scratch.path().join("no-such-tmux");
    let merge_sh = write_merge_sh(scratch.path());

    let s = spawn_driver_and_child(&home, scratch.path(), &no_tmux);

    // Close the child via `run merge` → appends the terminal `node.report`.
    let merged = run_ok(
        Command::new(env!("CARGO_BIN_EXE_orchestratectl"))
            .env("ORCHESTRATECTL_HOME", home.path())
            .env("OCTL_MERGE_SH", &merge_sh)
            .args(["--output", "json", "run", "merge", &s.child_id]),
    );
    assert_eq!(merged["data"]["merged"], true);

    // The child supervisor rolls the run up to `done` and tears down.
    let status = wait_for_status(
        &s.child_manifest,
        &["done", "failed"],
        Duration::from_secs(30),
    );
    assert_eq!(
        status,
        "done",
        "child did not reach done; events: {:?}",
        event_kinds(&s.child_events)
    );

    // The worker node IS registered (the reporter's `nodes: []` must not recur).
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(&s.child_manifest).unwrap()).unwrap();
    assert_eq!(
        manifest["node_count"].as_u64(),
        Some(1),
        "worker node must be registered: {manifest}"
    );
    assert_eq!(manifest["kind"], "orchestrated");

    // The terminal report was the explicit-merge one.
    assert!(
        read_events(&s.child_events)
            .into_iter()
            .any(|v| v["kind"] == "node.report" && v["data"]["via"] == "explicit-merge"),
        "node.report was not via explicit-merge"
    );

    // The worktree is physically gone (supervisor teardown, real git).
    assert!(
        wait_for_gone(&s.worktree, Duration::from_secs(15)),
        "worktree {} was not torn down by the supervisor",
        s.worktree.display()
    );

    // `run wait` returns cleanly now that the child is terminal.
    let waited = run_ok(
        Command::new(env!("CARGO_BIN_EXE_orchestratectl"))
            .env("ORCHESTRATECTL_HOME", home.path())
            .args(["--output", "json", "run", "wait", &s.child_id]),
    );
    assert_eq!(waited["data"]["runs"][0]["status"], "done");
}

/// The reporter's regression: `run cancel` on an orchestrated child must ALSO
/// tear down the worktree (it pushed the run terminal but left the worktree +
/// window behind). With a live child supervisor, the terminal `run.status` is
/// picked up and `cleanup_terminal_nodes` fires (orchestrated is autonomous).
#[test]
#[file_serial(key, path => "/tmp/octl-test-supervise.lock")]
fn orchestrated_child_cancel_tears_down_worktree() {
    let home = TestHome::new();
    let scratch = TempDir::new().unwrap();
    let no_tmux = scratch.path().join("no-such-tmux");

    let s = spawn_driver_and_child(&home, scratch.path(), &no_tmux);

    // Cancel the child: pushes its run terminal (cancelled).
    let cancelled = run_ok(
        Command::new(env!("CARGO_BIN_EXE_orchestratectl"))
            .env("ORCHESTRATECTL_HOME", home.path())
            .args(["--output", "json", "run", "cancel", &s.child_id]),
    );
    assert!(cancelled["data"]["cancelled_nodes"]
        .as_array()
        .is_some_and(|a| !a.is_empty()));

    let status = wait_for_status(&s.child_manifest, &["cancelled"], Duration::from_secs(30));
    assert_eq!(
        status,
        "cancelled",
        "child was not cancelled; events: {:?}",
        event_kinds(&s.child_events)
    );

    // The worktree must be torn down on cancel, not just on merge.
    assert!(
        wait_for_gone(&s.worktree, Duration::from_secs(15)),
        "worktree {} was not torn down after run cancel",
        s.worktree.display()
    );
}
