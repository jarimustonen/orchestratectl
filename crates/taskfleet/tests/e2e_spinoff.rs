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

/// Read the supervisor pid recorded in `<run-dir>/supervisor.pid` (the first
/// whitespace token — the file is `"<pid> <start_time>"`).
fn read_supervisor_pid(pid_file: &Path) -> Option<i32> {
    let s = std::fs::read_to_string(pid_file).ok()?;
    s.split_whitespace().next()?.parse::<i32>().ok()
}

/// Poll until `kill(pid, 0)` reports the process gone (or `timeout` elapses).
fn wait_for_process_gone(pid: i32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if unsafe { libc::kill(pid, 0) } != 0 {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Poll `manifest.json` until its `status` equals `want` (or `timeout` elapses).
fn wait_for_manifest_status(manifest: &Path, want: &str, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(bytes) = std::fs::read(manifest) {
            if let Ok(v) = serde_json::from_slice::<Value>(&bytes) {
                if v["status"] == want {
                    return true;
                }
            }
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
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
        Command::new(env!("CARGO_BIN_EXE_taskfleet"))
            .env("TASKFLEET_HOME", home.path())
            .env("HOME", home.path())
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
        Command::new(env!("CARGO_BIN_EXE_taskfleet"))
            .env("TASKFLEET_HOME", home.path())
            .env("HOME", home.path())
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

/// Run `git <args>` in `cwd`, asserting success — for the real-git teardown
/// tests below (which drive the supervisor's actual `git worktree remove` /
/// `git branch -{d,D}` against a real repo instead of the nonexistent-`GIT_BIN`
/// no-op the merge round-trip uses).
fn git(cwd: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("spawn git")
        .success();
    assert!(ok, "git {args:?} failed in {}", cwd.display());
}

/// Init a real repo (one commit on `main`) with a linked worktree on `branch`
/// carrying one further "agent work" commit that is NOT merged into `main`.
/// Returns `(repo, worktree)`. The supervisor's real-git teardown resolves the
/// main worktree from the linked one, so this is enough for a full round-trip.
fn init_real_repo_with_committed_work(scratch: &Path, branch: &str) -> (PathBuf, PathBuf) {
    let repo = scratch.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.email", "t@example.com"]);
    git(&repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join("README"), "x").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-qm", "init"]);
    let wt = scratch.join("agent-wt");
    git(
        &repo,
        &["worktree", "add", "-q", "-b", branch, wt.to_str().unwrap()],
    );
    // The agent commits real, unmerged work on its branch.
    std::fs::write(wt.join("fix.rs"), "agent work").unwrap();
    git(&wt, &["add", "-A"]);
    git(&wt, &["commit", "-qm", "agent work"]);
    (repo, wt)
}

/// Count commits on `branch` not reachable from `main` in `repo`.
fn commits_ahead_of_main(repo: &Path, branch: &str) -> usize {
    let out = Command::new("git")
        .current_dir(repo)
        .args(["rev-list", "--count", &format!("main..{branch}")])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
}

/// True when `branch` still exists in `repo`.
fn branch_exists(repo: &Path, branch: &str) -> bool {
    Command::new("git")
        .current_dir(repo)
        .args(["rev-parse", "--verify", "--quiet", branch])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap()
        .success()
}

/// THE data-loss regression, end-to-end (`blocked-report-deletes-branch`): a
/// single-worker autonomous run whose agent commits real work and then submits a
/// BLOCKED terminal `node report` (`success: false`, NO `run merge`) must, after
/// the real supervisor tears the run down, have its branch AND worktree STILL
/// present with the agent's commits intact. Deleting them is the silent data
/// loss this fix prevents.
///
/// Unlike the merge round-trip above (which stubs git to a nonexistent binary),
/// this test uses REAL git so the supervisor's actual teardown executes — the
/// only way to prove the branch survives the real `cleanup_node` path.
#[test]
#[file_serial(key, path => "/tmp/octl-test-supervise.lock")]
fn blocked_report_preserves_branch_and_worktree_e2e() {
    let home = TestHome::new();
    let scratch = TempDir::new().unwrap();
    let branch = "wt/e2e-blocked";
    let (repo, worktree) = init_real_repo_with_committed_work(scratch.path(), branch);
    assert_eq!(commits_ahead_of_main(&repo, branch), 1);

    let agent_pid_file = scratch.path().join("agent.pid");
    let create_sh = write_create_sh(scratch.path(), &worktree, &agent_pid_file, branch);
    // REAL git (GIT_BIN unset → defaults to `git`) so teardown runs for real;
    // only tmux is neutralized (a nonexistent binary → lenient no-op windows and
    // an `Unknown` liveness probe so PID liveness keeps the agent Alive until the
    // blocked report terminalizes the node).
    let no_tmux = scratch.path().join("no-such-tmux");

    let created = run_ok(
        Command::new(env!("CARGO_BIN_EXE_taskfleet"))
            .env("TASKFLEET_HOME", home.path())
            .env("HOME", home.path())
            .env("OCTL_CREATE_SH", &create_sh)
            .env("TMUX_BIN", &no_tmux)
            // Drop any ambient GIT_BIN stub so the supervisor's teardown runs
            // against REAL git — the whole point of this test.
            .env_remove("GIT_BIN")
            .args([
                "--output",
                "json",
                "run",
                "create",
                "--kind",
                "spinoff",
                "--headless",
                "--title",
                "e2e-blocked",
                "--task",
                "investigate",
            ]),
    );
    let run_id = created["data"]["run_id"].as_str().unwrap().to_string();

    let agent_pid: i32 = std::fs::read_to_string(&agent_pid_file)
        .expect("create.sh recorded the agent pid")
        .trim()
        .parse()
        .expect("agent pid is an integer");
    let _agent = AgentGuard { pid: agent_pid };

    let run_root = home.path().join("runs").join(&run_id);
    let events = run_root.join("events.jsonl");
    let manifest = run_root.join("manifest.json");

    assert!(
        wait_for_event(&events, "supervisor.started", Duration::from_secs(15)),
        "supervisor never started; events: {:?}",
        event_kinds(&events)
    );

    // Submit the BLOCKED terminal report (success:false, plain `node report` —
    // NO `run merge`), exactly the documented "needs a human" handoff.
    let report_file = scratch.path().join("blocked.json");
    std::fs::write(
        &report_file,
        r#"{"success": false, "summary": "needs the user's sudo",
            "discussion_items": [{"topic": "blocked", "detail": "need a human"}]}"#,
    )
    .unwrap();
    run_ok(
        Command::new(env!("CARGO_BIN_EXE_taskfleet"))
            .env("TASKFLEET_HOME", home.path())
            .env("HOME", home.path())
            .args([
                "--output",
                "json",
                "node",
                "report",
                &run_id,
                "n-0001",
                "--from-file",
                report_file.to_str().unwrap(),
            ]),
    );

    // The supervisor rolls the run up (Failed — a node reported failure) and
    // winds down, but the blocked path must NOT touch the branch/worktree.
    assert!(
        wait_for_manifest_status(&manifest, "failed", Duration::from_secs(30)),
        "run never rolled up to failed; events: {:?}",
        event_kinds(&events)
    );
    assert!(
        wait_for_event(&events, "supervisor.exited", Duration::from_secs(30)),
        "supervisor never exited; events: {:?}",
        event_kinds(&events)
    );

    // THE assertions: the branch survives with the agent's commit, and the
    // worktree is preserved for the human — no silent data loss.
    assert!(
        branch_exists(&repo, branch),
        "blocked terminal report must leave the branch for the human"
    );
    assert_eq!(
        commits_ahead_of_main(&repo, branch),
        1,
        "the agent's committed work must survive on the preserved branch"
    );
    assert!(
        worktree.exists(),
        "blocked path should preserve the worktree too"
    );

    // The preservation is auditable: a `cleanup.branch_preserved` event names it.
    let preserved = read_events(&events)
        .into_iter()
        .any(|v| v["kind"] == "cleanup.branch_preserved" && v["data"]["branch"] == branch);
    assert!(
        preserved,
        "expected a cleanup.branch_preserved audit event; events: {:?}",
        event_kinds(&events)
    );
}

/// Companion no-regression: the SUCCESS/merge path (`run merge`, stamping
/// `via: "explicit-merge"`) still force-deletes the branch and removes the
/// worktree under real git — the confirmed merge earns the force `-D`, so even
/// though the stub merge.sh does not actually merge the commit into `main`, the
/// branch is torn down exactly as before this fix.
#[test]
#[file_serial(key, path => "/tmp/octl-test-supervise.lock")]
fn merge_path_deletes_branch_e2e() {
    let home = TestHome::new();
    let scratch = TempDir::new().unwrap();
    let branch = "wt/e2e-merge";
    let (repo, worktree) = init_real_repo_with_committed_work(scratch.path(), branch);

    let agent_pid_file = scratch.path().join("agent.pid");
    let create_sh = write_create_sh(scratch.path(), &worktree, &agent_pid_file, branch);
    let merge_sh = write_merge_sh(scratch.path());
    let no_tmux = scratch.path().join("no-such-tmux");

    let created = run_ok(
        Command::new(env!("CARGO_BIN_EXE_taskfleet"))
            .env("TASKFLEET_HOME", home.path())
            .env("HOME", home.path())
            .env("OCTL_CREATE_SH", &create_sh)
            .env("TMUX_BIN", &no_tmux)
            // Drop any ambient GIT_BIN stub so the supervisor's teardown runs
            // against REAL git — the whole point of this test.
            .env_remove("GIT_BIN")
            .args([
                "--output",
                "json",
                "run",
                "create",
                "--kind",
                "spinoff",
                "--headless",
                "--title",
                "e2e-merge",
                "--task",
                "echo done",
            ]),
    );
    let run_id = created["data"]["run_id"].as_str().unwrap().to_string();

    let agent_pid: i32 = std::fs::read_to_string(&agent_pid_file)
        .expect("create.sh recorded the agent pid")
        .trim()
        .parse()
        .expect("agent pid is an integer");
    let _agent = AgentGuard { pid: agent_pid };

    let run_root = home.path().join("runs").join(&run_id);
    let events = run_root.join("events.jsonl");
    let manifest = run_root.join("manifest.json");

    assert!(
        wait_for_event(&events, "supervisor.started", Duration::from_secs(15)),
        "supervisor never started; events: {:?}",
        event_kinds(&events)
    );

    // Merge to close: stamps `via: "explicit-merge"` — the confirmed-merge signal.
    let merged = run_ok(
        Command::new(env!("CARGO_BIN_EXE_taskfleet"))
            .env("TASKFLEET_HOME", home.path())
            .env("HOME", home.path())
            .env("OCTL_MERGE_SH", &merge_sh)
            .args(["--output", "json", "run", "merge", &run_id]),
    );
    assert_eq!(merged["data"]["merged"], true);

    assert!(
        wait_for_manifest_status(&manifest, "done", Duration::from_secs(30)),
        "run never rolled up to done; events: {:?}",
        event_kinds(&events)
    );
    assert!(
        wait_for_event(&events, "supervisor.exited", Duration::from_secs(30)),
        "supervisor never exited; events: {:?}",
        event_kinds(&events)
    );

    // The merge path force-deletes the branch and removes the worktree — the
    // pre-existing teardown behaviour this fix must not regress.
    assert!(
        !branch_exists(&repo, branch),
        "explicit-merge path must still delete the branch"
    );
    assert!(
        !worktree.exists(),
        "explicit-merge path must still remove the worktree"
    );
    assert!(
        read_events(&events)
            .into_iter()
            .all(|v| v["kind"] != "cleanup.branch_preserved"),
        "the merge path must not preserve (must delete) the branch"
    );
}

/// THE `reducer-adopt-explicit-merge` end-to-end proof (staging the
/// watchdog-terminal-then-explicit-merge sequence of
/// `agent-died-merge-no-teardown-interactive`): a node is terminalized FAILED by
/// a (here forged) watchdog `agent-died` report BEFORE the merge, so the first
/// supervisor rolls the run up and exits WITHOUT tearing the branch/worktree down
/// (they are PRESERVED — a `success: false` handoff). Then `run merge` appends its
/// `via: "explicit-merge"` report. Pre-fix the reducer dropped that as a dead
/// event and the resources leaked forever; now the reducer ADOPTS it, so
/// `any_node_merged_explicitly` sees the merge and `run merge` reattaches a
/// supervisor that — as the SOLE teardown actor (invariant #5) — force-removes the
/// worktree and deletes the branch. Runs against REAL git (`GIT_BIN` removed) so
/// the teardown is genuine, not a stub no-op.
#[test]
#[file_serial(key, path => "/tmp/octl-test-supervise.lock")]
fn swallowed_agent_died_then_merge_reattaches_and_tears_down() {
    let home = TestHome::new();
    let scratch = TempDir::new().unwrap();
    let branch = "wt/e2e-swallowed";
    let (repo, worktree) = init_real_repo_with_committed_work(scratch.path(), branch);

    let agent_pid_file = scratch.path().join("agent.pid");
    let create_sh = write_create_sh(scratch.path(), &worktree, &agent_pid_file, branch);
    let merge_sh = write_merge_sh(scratch.path());
    let no_tmux = scratch.path().join("no-such-tmux");

    // 1. Create a supervised run (real detached supervisor, real git for teardown).
    let created = run_ok(
        Command::new(env!("CARGO_BIN_EXE_taskfleet"))
            .env("TASKFLEET_HOME", home.path())
            .env("HOME", home.path())
            .env("OCTL_CREATE_SH", &create_sh)
            .env("TMUX_BIN", &no_tmux)
            .env_remove("GIT_BIN")
            .args([
                "--output",
                "json",
                "run",
                "create",
                "--kind",
                "spinoff",
                "--headless",
                "--title",
                "e2e-swallowed",
                "--task",
                "echo done",
            ]),
    );
    let run_id = created["data"]["run_id"].as_str().unwrap().to_string();

    let agent_pid: i32 = std::fs::read_to_string(&agent_pid_file)
        .expect("create.sh recorded the agent pid")
        .trim()
        .parse()
        .expect("agent pid is an integer");
    let _agent = AgentGuard { pid: agent_pid };

    let run_root = home.path().join("runs").join(&run_id);
    let events = run_root.join("events.jsonl");
    let pid_file = run_root.join("supervisor.pid");

    assert!(
        wait_for_event(&events, "supervisor.started", Duration::from_secs(15)),
        "supervisor never started; events: {:?}",
        event_kinds(&events)
    );
    let first_pid = read_supervisor_pid(&pid_file).expect("supervisor.pid recorded a pid");

    // 2. Forge a watchdog `agent-died` FALSE POSITIVE terminal report (the agent
    //    `sleep` is still alive). The first supervisor rolls the run up and, since
    //    this is a blocked `success: false` handoff, PRESERVES the branch+worktree
    //    and exits — the exact "terminal, but no teardown" precondition of the bug.
    let report = scratch.path().join("agent-died.json");
    std::fs::write(
        &report,
        r#"{"success": false, "failed": true, "reason": "agent-died", "summary": "watchdog false positive"}"#,
    )
    .unwrap();
    run_ok(
        Command::new(env!("CARGO_BIN_EXE_taskfleet"))
            .env("TASKFLEET_HOME", home.path())
            .env("HOME", home.path())
            .env("TMUX_BIN", &no_tmux)
            .env_remove("GIT_BIN")
            .args([
                "--output",
                "json",
                "node",
                "report",
                &run_id,
                "n-0001",
                "--from-file",
                report.to_str().unwrap(),
            ]),
    );

    // The first supervisor sees the terminal node, preserves (does not tear down),
    // and exits.
    assert!(
        wait_for_event(&events, "supervisor.exited", Duration::from_secs(30)),
        "first supervisor never exited after the agent-died terminal; events: {:?}",
        event_kinds(&events)
    );
    assert!(
        wait_for_process_gone(first_pid, Duration::from_secs(10)),
        "first supervisor pid {first_pid} still alive"
    );
    // Precondition proven: the branch + worktree survived the (blocked) terminal.
    assert!(
        branch_exists(&repo, branch),
        "the blocked terminal must PRESERVE the branch (not tear it down)"
    );
    assert!(
        worktree.exists(),
        "the blocked terminal must preserve the worktree"
    );
    assert!(
        read_events(&events)
            .into_iter()
            .any(|v| v["kind"] == "cleanup.branch_preserved"),
        "expected a cleanup.branch_preserved on the pre-merge terminal; events: {:?}",
        event_kinds(&events)
    );

    // 3. The still-alive agent runs `run merge`. The reducer ADOPTS the late
    //    explicit-merge report against the terminal node, so `run merge`
    //    reattaches a supervisor to own teardown (single-owner, invariant #5).
    let merged = run_ok(
        Command::new(env!("CARGO_BIN_EXE_taskfleet"))
            .env("TASKFLEET_HOME", home.path())
            .env("HOME", home.path())
            .env("OCTL_MERGE_SH", &merge_sh)
            .env("TMUX_BIN", &no_tmux)
            .env_remove("GIT_BIN")
            .args(["--output", "json", "run", "merge", &run_id]),
    );
    assert_eq!(merged["data"]["merged"], true);
    assert_eq!(
        merged["data"]["supervisor"]["state"], "reattached",
        "the swallowed path must reattach a supervisor to tear down, got: {}",
        merged["data"]["supervisor"]
    );

    // 4. The reattached supervisor — the SOLE teardown actor — force-removes the
    //    worktree and deletes the branch that was preserved a moment ago.
    let deadline = Instant::now() + Duration::from_secs(30);
    while (branch_exists(&repo, branch) || worktree.exists()) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        !branch_exists(&repo, branch),
        "the reattached supervisor must delete the adopted-merge branch; events: {:?}",
        event_kinds(&events)
    );
    assert!(
        !worktree.exists(),
        "the reattached supervisor must remove the worktree; events: {:?}",
        event_kinds(&events)
    );
    assert!(
        event_kinds(&events)
            .iter()
            .any(|k| k == "supervisor.reattached"),
        "expected a supervisor.reattached event; got {:?}",
        event_kinds(&events)
    );
}

/// Regression for `supervisor-dead-merge-no-teardown`: if the per-run
/// supervisor has died (here: SIGKILL, leaving a stale `supervisor.pid`), a
/// subsequent `run merge` must NOT report a bare, silent `merged: true`. It
/// auto-reattaches — restarting the supervisor to consume the terminal report
/// and complete teardown — AND surfaces a warning so the caller is never
/// misled into telling the user cleanup happened when it was momentarily
/// broken. We assert BOTH: the warning is present (no silent success) and the
/// recovery actually lands the run in a terminal `done` state.
#[test]
#[file_serial(key, path => "/tmp/octl-test-supervise.lock")]
fn merge_reattaches_and_warns_when_supervisor_dead() {
    let home = TestHome::new();
    let scratch = TempDir::new().unwrap();
    let worktree = scratch.path().join("worktree");
    std::fs::create_dir_all(&worktree).unwrap();
    let agent_pid_file = scratch.path().join("agent.pid");
    let branch = "wt/e2e-dead-supervisor";

    let create_sh = write_create_sh(scratch.path(), &worktree, &agent_pid_file, branch);
    let merge_sh = write_merge_sh(scratch.path());
    let no_tmux = scratch.path().join("no-such-tmux");
    let no_git = scratch.path().join("no-such-git");

    // 1. Create the spinoff with a real detached supervisor.
    let created = run_ok(
        Command::new(env!("CARGO_BIN_EXE_taskfleet"))
            .env("TASKFLEET_HOME", home.path())
            .env("HOME", home.path())
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
                "e2e-dead",
                "--task",
                "echo done",
            ]),
    );
    let run_id = created["data"]["run_id"].as_str().unwrap().to_string();

    let agent_pid: i32 = std::fs::read_to_string(&agent_pid_file)
        .expect("create.sh recorded the agent pid")
        .trim()
        .parse()
        .expect("agent pid is an integer");
    let _agent = AgentGuard { pid: agent_pid };

    let run_root = home.path().join("runs").join(&run_id);
    let events = run_root.join("events.jsonl");
    let pid_file = run_root.join("supervisor.pid");
    let manifest = run_root.join("manifest.json");

    // 2. Wait for the supervisor to announce itself, then read + KILL it. SIGKILL
    //    cannot be caught, so it leaves the stale `supervisor.pid` behind — the
    //    exact orphaned condition the bug describes.
    assert!(
        wait_for_event(&events, "supervisor.started", Duration::from_secs(15)),
        "supervisor never started; events: {:?}",
        event_kinds(&events)
    );
    let dead_pid = read_supervisor_pid(&pid_file).expect("supervisor.pid recorded a pid");
    unsafe { libc::kill(dead_pid, libc::SIGKILL) };
    assert!(
        wait_for_process_gone(dead_pid, Duration::from_secs(10)),
        "killed supervisor pid {dead_pid} did not exit"
    );

    // 3. Merge with a dead supervisor. `run merge` inherits the nonexistent
    //    tmux/git binaries so the *reattached* supervisor's teardown is a
    //    lenient no-op, matching the original.
    let merged = run_ok(
        Command::new(env!("CARGO_BIN_EXE_taskfleet"))
            .env("TASKFLEET_HOME", home.path())
            .env("HOME", home.path())
            .env("OCTL_MERGE_SH", &merge_sh)
            .env("TMUX_BIN", &no_tmux)
            .env("GIT_BIN", &no_git)
            .args(["--output", "json", "run", "merge", &run_id]),
    );

    // The merge still lands (no data loss) ...
    assert_eq!(merged["data"]["merged"], true);
    assert_eq!(merged["data"]["branch"], branch);
    // ... but it is NOT silent in the MACHINE channel: the structured
    // `supervisor` outcome records the reattach (an agent reads `state`, not
    // prose). `not-supervised`/`terminal`/`alive` here would mean the recovery
    // path did not fire.
    assert_eq!(
        merged["data"]["supervisor"]["state"], "reattached",
        "merge on a dead supervisor must record a reattached outcome, got: {}",
        merged["data"]["supervisor"]
    );
    // ... and NOT silent in the HUMAN channel: a warning names the restart.
    let warnings = merged["warnings"]
        .as_array()
        .expect("envelope carries a warnings array");
    assert!(
        warnings
            .iter()
            .any(|w| w.as_str().is_some_and(|s| s.contains("supervisor")
                && (s.contains("restarted") || s.contains("run reattach")))),
        "merge on a dead supervisor must warn about the restart/recovery, got: {warnings:?}"
    );

    // 4. Recovery actually happened: the auto-reattached supervisor consumed the
    //    terminal report, rolled the run up to `done`, and tore down. (No silent
    //    orphan left at `pending`.)
    assert!(
        wait_for_manifest_status(&manifest, "done", Duration::from_secs(30)),
        "auto-reattached supervisor never rolled the run up to done; events: {:?}",
        event_kinds(&events)
    );

    // The stale-supervisor recovery is recorded in the event log: the dead prior
    // incarnation is noted and a fresh supervisor is reattached.
    let kinds = event_kinds(&events);
    assert!(
        kinds.iter().any(|k| k == "supervisor.reattached"),
        "expected a supervisor.reattached event after auto-reattach; got {kinds:?}"
    );
    // Prove the run reached `done` via the RECOVERY path (a reattached
    // supervisor consuming the report), not some ambient consumer: the
    // `supervisor.exited{reason:stale-on-reattach}` marker is emitted only by
    // `spawn_supervisor` when it finds the stale pid file we left behind.
    let recovered_via_stale_marker = read_events(&events)
        .into_iter()
        .any(|v| v["kind"] == "supervisor.exited" && v["data"]["reason"] == "stale-on-reattach");
    assert!(
        recovered_via_stale_marker,
        "expected supervisor.exited{{reason:stale-on-reattach}} proving the reattach recovery path ran; got {kinds:?}"
    );
}
