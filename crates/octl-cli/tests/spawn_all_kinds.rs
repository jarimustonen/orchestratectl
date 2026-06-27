//! Integration tests for `run create --kind <X>` materialization path.
//!
//! Uses a fake create.sh fixture so the test never touches tmux,
//! workmux, or the user's git tree. The fake script echoes a canned
//! JSON envelope using the current process PID as `agent_pid_hint` so
//! the supervisor's PID-liveness check passes.
//!
//! Coverage:
//! - All 8 kinds spawn cleanly and produce the expected node + payload.
//! - create.sh exit 2 → orchestratectl exit 2 with envelope code
//!   prefix `create_sh_error_`.
//! - Missing `--task`/`--prompt-file` is a structured user error.
//! - Top-level run writes node.created event and records `agent_pid`.

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use serde_json::Value;
use tempfile::TempDir;

/// Reaps the detached `orchestratectl supervise` processes that
/// `run create` spawns, so the test suite never leaks them
/// (issue: test-harness-leaks-supervisors). These supervisors are
/// grandchildren of the test process (forked by the `run create`
/// subprocess, which has already exited), so they cannot be `waitpid`-ed
/// directly — we SIGTERM each tracked PID and poll `kill(pid, 0)` for it
/// to vanish, escalating to SIGKILL after a short grace period.
///
/// Held as a guard so the reap runs even when an assertion panics, and —
/// declared *after* the run's `TempDir` — drops *before* it, killing the
/// supervisor before its run dir is removed.
struct SupervisorReaper {
    pids: Vec<i32>,
}

impl SupervisorReaper {
    fn new() -> Self {
        Self { pids: Vec::new() }
    }

    fn track(&mut self, pid: i32) {
        if pid > 0 {
            self.pids.push(pid);
        }
    }

    /// Pull the supervisor PID out of a `run create` response envelope's
    /// `data.supervisor` field and track it. Ignored for dry-run /
    /// child-spawn responses where the field is not a PID, and for the
    /// (impossible-in-practice) case of a PID that does not fit `i32`.
    fn track_from_response(&mut self, v: &Value) {
        if let Some(pid) = v["data"]["supervisor"]
            .as_u64()
            .and_then(|p| i32::try_from(p).ok())
        {
            self.track(pid);
        }
    }
}

/// True once `pid` no longer refers to a process *we* can signal.
/// `kill(pid, 0)` succeeding means it is alive and ours. `ESRCH` means
/// gone; `EPERM` means the PID was recycled to a process owned by someone
/// else — our supervisor is gone either way, and we must NOT escalate
/// SIGKILL to a stranger.
fn process_gone(pid: i32) -> bool {
    unsafe { libc::kill(pid, 0) != 0 }
}

impl Drop for SupervisorReaper {
    fn drop(&mut self) {
        for &pid in &self.pids {
            // If it already exited, do nothing — never signal a PID that
            // may have been recycled.
            if process_gone(pid) {
                continue;
            }
            unsafe { libc::kill(pid, libc::SIGTERM) };
            // Wait up to 2s for a clean exit before escalating. We only
            // SIGKILL while `kill(pid, 0)` keeps succeeding (still alive
            // and still ours); the moment it reports gone/recycled we
            // stop, so we never SIGKILL a recycled PID.
            let deadline = Instant::now() + Duration::from_secs(2);
            while !process_gone(pid) {
                if Instant::now() >= deadline {
                    unsafe { libc::kill(pid, libc::SIGKILL) };
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

const KINDS: &[&str] = &[
    "code",
    "spinoff",
    "orchestrated",
    "research",
    "technical-decision",
    "make-skill",
    "fan-out",
    "bugfix",
];

fn write_fake_create_sh(dir: &TempDir, stdout: &str, exit_code: i32) -> PathBuf {
    let path = dir.path().join("fake-create.sh");
    let body = format!("#!/bin/bash\ncat <<'EOF'\n{stdout}\nEOF\nexit {exit_code}\n");
    std::fs::write(&path, body).unwrap();
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).unwrap();
    path
}

fn fake_success_stdout(kind: &str, pid: u32) -> String {
    format!(
        r#"{{"schema_version":1,"type":"{kind}","branch":"wt/test-{kind}","worktree_path":"/tmp/wt-{kind}","tmux_window":"🚀 wt/test-{kind}","agent_pid_hint":{pid},"workmux_session":"test"}}"#
    )
}

fn bin(home: &TempDir, script: &std::path::Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_orchestratectl"));
    c.env("ORCHESTRATECTL_HOME", home.path());
    c.env("OCTL_CREATE_SH", script);
    // Intentionally do NOT set OCTL_TEST_SKIP_MATERIALIZE — these tests
    // exercise the real materialization path against the fake script.
    c
}

fn run_ok(cmd: &mut Command) -> Value {
    let out = cmd.output().expect("spawn");
    assert!(
        out.status.success(),
        "exit={:?} stderr={}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("stdout is JSON")
}

fn run_fail(cmd: &mut Command) -> (i32, Value) {
    let out = cmd.output().expect("spawn");
    assert!(!out.status.success(), "expected failure");
    let code = out.status.code().expect("exit code");
    let stderr = String::from_utf8(out.stderr).expect("utf8");
    let last = stderr.lines().last().expect("stderr line");
    let v: Value = serde_json::from_str(last).expect("error envelope JSON");
    (code, v)
}

#[test]
fn each_kind_spawns_and_emits_node_created() {
    for kind in KINDS {
        let home = TempDir::new().unwrap();
        // Declared after `home` so it drops first: reap the supervisor
        // before the run's TempDir is removed.
        let mut reaper = SupervisorReaper::new();
        let pid = std::process::id();
        let script = write_fake_create_sh(&home, &fake_success_stdout(kind, pid), 0);
        let v = run_ok(bin(&home, &script).args([
            "--output", "json", "run", "create", "--kind", kind, "--title", "smoke", "--task",
            "do work",
        ]));
        reaper.track_from_response(&v);
        let data = &v["data"];
        assert_eq!(data["kind"], *kind, "kind in payload for {kind}: {data}");
        assert_eq!(data["node_id"], "n-0001", "node_id for {kind}");
        assert_eq!(data["branch"], format!("wt/test-{kind}"));
        assert_eq!(data["worktree_path"], format!("/tmp/wt-{kind}"));
        assert!(
            data["supervisor"].as_u64().is_some(),
            "supervisor pid for {kind}: {data}"
        );

        // events.jsonl should contain node.created with agent_pid set.
        let run_id = data["run_id"].as_str().unwrap();
        let events =
            std::fs::read_to_string(home.path().join("runs").join(run_id).join("events.jsonl"))
                .unwrap();
        let saw = events.lines().any(|l| {
            let v: Value = serde_json::from_str(l).unwrap();
            v["kind"] == "node.created" && v["data"]["agent_pid"].as_u64() == Some(u64::from(pid))
        });
        assert!(
            saw,
            "node.created with agent_pid missing for {kind}: {events}"
        );

        // `reaper` (declared above) SIGTERMs the spawned supervisor on
        // drop — before `home`'s TempDir removes the run dir — so the
        // process is reaped deterministically instead of being left to
        // poll a vanished directory.
    }
}

#[test]
fn missing_task_and_prompt_file_is_user_error() {
    let home = TempDir::new().unwrap();
    let script = write_fake_create_sh(&home, "", 0);
    let (code, v) = run_fail(bin(&home, &script).args([
        "--output", "json", "run", "create", "--kind", "spinoff", "--title", "x",
    ]));
    assert_eq!(code, 1);
    assert_eq!(v["error"]["code"], "missing-task-or-prompt-file");
}

#[test]
fn create_sh_exit_2_propagates_as_system_error() {
    let home = TempDir::new().unwrap();
    let path = home.path().join("fake-create.sh");
    let body = "#!/bin/bash\necho '{\"schema_version\":1,\"error\":{\"code\":\"workmux-missing\",\"message\":\"workmux not installed\"}}' >&2\nexit 2\n";
    std::fs::write(&path, body).unwrap();
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).unwrap();
    let (code, v) = run_fail(bin(&home, &path).args([
        "--output", "json", "run", "create", "--kind", "spinoff", "--title", "x", "--task", "do",
    ]));
    assert_eq!(
        code, 2,
        "create.sh exit 2 should map to orchestratectl exit 2"
    );
    assert!(
        v["error"]["code"]
            .as_str()
            .unwrap()
            .starts_with("create_sh_error_"),
        "expected create_sh_error_ prefix: {v}"
    );
}

#[test]
fn task_writes_prompt_file_in_run_dir() {
    let home = TempDir::new().unwrap();
    // Declared after `home` so it drops first, reaping the supervisor
    // before the run dir is removed.
    let mut reaper = SupervisorReaper::new();
    let script = write_fake_create_sh(
        &home,
        &fake_success_stdout("spinoff", std::process::id()),
        0,
    );
    let v = run_ok(bin(&home, &script).args([
        "--output",
        "json",
        "run",
        "create",
        "--kind",
        "spinoff",
        "--title",
        "p",
        "--task",
        "investigate the bug",
    ]));
    reaper.track_from_response(&v);
    let run_id = v["data"]["run_id"].as_str().unwrap();
    let prompt =
        std::fs::read_to_string(home.path().join("runs").join(run_id).join("prompt.md")).unwrap();
    assert_eq!(prompt, "investigate the bug");
}
