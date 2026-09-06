//! Creation transaction reliability on the native materializer.

use std::os::unix::fs::PermissionsExt as _;
use std::process::Command;
use std::time::{Duration, Instant};

use tempfile::TempDir;

mod common;
use common::{NativeSpawnTools, TestHome};

fn executable(path: &std::path::Path, body: &str) {
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// Owns the interrupted creator and its deliberately blocking workmux child.
/// Drop runs during assertion panic as well as success, so this failure-path
/// test cannot leave either process behind.
struct InterruptedChildren {
    creator: Option<std::process::Child>,
    owned_pids: Vec<std::path::PathBuf>,
}

impl Drop for InterruptedChildren {
    fn drop(&mut self) {
        if let Some(child) = &mut self.creator {
            let _ = child.kill();
            let _ = child.wait();
        }
        for pid_file in &self.owned_pids {
            if let Ok(pid) = std::fs::read_to_string(pid_file) {
                if let Ok(pid) = pid.trim().parse::<libc::pid_t>() {
                    unsafe { libc::kill(pid, libc::SIGTERM) };
                }
            }
        }
    }
}

fn profile(home: &TestHome, scratch: &TempDir) {
    let worker = scratch.path().join("worker.sh");
    executable(&worker, "#!/bin/sh\nexec /bin/sleep 30\n");
    std::fs::write(
        home.path().join("config.toml"),
        format!(
            r#"[profiles.test]
description="test"
capability="fast"
residency="local"
agents=[{{harness="pi",command=["{}"],telemetry="worker-v1"}}]
[profile]
default="test"
"#,
            worker.display()
        ),
    )
    .unwrap();
}

#[test]
fn missing_candidate_fails_before_publication_and_rolls_back() {
    let home = TestHome::new();
    let tools = NativeSpawnTools::new();
    std::fs::write(
        home.path().join("config.toml"),
        r#"[profiles.test]
description="test"
capability="fast"
residency="local"
agents=[{harness="pi",command=["/definitely/missing/taskfleet-agent"],telemetry="worker-v1"}]
[profile]
default="test"
"#,
    )
    .unwrap();
    let worktree = tools.worktree("worktree");
    let mut command = Command::new(env!("CARGO_BIN_EXE_taskfleet"));
    command
        .env("TASKFLEET_HOME", home.path())
        .env("HOME", home.path());
    tools.configure(&mut command, &worktree, "headless");
    let output = command
        .args([
            "--output",
            "json",
            "run",
            "create",
            "--kind",
            "spinoff",
            "--headless",
            "--title",
            "missing",
            "--task",
            "work",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(std::fs::read_dir(home.path().join("runs"))
        .map_or(true, |mut entries| entries.next().is_none()));
    assert!(!worktree.exists(), "rollback removes fake workmux worktree");
}

#[test]
fn immediate_exit_candidate_fails_before_publication() {
    let home = TestHome::new();
    let tools = NativeSpawnTools::new();
    std::fs::write(
        home.path().join("config.toml"),
        r#"[profiles.test]
description="test"
capability="fast"
residency="local"
agents=[{harness="pi",command=["/usr/bin/true"],telemetry="worker-v1"}]
[profile]
default="test"
"#,
    )
    .unwrap();
    let worktree = tools.worktree("worktree");
    let mut command = Command::new(env!("CARGO_BIN_EXE_taskfleet"));
    command
        .env("TASKFLEET_HOME", home.path())
        .env("HOME", home.path());
    tools.configure(&mut command, &worktree, "headless");
    let output = command
        .args([
            "--output",
            "json",
            "run",
            "create",
            "--kind",
            "spinoff",
            "--headless",
            "--title",
            "immediate-exit",
            "--task",
            "work",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(std::fs::read_dir(home.path().join("runs"))
        .map_or(true, |mut entries| entries.next().is_none()));
    assert!(!worktree.exists());
}

#[test]
fn interrupted_native_materialization_never_publishes_zero_node_run() {
    let home = TestHome::new();
    let scratch = TempDir::new().unwrap();
    let tools = NativeSpawnTools::new();
    profile(&home, &scratch);
    let started = scratch.path().join("started");
    let workmux_pid = scratch.path().join("workmux.pid");
    let sleeper_pid = scratch.path().join("sleeper.pid");
    let workmux = scratch.path().join("blocking-workmux.sh");
    executable(
        &workmux,
        &format!(
            "#!/bin/sh\nif [ \"$1\" = add ]; then echo $$ > '{}'; : > '{}'; /bin/sleep 30 & echo $! > '{}'; wait; exit 1; fi\nexit 1\n",
            workmux_pid.display(),
            started.display(),
            sleeper_pid.display()
        ),
    );
    let worktree = tools.worktree("worktree");
    let mut command = Command::new(env!("CARGO_BIN_EXE_taskfleet"));
    command
        .env("TASKFLEET_HOME", home.path())
        .env("HOME", home.path());
    tools.configure(&mut command, &worktree, "headless");
    command.env("WORKMUX_BIN", &workmux).args([
        "--output",
        "json",
        "run",
        "create",
        "--kind",
        "spinoff",
        "--headless",
        "--title",
        "interrupt",
        "--task",
        "work",
    ]);
    let mut children = InterruptedChildren {
        creator: Some(command.spawn().unwrap()),
        owned_pids: vec![workmux_pid, sleeper_pid],
    };
    // A full no-fail-fast release run starts many process-heavy integration
    // tests concurrently. On macOS the creator can be descheduled for more than
    // 10 seconds before the blocking workmux fixture gets CPU; keep this bounded
    // without confusing scheduler delay with a publication failure.
    let deadline = Instant::now() + Duration::from_secs(30);
    while !started.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(started.exists());
    unsafe {
        libc::kill(
            children.creator.as_ref().unwrap().id() as libc::pid_t,
            libc::SIGKILL,
        );
    }
    let _ = children.creator.as_mut().unwrap().wait();
    children.creator = None;
    assert!(std::fs::read_dir(home.path().join("runs"))
        .map_or(true, |mut entries| entries.next().is_none()));
}

#[test]
fn supervisor_boot_failure_is_loud_after_native_publication() {
    let home = TestHome::new();
    let scratch = TempDir::new().unwrap();
    let tools = NativeSpawnTools::new();
    profile(&home, &scratch);
    let worktree = tools.worktree("worktree");
    let mut command = Command::new(env!("CARGO_BIN_EXE_taskfleet"));
    command
        .env("TASKFLEET_HOME", home.path())
        .env("HOME", home.path());
    tools.configure(&mut command, &worktree, "headless");
    let output = command
        .env("TASKFLEET_SUPERVISE_BIN", "/bin/false")
        .args([
            "--output",
            "json",
            "run",
            "create",
            "--kind",
            "spinoff",
            "--headless",
            "--title",
            "bad supervisor",
            "--task",
            "work",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("spawn_failed"),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let run = std::fs::read_dir(home.path().join("runs"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let node: serde_json::Value =
        serde_json::from_slice(&std::fs::read(run.join("nodes/n-0001.json")).unwrap()).unwrap();
    assert!(node["agent_pid"].as_u64().is_some());
}
