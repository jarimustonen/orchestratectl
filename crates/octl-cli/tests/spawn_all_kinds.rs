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

use serde_json::Value;
use tempfile::TempDir;

mod common;
use common::TestHome;

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
        // `home` reaps the supervisor `run create` spawns when it drops,
        // before the run's TempDir is removed.
        let home = TestHome::new();
        let pid = std::process::id();
        let script = write_fake_create_sh(&home, &fake_success_stdout(kind, pid), 0);
        let v = run_ok(bin(&home, &script).args([
            "--output", "json", "run", "create", "--kind", kind, "--title", "smoke", "--task",
            "do work",
        ]));
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

        // `home` (a `TestHome`) SIGTERMs the spawned supervisor on drop —
        // before its TempDir removes the run dir — so the process is reaped
        // deterministically instead of being left to poll a vanished
        // directory.
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

/// A fixture create.sh that records its own argv to `argv_path` (one arg per
/// line) before emitting the canned success envelope. Lets a test assert which
/// flags `run create` forwarded to create.sh.
fn write_argv_recording_create_sh(
    dir: &TempDir,
    argv_path: &std::path::Path,
    stdout: &str,
) -> PathBuf {
    let path = dir.path().join("argv-create.sh");
    let body = format!(
        "#!/bin/bash\nprintf '%s\\n' \"$@\" > '{}'\ncat <<'EOF'\n{stdout}\nEOF\nexit 0\n",
        argv_path.display()
    );
    std::fs::write(&path, body).unwrap();
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).unwrap();
    path
}

#[test]
fn headless_forwards_parent_session_to_create_sh() {
    // `home` reaps the spawned supervisor on drop, before the run dir vanishes.
    let home = TestHome::new();
    let argv = home.path().join("create-argv.txt");
    let script = write_argv_recording_create_sh(
        &home,
        &argv,
        &fake_success_stdout("spinoff", std::process::id()),
    );
    run_ok(bin(&home, &script).args([
        "--output",
        "json",
        "run",
        "create",
        "--kind",
        "spinoff",
        "--title",
        "hl",
        "--task",
        "do work",
        "--headless",
    ]));

    let recorded = std::fs::read_to_string(&argv).expect("create.sh recorded its argv");
    let forwarded: Vec<&str> = recorded.lines().collect();
    // `--headless` with no explicit name resolves to the default `headless`
    // session, forwarded as the `--parent-session <name>` pair.
    let pos = forwarded
        .iter()
        .position(|a| *a == "--parent-session")
        .unwrap_or_else(|| panic!("--parent-session not forwarded; argv={forwarded:?}"));
    assert_eq!(
        forwarded.get(pos + 1).copied(),
        Some("headless"),
        "--parent-session value should be the default headless session; argv={forwarded:?}"
    );
}

#[test]
fn foreground_omits_parent_session_flag() {
    let home = TestHome::new();
    let argv = home.path().join("create-argv.txt");
    let script = write_argv_recording_create_sh(
        &home,
        &argv,
        &fake_success_stdout("spinoff", std::process::id()),
    );
    run_ok(bin(&home, &script).args([
        "--output", "json", "run", "create", "--kind", "spinoff", "--title", "fg", "--task",
        "do work",
    ]));

    let recorded = std::fs::read_to_string(&argv).expect("create.sh recorded its argv");
    assert!(
        !recorded.lines().any(|a| a == "--parent-session"),
        "foreground spawn must not forward --parent-session; argv={recorded:?}"
    );
}

#[test]
fn source_branch_forwards_base_flag_to_create_sh() {
    // The create.rs path must hand `--source-branch <branch>` to create.sh as
    // `--base <branch>` so the worktree forks from the named branch (e.g. an
    // orchestrate integration branch) rather than workmux's default base.
    let home = TestHome::new();
    let argv = home.path().join("create-argv.txt");
    let script = write_argv_recording_create_sh(
        &home,
        &argv,
        &fake_success_stdout("spinoff", std::process::id()),
    );
    run_ok(bin(&home, &script).args([
        "--output",
        "json",
        "run",
        "create",
        "--kind",
        "spinoff",
        "--title",
        "sb",
        "--task",
        "do work",
        "--source-branch",
        "orchestrate/integration",
    ]));

    let recorded = std::fs::read_to_string(&argv).expect("create.sh recorded its argv");
    let forwarded: Vec<&str> = recorded.lines().collect();
    let pos = forwarded
        .iter()
        .position(|a| *a == "--base")
        .unwrap_or_else(|| panic!("--base not forwarded; argv={forwarded:?}"));
    assert_eq!(
        forwarded.get(pos + 1).copied(),
        Some("orchestrate/integration"),
        "--base value should be the source branch; argv={forwarded:?}"
    );
}

#[test]
fn no_source_branch_omits_base_flag() {
    let home = TestHome::new();
    let argv = home.path().join("create-argv.txt");
    let script = write_argv_recording_create_sh(
        &home,
        &argv,
        &fake_success_stdout("spinoff", std::process::id()),
    );
    run_ok(bin(&home, &script).args([
        "--output", "json", "run", "create", "--kind", "spinoff", "--title", "nosb", "--task",
        "do work",
    ]));

    let recorded = std::fs::read_to_string(&argv).expect("create.sh recorded its argv");
    assert!(
        !recorded.lines().any(|a| a == "--base"),
        "run without --source-branch must not forward --base; argv={recorded:?}"
    );
}

#[test]
fn task_writes_prompt_file_in_run_dir() {
    // `home` reaps the supervisor `run create` spawns when it drops, before
    // the run dir is removed.
    let home = TestHome::new();
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
    let run_id = v["data"]["run_id"].as_str().unwrap();
    let prompt =
        std::fs::read_to_string(home.path().join("runs").join(run_id).join("prompt.md")).unwrap();
    assert_eq!(prompt, "investigate the bug");
}
