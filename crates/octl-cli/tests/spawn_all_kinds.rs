//! Integration tests for `run create --kind <X>` materialization path.
//!
//! Uses a fake create.sh fixture so the test never touches tmux,
//! workmux, or the user's git tree. The fake script echoes a canned
//! JSON envelope using the current process PID as `agent_pid_hint` so
//! the supervisor's PID-liveness check passes.
//!
//! Coverage:
//! - All surviving kinds spawn cleanly and produce the expected node + payload.
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

const KINDS: &[&str] = &["spinoff", "research", "technical-decision", "fan-out"];

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

/// A successfully materialized worker is published while still `pending`; both
/// read surfaces must already carry the repo coordinates at that point.
#[test]
fn pending_materialized_run_surfaces_worktree_and_source_branch() {
    let home = TestHome::new();
    let repo = TempDir::new().unwrap();
    let git_init = Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(repo.path())
        .output()
        .expect("git init");
    assert!(git_init.status.success(), "git init failed");
    for args in [
        &["config", "user.email", "test@example.invalid"][..],
        &["config", "user.name", "Test"][..],
        &["commit", "--allow-empty", "-m", "base"][..],
        &["branch", "wt/test-spinoff", "main"][..],
    ] {
        let out = Command::new("git")
            .args(args)
            .current_dir(repo.path())
            .output()
            .expect("prepare git repo");
        assert!(out.status.success(), "git {args:?} failed");
    }
    let worker = repo.path().join("worker");
    let worktree_add = Command::new("git")
        .args(["worktree", "add"])
        .arg(&worker)
        .arg("wt/test-spinoff")
        .current_dir(repo.path())
        .output()
        .expect("git worktree add");
    assert!(worktree_add.status.success(), "git worktree add failed");
    let fake_output = serde_json::json!({
        "schema_version": 1,
        "type": "spinoff",
        "branch": "wt/test-spinoff",
        "worktree_path": worker,
        "tmux_window": "test",
        "agent_pid_hint": std::process::id(),
        "workmux_session": "test"
    })
    .to_string();
    let script = write_fake_create_sh(&home, &fake_output, 0);
    let created = run_ok(bin(&home, &script).current_dir(repo.path()).args([
        "--output",
        "json",
        "run",
        "create",
        "--kind",
        "spinoff",
        "--title",
        "pending coordinates",
        "--task",
        "do work",
    ]));
    let run_id = created["data"]["run_id"].as_str().unwrap();

    let shown = run_ok(bin(&home, &script).args(["--output", "json", "run", "show", run_id]));
    assert_eq!(shown["data"]["status"], "pending");
    assert_eq!(shown["data"]["worktree_path"], worker.display().to_string());
    assert_eq!(shown["data"]["source_branch"], "main");
    assert_eq!(shown["data"]["manifest"]["source_branch"], "main");

    let listed = run_ok(bin(&home, &script).args(["--output", "json", "run", "list"]));
    let row = &listed["data"]["runs"][0];
    assert_eq!(row["status"], "pending");
    assert_eq!(row["worktree_path"], worker.display().to_string());
    assert_eq!(row["source_branch"], "main");
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
fn long_title_forwards_a_workmux_window_safe_branch_name() {
    // Regression for run-create-long-title-stillborn: workmux flattens the
    // branch into its window name and truncates names over 50 bytes. The old
    // 40-character slug made this title's branch 54 bytes, so create.sh looked
    // up a name different from the one workmux had created.
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
        "DAG head-of-line: in-progress issues are resumable, not excluded (stint-head-of-line-in-progress-eligible)",
        "--task",
        "do work",
        "--headless",
    ]));

    let forwarded: Vec<String> = std::fs::read_to_string(&argv)
        .expect("create.sh recorded its argv")
        .lines()
        .map(str::to_string)
        .collect();
    // The create.sh contract has branch and prompt-file as its two positional
    // arguments. Find the branch by its unambiguous convention rather than
    // depending on where optional flags were emitted.
    let branch = forwarded
        .iter()
        .find(|arg| arg.starts_with("wt/"))
        .expect("branch argument must be passed to create.sh");
    assert!(
        branch.len() <= 50,
        "branch must fit workmux's window-name bound: {branch}"
    );
    assert!(
        !branch.ends_with('-'),
        "branch must not end with a separator after capping: {branch}"
    );
    assert!(
        branch.starts_with("wt/"),
        "branch must retain the worktree convention: {branch}"
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
fn harness_flag_forwards_agent_records_and_surfaces() {
    // End-to-end for `--harness pi`: create.sh receives `--agent pi`, the run's
    // manifest records `harness = "pi"`, and `run show --json` surfaces it at
    // both `data.harness` and `data.manifest.harness`.
    let home = TestHome::new();
    let argv = home.path().join("create-argv.txt");
    let script = write_argv_recording_create_sh(
        &home,
        &argv,
        &fake_success_stdout("spinoff", std::process::id()),
    );
    let v = run_ok(bin(&home, &script).args([
        "--output",
        "json",
        "run",
        "create",
        "--kind",
        "spinoff",
        "--title",
        "h",
        "--task",
        "do work",
        "--harness",
        "pi",
    ]));
    let run_id = v["data"]["run_id"].as_str().unwrap().to_string();

    let recorded = std::fs::read_to_string(&argv).expect("create.sh recorded its argv");
    let forwarded: Vec<&str> = recorded.lines().collect();
    let pos = forwarded
        .iter()
        .position(|a| *a == "--agent")
        .unwrap_or_else(|| panic!("--agent not forwarded; argv={forwarded:?}"));
    assert_eq!(
        forwarded.get(pos + 1).copied(),
        Some("pi"),
        "--agent value should be the selected harness; argv={forwarded:?}"
    );

    let manifest: Value = serde_json::from_str(
        &std::fs::read_to_string(home.path().join("runs").join(&run_id).join("manifest.json"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["harness"], "pi", "manifest records the harness");

    let show = run_ok(bin(&home, &script).args(["--output", "json", "run", "show", &run_id]));
    assert_eq!(show["data"]["harness"], "pi", "run show surfaces harness");
    assert_eq!(
        show["data"]["manifest"]["harness"], "pi",
        "run show manifest surfaces harness"
    );
}

#[test]
fn default_harness_forwards_pi_agent_and_records_pi() {
    // No `--harness`: the built-in pi default is forwarded explicitly to
    // create.sh, and the manifest records the resolved default `pi`.
    let home = TestHome::new();
    let argv = home.path().join("create-argv.txt");
    let script = write_argv_recording_create_sh(
        &home,
        &argv,
        &fake_success_stdout("spinoff", std::process::id()),
    );
    let v = run_ok(bin(&home, &script).args([
        "--output", "json", "run", "create", "--kind", "spinoff", "--title", "d", "--task",
        "do work",
    ]));
    let run_id = v["data"]["run_id"].as_str().unwrap().to_string();

    let recorded = std::fs::read_to_string(&argv).expect("create.sh recorded its argv");
    let forwarded: Vec<&str> = recorded.lines().collect();
    let pos = forwarded
        .iter()
        .position(|a| *a == "--agent")
        .unwrap_or_else(|| panic!("default harness must forward --agent; argv={forwarded:?}"));
    assert_eq!(
        forwarded.get(pos + 1).copied(),
        Some("pi"),
        "default harness must forward the pi agent; argv={forwarded:?}"
    );
    let manifest: Value = serde_json::from_str(
        &std::fs::read_to_string(home.path().join("runs").join(&run_id).join("manifest.json"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["harness"], "pi");
}

#[test]
fn flag_harness_works_despite_broken_config_but_default_path_surfaces_it() {
    // The flag is top precedence and self-sufficient: a malformed config.toml must
    // NOT fail a `run create --harness pi`. Conversely, the no-flag path DOES
    // consult the config, so the same broken file is surfaced as a clear error.
    let home = TestHome::new();
    std::fs::write(
        home.path().join("config.toml"),
        "this is not = valid toml [[[\n",
    )
    .unwrap();
    let argv = home.path().join("create-argv.txt");
    let script = write_argv_recording_create_sh(
        &home,
        &argv,
        &fake_success_stdout("spinoff", std::process::id()),
    );

    // Flag present → config never read → success.
    let v = run_ok(bin(&home, &script).args([
        "--output",
        "json",
        "run",
        "create",
        "--kind",
        "spinoff",
        "--title",
        "bc",
        "--task",
        "do work",
        "--harness",
        "pi",
    ]));
    assert!(v["data"]["run_id"].as_str().is_some());

    // Flag absent → config consulted → the broken file is a clear error (and no
    // worktree/supervisor is spawned, since resolution fails before materialize).
    let (code, err) = run_fail(bin(&home, &script).args([
        "--output", "json", "run", "create", "--kind", "spinoff", "--title", "bc2", "--task",
        "do work",
    ]));
    assert_eq!(code, 1, "malformed config is a user error: {err}");
    assert_eq!(err["error"]["code"], "invalid_config");
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
    assert!(prompt.starts_with("# Orchestratectl run context"));
    assert!(prompt.contains(&format!("run `{run_id}`")));
    assert!(prompt.contains("issuectl intake file"));
    assert!(prompt.contains("originating_run_kind=spinoff"));
    assert!(prompt.trim_end().ends_with("investigate the bug"));
}

/// Spawn a top-level `--kind fan-out` driver run as a skeleton and return its
/// run id. `OCTL_TEST_SKIP_MATERIALIZE` skips create.sh and the supervisor, so
/// it makes a clean parent for child-spawn tests without booting any process.
/// The parent has no `n-0001` node, but a child's `child.spawned` still lands on
/// the parent's log regardless (the reducer just no-ops the child-ref fold when
/// the parent node is absent), which is all these tests inspect.
fn spawn_parent_fanout(home: &TempDir) -> String {
    let mut c = Command::new(env!("CARGO_BIN_EXE_orchestratectl"));
    c.env("ORCHESTRATECTL_HOME", home.path());
    c.env("OCTL_TEST_SKIP_MATERIALIZE", "1");
    let v = run_ok(c.args([
        "--output",
        "json",
        "run",
        "create",
        "--kind",
        "fan-out",
        "--title",
        "driver",
        "--task",
        "drive the fan-out",
    ]));
    v["data"]["run_id"].as_str().unwrap().to_string()
}

/// Count `child.spawned` events in a run's event log.
fn count_child_spawned(home: &TempDir, run_id: &str) -> usize {
    let path = home.path().join("runs").join(run_id).join("events.jsonl");
    let Ok(events) = std::fs::read_to_string(path) else {
        return 0;
    };
    events
        .lines()
        .filter(|l| serde_json::from_str::<Value>(l).is_ok_and(|v| v["kind"] == "child.spawned"))
        .count()
}

#[test]
fn failed_child_spawn_leaves_no_phantom_child() {
    // Regression for `failed-spawn-leaves-phantom-child`: a create.sh failure
    // during a child spawn must be transactional — no `child.spawned` on the
    // parent and no child run dir left behind in `pending`. (Before the fix, the
    // parent log carried a child.spawned and a 0-node phantom child sat in
    // `pending` forever.)
    let home = TestHome::new();
    let parent = spawn_parent_fanout(&home);

    // A create.sh that fails the way the original bug did (exit 2, error
    // envelope on stderr) instead of materializing the child.
    let fail_path = home.path().join("fail-create.sh");
    std::fs::write(
        &fail_path,
        "#!/bin/bash\necho '{\"schema_version\":1,\"error\":{\"code\":\"workmux-add-failed\",\"message\":\"boom\"}}' >&2\nexit 2\n",
    )
    .unwrap();
    let mut perms = std::fs::metadata(&fail_path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&fail_path, perms).unwrap();

    // Snapshot the runs dir so we can prove the failed spawn created no new run.
    let runs_dir = home.path().join("runs");
    let before: std::collections::BTreeSet<_> = std::fs::read_dir(&runs_dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();

    let (code, v) = run_fail(bin(&home, &fail_path).args([
        "--output",
        "json",
        "run",
        "create",
        "--kind",
        "fan-out",
        "--title",
        "doomed child",
        "--task",
        "do work",
        "--parent-run-id",
        &parent,
        "--parent-node-id",
        "n-0001",
    ]));
    assert_eq!(code, 2, "create.sh exit 2 should surface as exit 2: {v}");
    assert!(
        v["error"]["code"]
            .as_str()
            .unwrap()
            .starts_with("create_sh_error_"),
        "expected create_sh_error_ prefix: {v}"
    );

    // (a) No child.spawned landed on the parent.
    assert_eq!(
        count_child_spawned(&home, &parent),
        0,
        "failed spawn must not emit child.spawned on the parent"
    );

    // (b) No new child run dir exists — the orphan was cleaned up.
    let after: std::collections::BTreeSet<_> = std::fs::read_dir(&runs_dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(
        before, after,
        "failed child spawn must leave no new run dir behind"
    );
}

#[test]
fn successful_child_spawn_emits_child_spawned() {
    // Regression guard for the happy path: a successful child spawn still emits
    // exactly one `child.spawned` on the parent, the child run is materialized
    // (node.created + autonomous lifecycle), and the child run dir exists.
    let home = TestHome::new();
    let script = write_fake_create_sh(
        &home,
        &fake_success_stdout("fan-out", std::process::id()),
        0,
    );
    let parent = spawn_parent_fanout(&home);

    let v = run_ok(bin(&home, &script).args([
        "--output",
        "json",
        "run",
        "create",
        "--kind",
        "fan-out",
        "--title",
        "live child",
        "--task",
        "do work",
        "--parent-run-id",
        &parent,
        "--parent-node-id",
        "n-0001",
    ]));
    let child_run_id = v["data"]["run_id"].as_str().unwrap();
    assert_eq!(v["data"]["node_id"], "n-0001");
    assert_eq!(v["data"]["parent_run_id"], parent);
    assert_eq!(v["data"]["lifecycle"], "autonomous");

    // Exactly one child.spawned on the parent, referencing this child.
    let parent_events =
        std::fs::read_to_string(home.path().join("runs").join(&parent).join("events.jsonl"))
            .unwrap();
    let spawned: Vec<Value> = parent_events
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .filter(|v| v["kind"] == "child.spawned")
        .collect();
    assert_eq!(
        spawned.len(),
        1,
        "expected one child.spawned: {parent_events}"
    );
    assert_eq!(spawned[0]["data"]["child_run_id"], child_run_id);
    assert_eq!(spawned[0]["data"]["child_kind"], "fan-out");
    assert_eq!(spawned[0]["node_id"], "n-0001");

    // The child run is materialized: its dir exists and carries node.created.
    let child_events = std::fs::read_to_string(
        home.path()
            .join("runs")
            .join(child_run_id)
            .join("events.jsonl"),
    )
    .unwrap();
    assert!(
        child_events
            .lines()
            .any(|l| serde_json::from_str::<Value>(l).is_ok_and(|v| v["kind"] == "node.created")),
        "child run must have node.created: {child_events}"
    );
}
