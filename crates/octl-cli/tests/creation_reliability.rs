//! Integration tests for the creation-path reliability guards (issue
//! `supervisor-spawn-fails-silently-at-run-create`).
//!
//! Two properties that must hold WITHOUT reproducing the flaky load trigger:
//!
//! 1. **Fail loudly, not silently.** When the detached supervisor never
//!    confirms boot (its readiness pipe closes without a ready signal because
//!    it died during init), `run create` must return a `supervisor_spawn_failed`
//!    error envelope carrying the run id — never hang, never misreport `pid: 0`
//!    as success — and a `supervisor.stderr.log` with the failure reason must
//!    exist on disk.
//!
//! 2. **No stuck `pending`.** A supervised run that never got a worker node
//!    (and has no children) must be terminalized `failed`, not left `pending`
//!    forever or falsely reported `work-complete`.
//!
//! The supervisor spawn is forced to fail deterministically via the
//! `OCTL_SUPERVISE_BIN` seam (point it at `/usr/bin/false`, which execs and
//! exits at once without signalling readiness), so the parent's readiness read
//! returns EOF — a real death, detected immediately with no timeout. Mirrors the
//! existing `OCTL_CREATE_SH` binary-override hook. No real tmux/workmux/git is
//! touched.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde_json::Value;
use serial_test::file_serial;
use tempfile::TempDir;

mod common;
use common::TestHome;

/// A live `sleep` the stub `create.sh` spawned as the agent; killed on drop so
/// a panicking assertion never leaks it (the `TestHome` reaper only knows
/// supervisor pids).
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

/// A stub create.sh that spawns a long-lived agent, records its pid, and emits
/// the structured `SpawnOutcome` production parses on exit 0 — so the create
/// path reaches `node.created` + the supervisor spawn (the step we force to
/// fail).
fn write_create_sh(
    scratch: &Path,
    worktree: &Path,
    agent_pid_file: &Path,
    branch: &str,
) -> PathBuf {
    let p = scratch.join("fake-create.sh");
    let body = format!(
        r#"#!/bin/bash
bash -c 'exec sleep 120' </dev/null >/dev/null 2>&1 &
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

fn wait_for(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !path.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(path.exists(), "timed out waiting for {}", path.display());
}

fn read_events(events: &Path) -> Vec<Value> {
    std::fs::read_to_string(events)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .collect()
}

/// An external client timeout can kill `run create` while its create.sh child is
/// still blocked. The unfinished create must remain private in `.creating`, not
/// publish the old `run.created`-only stillborn shape under `runs/`.
#[test]
#[file_serial(key, path => "/tmp/octl-test-supervise.lock")]
fn interrupted_create_never_publishes_a_zero_node_run() {
    let home = TestHome::new();
    let scratch = TempDir::new().unwrap();
    let ready = scratch.path().join("create-started");
    let script_pid = scratch.path().join("create.pid");
    let create_sh = scratch.path().join("blocking-create.sh");
    write_exec(
        &create_sh,
        &format!(
            "#!/bin/bash\necho $$ > '{}'\ntouch '{}'\nwhile :; do sleep 1; done\n",
            script_pid.display(),
            ready.display()
        ),
    );

    let mut child = Command::new(env!("CARGO_BIN_EXE_orchestratectl"))
        .env("ORCHESTRATECTL_HOME", home.path())
        .env("OCTL_CREATE_SH", &create_sh)
        .args([
            "--output",
            "json",
            "run",
            "create",
            "--kind",
            "spinoff",
            "--headless",
            "--title",
            "interrupted",
            "--idempotency-key",
            "interrupted-key",
            "--task",
            "echo done",
        ])
        .spawn()
        .expect("spawn blocking run create");
    wait_for(&ready);

    // This models the caller-side timeout in the field report. Kill the shell
    // too: `Command::output` does not create a new process group, so a direct
    // SIGKILL of its Rust parent would otherwise intentionally leave the test
    // fixture's blocking child alive.
    child.kill().expect("kill run create");
    child.wait().expect("reap run create");
    // A same-key retry must fail loudly instead of replaying the private run as
    // a success. It must not remove the live creator's reservation: concurrent
    // callers need the same protection.
    let retry = Command::new(env!("CARGO_BIN_EXE_orchestratectl"))
        .env("ORCHESTRATECTL_HOME", home.path())
        .env("OCTL_CREATE_SH", &create_sh)
        .env("OCTL_IDEMPOTENCY_PUBLISH_WAIT_MS", "0")
        .args([
            "--output",
            "json",
            "run",
            "create",
            "--kind",
            "spinoff",
            "--headless",
            "--title",
            "interrupted",
            "--idempotency-key",
            "interrupted-key",
            "--task",
            "echo done",
        ])
        .output()
        .expect("retry interrupted run create");
    assert!(
        !retry.status.success(),
        "private reservation must not replay"
    );
    let retry_error: Value = serde_json::from_slice(&retry.stderr).unwrap();
    assert_eq!(retry_error["error"]["code"], "idempotency_publish_timeout");

    if let Ok(pid) = std::fs::read_to_string(&script_pid)
        .map(|s| s.trim().parse::<i32>().expect("numeric script pid"))
    {
        unsafe { libc::kill(pid, libc::SIGKILL) };
    }

    let public_runs = std::fs::read_dir(home.path().join("runs"))
        .expect("public runs directory")
        .count();
    assert_eq!(public_runs, 0, "an interrupted create must publish no run");
    assert!(
        home.path()
            .join(".creating")
            .join("runs")
            .read_dir()
            .unwrap()
            .next()
            .is_some(),
        "the unfinished state stays in the private staging root for diagnosis"
    );
}

/// `run create` returns a loud `supervisor_spawn_failed` envelope (with the run
/// id) and always leaves a `supervisor.stderr.log` trace, instead of the
/// original silent hang with a `pending` orphan and zero trace.
#[test]
#[file_serial(key, path => "/tmp/octl-test-supervise.lock")]
fn run_create_fails_loud_when_supervisor_never_confirms() {
    let home = TestHome::new();
    let scratch = TempDir::new().unwrap();
    let worktree = scratch.path().join("worktree");
    std::fs::create_dir_all(&worktree).unwrap();
    let agent_pid_file = scratch.path().join("agent.pid");
    let branch = "wt/creation-fail-loud";
    let create_sh = write_create_sh(scratch.path(), &worktree, &agent_pid_file, branch);
    let no_git = scratch.path().join("no-such-git");

    let out = Command::new(env!("CARGO_BIN_EXE_orchestratectl"))
        .env("ORCHESTRATECTL_HOME", home.path())
        .env("OCTL_CREATE_SH", &create_sh)
        // Force the supervisor spawn to "succeed" at fork/exec but never confirm
        // boot: /usr/bin/false execs then exits 1 at once, so its readiness-pipe
        // write end closes with no ready signal → the parent reads EOF (a real
        // death), not a slow boot.
        .env("OCTL_SUPERVISE_BIN", "/usr/bin/false")
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
            "fail-loud",
            "--task",
            "echo done",
        ])
        .output()
        .expect("spawn run create");

    // Reap the stub agent regardless of assertion outcome.
    let _agent = std::fs::read_to_string(&agent_pid_file)
        .ok()
        .and_then(|s| s.trim().parse::<i32>().ok())
        .map(|pid| AgentGuard { pid });

    // 1. Loud failure: non-zero exit + a `supervisor_spawn_failed` envelope on
    //    stderr carrying the run id (in `invalid_value`), not a hang.
    assert!(
        !out.status.success(),
        "run create must fail loudly, not report a bogus success"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    let env: Value = serde_json::from_str(stderr.trim().lines().last().unwrap_or(""))
        .unwrap_or_else(|_| panic!("stderr is not a JSON error envelope: {stderr}"));
    assert_eq!(env["error"]["code"], "supervisor_spawn_failed");
    let run_id = env["error"]["invalid_value"]
        .as_str()
        .expect("error envelope carries the run id in invalid_value")
        .to_string();

    // 2. The run is recoverable on disk: run dir present, still `pending`, and
    //    its worker node was created (node_count 1) — only the supervisor
    //    failed to boot.
    let run_dir = home.path().join("runs").join(&run_id);
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(run_dir.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["status"], "pending");
    assert_eq!(manifest["node_count"], 1);

    // 3. A trace exists: `supervisor.stderr.log` was written with the reason
    //    (the original bug wrote nothing at all).
    let log = std::fs::read_to_string(run_dir.join("supervisor.stderr.log")).unwrap_or_default();
    assert!(
        log.contains("readiness pipe closed") || log.contains("died during init"),
        "supervisor.stderr.log must record the spawn failure reason, got: {log:?}"
    );

    // Sanity: node.created is durable so `run reattach` can recover.
    let kinds: Vec<String> = read_events(&run_dir.join("events.jsonl"))
        .into_iter()
        .filter_map(|v| v["kind"].as_str().map(str::to_string))
        .collect();
    assert!(
        kinds.contains(&"node.created".to_string()),
        "kinds: {kinds:?}"
    );
}
