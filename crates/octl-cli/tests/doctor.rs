//! Integration tests for the `doctor` subcommand (AGENTS-AI-FIRST-CLI
//! §18).
//!
//! Locks the contract an AI caller depends on: the §18 structured shape
//! under `--output json`, the streaming jsonl form, per-check `id`s and
//! statuses, exit-code semantics (any `fail` → 1; warnings never flip
//! it), the `skill.sync` drift WARN, the `schema.runs` corruption FAIL,
//! and the `--fix` / `--fix --dry-run` behaviour.
//!
//! Every test sandboxes `HOME`, `ORCHESTRATECTL_HOME`, and `PATH` into
//! tempdirs so it never touches the developer's real install and the
//! dependency checks are deterministic regardless of what is on the CI
//! machine's PATH.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use tempfile::TempDir;

/// Build a fake `PATH` dir containing stub executables for every binary
/// `doctor`'s `dep.*` checks look for, so those checks pass deterministically.
fn fake_path_dir() -> TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    for bin in ["tmux", "git", "workmux", "issuectl"] {
        let p = dir.path().join(bin);
        std::fs::write(&p, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    dir
}

struct Env {
    home: TempDir,
    orch: PathBuf,
    path: TempDir,
}

fn setup() -> Env {
    let home = tempfile::tempdir().expect("tempdir");
    let orch = home.path().join("orch");
    // Create the orchestratectl home so `config.home` reports OK.
    std::fs::create_dir_all(&orch).unwrap();
    Env {
        home,
        orch,
        path: fake_path_dir(),
    }
}

fn bin(env: &Env) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_orchestratectl"));
    cmd.env("HOME", env.home.path());
    cmd.env("ORCHESTRATECTL_HOME", &env.orch);
    cmd.env("PATH", env.path.path());
    cmd
}

/// Install a hand-written skill SKILL.md at the default claude path with
/// the given `cli_version`, so `skill.sync` sees it on disk.
fn install_skill(env: &Env, name: &str, cli_version: &str) {
    let dir = env.home.path().join(".claude/skills").join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("SKILL.md"),
        format!(
            "---\nname: {name}\ndescription: test\ncli_version: \"{cli_version}\"\nschema_version: 1\n---\nbody\n"
        ),
    )
    .unwrap();
}

fn find_check<'a>(checks: &'a [Value], id: &str) -> &'a Value {
    checks
        .iter()
        .find(|c| c["id"] == id)
        .unwrap_or_else(|| panic!("no check with id {id}; got {checks:?}"))
}

#[test]
fn healthy_install_passes_with_exit_zero() {
    let env = setup();
    let out = bin(&env)
        .args(["--output", "text", "doctor"])
        .output()
        .expect("spawn");
    // No FAILs (deps stubbed, home writable, no broken runs) → exit 0,
    // even though skill.sync warns about not-installed skills.
    assert!(out.status.success(), "expected exit 0; got {:?}", out);
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("config.home"));
    assert!(stdout.contains("dep.tmux"));
    assert!(stdout.contains("summary:"));
}

#[test]
fn json_emits_section18_bundled_shape() {
    let env = setup();
    let out = bin(&env)
        .args(["--output", "json", "doctor"])
        .output()
        .expect("spawn");
    let v: Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(v["schema_version"], 1);
    let checks = v["data"]["checks"].as_array().expect("checks array");
    assert!(!checks.is_empty());
    // Each check has the §18 fields.
    for c in checks {
        assert!(c["id"].is_string());
        assert!(c["status"].is_string());
        assert!(c["message"].is_string());
    }
    let summary = &v["data"]["summary"];
    assert!(summary["ok"].is_number());
    assert!(summary["warn"].is_number());
    assert!(summary["fail"].is_number());
    // config.home is OK in a freshly-created home.
    assert_eq!(find_check(checks, "config.home")["status"], "ok");
    assert_eq!(find_check(checks, "dep.git")["status"], "ok");
}

#[test]
fn jsonl_streams_one_check_per_line_then_summary_event() {
    let env = setup();
    let out = bin(&env).arg("doctor").output().expect("spawn");
    let stdout = String::from_utf8(out.stdout).unwrap();
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
    assert!(lines.len() >= 2);
    // Every line is self-describing: it carries schema_version and an
    // event discriminator so a streaming consumer can version-check and
    // route each record independently.
    let last: Value = serde_json::from_str(lines.last().unwrap()).expect("summary json");
    assert_eq!(last["event"], "summary");
    assert_eq!(last["schema_version"], 1);
    assert!(last["ok"].is_number());
    let first: Value = serde_json::from_str(lines[0]).expect("check json");
    assert_eq!(first["event"], "check");
    assert_eq!(first["schema_version"], 1);
    assert!(first["id"].is_string());
}

#[test]
fn broken_manifest_fails_schema_check_and_exits_one() {
    let env = setup();
    let run_dir = env.orch.join("runs").join("01jxbad0000000000000000000");
    std::fs::create_dir_all(&run_dir).unwrap();
    std::fs::write(run_dir.join("manifest.json"), "{ this is not json").unwrap();

    let out = bin(&env)
        .args(["--output", "json", "doctor"])
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(1), "any fail → exit 1");
    let v: Value = serde_json::from_slice(&out.stdout).expect("json");
    let checks = v["data"]["checks"].as_array().unwrap();
    let c = find_check(checks, "schema.runs.01jxbad0000000000000000000");
    assert_eq!(c["status"], "fail");
    assert!(c["fix_suggestion"]
        .as_str()
        .unwrap()
        .contains("run cancel 01jxbad0000000000000000000"));
    assert_eq!(v["data"]["summary"]["fail"], 1);
}

#[test]
fn skill_drift_warns_with_install_suggestion() {
    let env = setup();
    install_skill(&env, "octl-run-overview", "0.0.0");

    let out = bin(&env)
        .args(["--output", "json", "doctor"])
        .output()
        .expect("spawn");
    let v: Value = serde_json::from_slice(&out.stdout).expect("json");
    let checks = v["data"]["checks"].as_array().unwrap();
    let c = find_check(checks, "skill.sync.octl-run-overview");
    assert_eq!(c["status"], "warn");
    assert!(c["message"].as_str().unwrap().contains("0.0.0"));
    assert!(c["fix_suggestion"]
        .as_str()
        .unwrap()
        .contains("skill install octl-run-overview --force"));
    // A drift WARN never flips the exit code.
    assert!(out.status.success(), "warnings must not fail the run");
}

#[test]
fn fix_reinstalls_drifted_skill() {
    let env = setup();
    install_skill(&env, "octl-run-overview", "0.0.0");
    let on_disk = env
        .home
        .path()
        .join(".claude/skills/octl-run-overview/SKILL.md");

    let out = bin(&env)
        .args(["--output", "json", "doctor", "--fix"])
        .output()
        .expect("spawn");
    let v: Value = serde_json::from_slice(&out.stdout).expect("json");
    let fixes = v["data"]["fixes_applied"]
        .as_array()
        .expect("fixes_applied");
    let f = fixes
        .iter()
        .find(|f| f["check_id"] == "skill.sync.octl-run-overview")
        .expect("install fix applied");
    assert_eq!(f["applied"], true);

    // The on-disk skill now matches the running binary's version.
    let after = std::fs::read_to_string(&on_disk).unwrap();
    assert!(
        after.contains(&format!("cli_version: \"{}\"", env!("CARGO_PKG_VERSION"))),
        "skill was not re-installed: {after}"
    );
}

#[test]
fn fix_dry_run_emits_plan_and_changes_nothing() {
    let env = setup();
    install_skill(&env, "octl-run-overview", "0.0.0");
    let on_disk = env
        .home
        .path()
        .join(".claude/skills/octl-run-overview/SKILL.md");

    let out = bin(&env)
        .args(["--output", "json", "doctor", "--fix", "--dry-run"])
        .output()
        .expect("spawn");
    assert!(out.status.success(), "dry-run exits 0");
    let v: Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(v["dry_run"], true);
    let would = v["would"].as_array().expect("would array");
    assert!(would
        .iter()
        .any(|w| w["target"] == "octl-run-overview" && w["action"] == "install"));

    // Nothing was applied — the drifted file is untouched.
    let after = std::fs::read_to_string(&on_disk).unwrap();
    assert!(after.contains("0.0.0"), "dry-run must not mutate: {after}");
}

#[test]
fn dry_run_without_fix_is_rejected() {
    let env = setup();
    let out = bin(&env)
        .args(["doctor", "--dry-run"])
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(1));
    let err: Value = serde_json::from_slice(&out.stderr).expect("err envelope");
    assert_eq!(err["error"]["code"], "invalid_arguments");
}

#[test]
fn dead_supervisor_pid_warns() {
    let env = setup();
    let run_dir = env.orch.join("runs").join("01jxdead000000000000000000");
    std::fs::create_dir_all(&run_dir).unwrap();
    // PID 2^31-1 is effectively never live.
    write_minimal_manifest(&run_dir, "01jxdead000000000000000000");
    std::fs::write(run_dir.join("supervisor.pid"), "2147483647").unwrap();

    let out = bin(&env)
        .args(["--output", "json", "doctor"])
        .output()
        .expect("spawn");
    let v: Value = serde_json::from_slice(&out.stdout).expect("json");
    let checks = v["data"]["checks"].as_array().unwrap();
    let c = find_check(checks, "data.orphan-supervisor.01jxdead000000000000000000");
    assert_eq!(c["status"], "warn");
    assert!(c["fix_suggestion"]
        .as_str()
        .unwrap()
        .contains("run reattach 01jxdead000000000000000000"));
}

#[test]
fn corrupt_supervisor_pid_warns() {
    let env = setup();
    let run_dir = env.orch.join("runs").join("01jxcrpt000000000000000000");
    std::fs::create_dir_all(&run_dir).unwrap();
    write_minimal_manifest(&run_dir, "01jxcrpt000000000000000000");
    std::fs::write(run_dir.join("supervisor.pid"), "not-a-pid").unwrap();

    let out = bin(&env)
        .args(["--output", "json", "doctor"])
        .output()
        .expect("spawn");
    let v: Value = serde_json::from_slice(&out.stdout).expect("json");
    let checks = v["data"]["checks"].as_array().unwrap();
    let c = find_check(checks, "data.orphan-supervisor.01jxcrpt000000000000000000");
    assert_eq!(c["status"], "warn");
    assert!(c["message"]
        .as_str()
        .unwrap()
        .contains("unreadable/unparseable"));
}

#[test]
fn fix_removes_stale_dead_supervisor_pid() {
    let env = setup();
    let run_dir = env.orch.join("runs").join("01jxstp0000000000000000000");
    std::fs::create_dir_all(&run_dir).unwrap();
    write_minimal_manifest(&run_dir, "01jxstp0000000000000000000");
    let pid_path = run_dir.join("supervisor.pid");
    std::fs::write(&pid_path, "2147483647").unwrap();
    // Backdate the mtime well past the 24h staleness threshold so the
    // safe fix is offered. `touch -t` is portable across macOS/Linux.
    let ok = Command::new("touch")
        .args(["-t", "202001010000"])
        .arg(&pid_path)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    assert!(ok, "touch -t failed; cannot backdate pid file");

    let out = bin(&env)
        .args(["--output", "json", "doctor", "--fix"])
        .output()
        .expect("spawn");
    let v: Value = serde_json::from_slice(&out.stdout).expect("json");
    let fixes = v["data"]["fixes_applied"]
        .as_array()
        .expect("fixes_applied");
    let f = fixes
        .iter()
        .find(|f| f["check_id"] == "data.orphan-supervisor.01jxstp0000000000000000000")
        .expect("stale pid fix applied");
    assert_eq!(f["applied"], true, "fix outcome: {f:?}");
    assert!(!pid_path.exists(), "stale pid file should be removed");
}

/// Write a schema-valid manifest so the orphan-supervisor test exercises
/// only the data-integrity path (and schema.runs stays OK for that run).
fn write_minimal_manifest(run_dir: &Path, run_id: &str) {
    let manifest = serde_json::json!({
        "schema_version": 1,
        "run_id": run_id,
        "kind": "spinoff",
        "lifecycle": "autonomous",
        "title": "t",
        "status": "running",
        "created_at": "2026-06-27T00:00:00Z",
        "updated_at": "2026-06-27T00:00:00Z",
        "source_repo": null,
        "source_branch": null,
        "worktree_root": null,
        "node_count": 0,
        "open_discussions": 0,
        "pending_spinoffs": 0,
        "parent_run_id": null,
        "parent_node_id": null
    });
    std::fs::write(
        run_dir.join("manifest.json"),
        serde_json::to_string(&manifest).unwrap(),
    )
    .unwrap();
}
