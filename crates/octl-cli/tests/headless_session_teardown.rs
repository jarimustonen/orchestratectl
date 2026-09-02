//! Behaviour test for the `--headless` tmux SESSION lifecycle (issue
//! `headless-tmux-session-not-torn-down`).
//!
//! When orchestratectl bootstraps a `--headless` / `--tmux-session` run, tmux
//! opens a synthetic default shell window (`zsh`) in the new session and the
//! agent windows are added alongside it. Closing every agent window therefore
//! did NOT remove the session — the bootstrap window kept it alive, leaving an
//! empty `headless` session behind after the last spinoff self-merged.
//!
//! This drives the production spawn → supervise → merge → teardown loop and
//! asserts the parent session is gone once the last managed window is torn down.
//!
//! Unlike `e2e_spinoff.rs` (which points `TMUX_BIN` at a nonexistent path so
//! every tmux step is a lenient no-op), this test needs a REAL tmux so the
//! supervisor's `kill-window` / `list-windows` / `kill-session` actually act on
//! a server we can then inspect with `tmux list-sessions`. To stay safe and
//! CI-friendly it runs entirely on a PRIVATE tmux socket (`-S <tmp>/tmux.sock`)
//! — never the user's default server — and SKIPS (does not fail) when tmux is
//! not on `PATH`. `GIT_BIN` is still pointed at a nonexistent path so the
//! worktree/branch teardown stays an inert no-op; only the tmux SESSION
//! lifecycle is under test here.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde_json::Value;
use serial_test::file_serial;
use tempfile::TempDir;

mod common;
use common::TestHome;

/// A long-lived agent process the stub create.sh spawned, killed on drop so a
/// panicking assertion never leaks the `sleep` (the `TestHome` reaper only
/// knows about supervisor pids).
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

/// Kills the entire private tmux server on drop, so no test session/pane (and
/// the `sleep` commands inside its windows) outlives the test even on a panic.
struct TmuxServerGuard {
    tmux: PathBuf,
    socket: PathBuf,
}

impl Drop for TmuxServerGuard {
    fn drop(&mut self) {
        let _ = Command::new(&self.tmux)
            .args(["-S", self.socket.to_str().unwrap(), "kill-server"])
            .output();
    }
}

fn write_exec(path: &Path, body: &str) {
    std::fs::write(path, body).unwrap();
    let mut perms = std::fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).unwrap();
}

/// Resolve a real `tmux` on `PATH`, or `None` so the test can skip.
fn which_tmux() -> Option<PathBuf> {
    let out = Command::new("sh")
        .args(["-c", "command -v tmux"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!p.is_empty()).then(|| PathBuf::from(p))
}

/// A stub `create.sh` that mirrors the real session bootstrap on a PRIVATE tmux
/// socket: it idempotently creates the headless session with a synthetic `zsh`
/// window (exactly the leftover the issue is about), spawns a long-lived agent
/// for PID liveness, opens the agent's own tmux window, and emits the structured
/// envelope carrying the real qualified tmux identity (session + window id +
/// socket) the supervisor tears down.
fn write_create_sh(
    scratch: &Path,
    tmux: &Path,
    socket: &Path,
    worktree: &Path,
    agent_pid_file: &Path,
    session: &str,
    branch: &str,
) -> PathBuf {
    let p = scratch.join("fake-create.sh");
    let body = format!(
        r#"#!/bin/bash
set -e
TMUX={tmux}
SOCK={socket}
# Bootstrap the headless session exactly as the real create.sh does: a detached
# session whose lone window is the synthetic default shell (named `zsh` here so
# the assertion is deterministic across CI shells). Idempotent — a second spawn
# into the same session is a no-op (`|| true`), mirroring create.sh.
"$TMUX" -S "$SOCK" new-session -d -s {session} -n zsh "sleep 600" 2>/dev/null || true
# A long-lived agent process so PID liveness keeps the node Alive until the
# merge terminalizes it (the agent does NOT hold create.sh's stdout pipe open).
bash -c 'echo done; exec sleep 120' </dev/null >/dev/null 2>&1 &
agent_pid=$!
echo "$agent_pid" > '{pidfile}'
# The agent's own tmux window, alongside the bootstrap shell. Capture its id.
wid=$("$TMUX" -S "$SOCK" new-window -t {session} -n "agent-{branch}" -P -F '#{{window_id}}' "sleep 600")
cat <<EOF
{{"schema_version":1,"type":"spinoff","branch":"{branch}","worktree_path":"{worktree}","tmux_window":"agent-{branch}","agent_pid_hint":$agent_pid,"workmux_session":"{session}","tmux_socket":"{socket}","tmux_session":"{session}","tmux_window_id":"$wid"}}
EOF
"#,
        tmux = tmux.display(),
        socket = socket.display(),
        session = session,
        branch = branch,
        pidfile = agent_pid_file.display(),
        worktree = worktree.display(),
    );
    write_exec(&p, &body);
    p
}

/// A stub `merge.sh` that exits 0 so `run merge` submits the terminal report.
fn write_merge_sh(dir: &Path) -> PathBuf {
    let p = dir.join("fake-merge.sh");
    write_exec(&p, "#!/bin/bash\nexit 0\n");
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

/// `tmux -S <socket> list-sessions` names (empty when the server is gone).
fn list_sessions(tmux: &Path, socket: &Path) -> Vec<String> {
    let out = Command::new(tmux)
        .args([
            "-S",
            socket.to_str().unwrap(),
            "list-sessions",
            "-F",
            "#{session_name}",
        ])
        .output()
        .expect("spawn tmux list-sessions");
    // A torn-down server exits non-zero ("no server running") → no sessions.
    if !out.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

/// The full round-trip: a `--headless` spinoff self-merges, and the supervisor
/// tears down BOTH the agent window AND the now-empty parent session — so
/// `tmux list-sessions` no longer shows the `headless` session afterwards.
#[test]
#[file_serial(key, path => "/tmp/octl-test-supervise.lock")]
fn headless_session_is_torn_down_after_last_managed_run() {
    let Some(tmux) = which_tmux() else {
        eprintln!(
            "skipping headless_session_is_torn_down_after_last_managed_run: tmux not on PATH"
        );
        return;
    };

    let home = TestHome::new();
    let scratch = TempDir::new().unwrap();
    let socket = scratch.path().join("tmux.sock");
    let worktree = scratch.path().join("worktree");
    std::fs::create_dir_all(&worktree).unwrap();
    let agent_pid_file = scratch.path().join("agent.pid");
    let session = "headless";
    let branch = "wt/headless-teardown";

    // Tear the private server down no matter how the test exits.
    let _server = TmuxServerGuard {
        tmux: tmux.clone(),
        socket: socket.clone(),
    };

    let create_sh = write_create_sh(
        scratch.path(),
        &tmux,
        &socket,
        &worktree,
        &agent_pid_file,
        session,
        branch,
    );
    let merge_sh = write_merge_sh(scratch.path());
    // Real tmux (so the supervisor acts on our private server) but a nonexistent
    // git, so worktree/branch teardown is a lenient no-op — only the tmux SESSION
    // lifecycle is under test.
    let no_git = scratch.path().join("no-such-git");

    let created = run_ok(
        Command::new(env!("CARGO_BIN_EXE_orchestratectl"))
            .env("TASKFLEET_HOME", home.path())
            .env("OCTL_CREATE_SH", &create_sh)
            .env("TMUX_BIN", &tmux)
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
                "headless-teardown",
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

    let events = home.path().join("runs").join(&run_id).join("events.jsonl");

    assert!(
        wait_for_event(&events, "supervisor.started", Duration::from_secs(15)),
        "supervisor never started; events: {:?}",
        event_kinds(&events)
    );

    // Sanity: before merge the session exists with BOTH the synthetic shell and
    // the agent window — i.e. the exact pre-teardown shape from the report.
    assert!(
        list_sessions(&tmux, &socket).iter().any(|s| s == session),
        "the headless session should exist while the run is live"
    );

    // Merge to close: the stub merge.sh exits 0, so the terminal node.report is
    // submitted and the supervisor rolls the run up to `done`.
    let merged = run_ok(
        Command::new(env!("CARGO_BIN_EXE_orchestratectl"))
            .env("TASKFLEET_HOME", home.path())
            .env("OCTL_MERGE_SH", &merge_sh)
            .env("TMUX_BIN", &tmux)
            .env("GIT_BIN", &no_git)
            .args(["--output", "json", "run", "merge", &run_id]),
    );
    assert_eq!(merged["data"]["merged"], true);

    assert!(
        wait_for_event(&events, "supervisor.exited", Duration::from_secs(30)),
        "supervisor never exited; events: {:?}",
        event_kinds(&events)
    );

    // THE ASSERTION: the empty headless session must be gone once the last
    // managed window was torn down. Poll briefly — the supervisor kills the
    // window then the session in the same cleanup tick, just after it exits.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut sessions = list_sessions(&tmux, &socket);
    while sessions.iter().any(|s| s == session) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
        sessions = list_sessions(&tmux, &socket);
    }
    assert!(
        !sessions.iter().any(|s| s == session),
        "the empty `{session}` session must be torn down after the last managed run; \
         tmux list-sessions still shows: {sessions:?}"
    );

    // The teardown recorded a `cleanup.session_killed` audit event.
    let killed = read_events(&events)
        .into_iter()
        .any(|v| v["kind"] == "cleanup.session_killed" && v["data"]["session"] == session);
    assert!(
        killed,
        "expected a cleanup.session_killed audit event; events: {:?}",
        event_kinds(&events)
    );
}
