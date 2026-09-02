//! ADR 0002 R2: dual-name input/config and bounded legacy-home adoption.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::TempDir;

const BINARY: &str = env!("CARGO_BIN_EXE_taskfleet");

fn clean_command() -> Command {
    let home = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/test-homes/dual-name-resolver")
        .join(std::process::id().to_string());
    std::fs::create_dir_all(&home).unwrap();
    let mut command = Command::new(BINARY);
    command.env("HOME", home);
    for name in [
        "TASKFLEET_HOME",
        "ORCHESTRATECTL_HOME",
        "TASKFLEET_PROFILE",
        "ORCHESTRATECTL_PROFILE",
        "TASKFLEET_HARNESS",
        "ORCHESTRATECTL_HARNESS",
        "TASKFLEET_LOG",
        "ORCHESTRATECTL_LOG",
        "OCTL_INTERNAL_SELF_EXEC",
    ] {
        command.env_remove(name);
    }
    command
}

fn config_path(command: &mut Command) -> Output {
    command
        .args(["--output", "text", "config", "path"])
        .output()
        .expect("run config path")
}

fn stdout_path(output: &Output) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(&output.stdout).trim())
}

fn normalized_config(root: &Path) -> PathBuf {
    root.join("config.toml")
}

fn warning_lines(output: &Output) -> usize {
    String::from_utf8_lossy(&output.stderr)
        .lines()
        .filter(|line| line.starts_with("warning:"))
        .count()
}

#[test]
fn explicit_home_matrix_accepts_new_old_and_normalized_equal_but_rejects_conflict() {
    let sandbox = TempDir::new().unwrap();
    let canonical = sandbox.path().join("state");

    let new = config_path(clean_command().env("TASKFLEET_HOME", &canonical));
    assert!(new.status.success());
    assert_eq!(stdout_path(&new), normalized_config(&canonical));
    assert_eq!(warning_lines(&new), 0);

    let old = config_path(clean_command().env("ORCHESTRATECTL_HOME", &canonical));
    assert!(old.status.success());
    assert_eq!(stdout_path(&old), normalized_config(&canonical));
    assert_eq!(warning_lines(&old), 1);

    std::fs::create_dir_all(&canonical).unwrap();
    let equal = config_path(
        clean_command()
            .current_dir(sandbox.path())
            .env("TASKFLEET_HOME", "state")
            .env(
                "ORCHESTRATECTL_HOME",
                sandbox.path().join("./state/../state"),
            ),
    );
    assert!(
        equal.status.success(),
        "{}",
        String::from_utf8_lossy(&equal.stderr)
    );
    assert_eq!(warning_lines(&equal), 1);

    let left = sandbox.path().join("left");
    let right = sandbox.path().join("right");
    let conflict = config_path(
        clean_command()
            .env("TASKFLEET_HOME", &left)
            .env("ORCHESTRATECTL_HOME", &right),
    );
    assert!(!conflict.status.success());
    assert!(String::from_utf8_lossy(&conflict.stderr).contains("conflicting_home"));
    assert!(
        !left.exists(),
        "conflict must precede logging/filesystem writes"
    );
    assert!(
        !right.exists(),
        "conflict must precede logging/filesystem writes"
    );
}

#[test]
fn default_home_matrix_adopts_only_populated_legacy_and_refuses_dual_truth() {
    let sandbox = TempDir::new().unwrap();
    let user = sandbox.path().join("user");
    std::fs::create_dir(&user).unwrap();

    let fresh = config_path(clean_command().env("HOME", &user));
    assert!(fresh.status.success());
    assert_eq!(
        stdout_path(&fresh),
        normalized_config(&user.join(".taskfleet"))
    );
    assert_eq!(warning_lines(&fresh), 0);

    // Start from a clean HOME because the preceding invocation populated the
    // canonical root with its isolated log.
    let adopted_user = sandbox.path().join("adopted-user");
    let legacy = adopted_user.join(".orchestratectl");
    std::fs::create_dir_all(legacy.join("runs")).unwrap();
    std::fs::write(legacy.join("runs/evidence"), b"legacy").unwrap();
    let adopted = config_path(clean_command().env("HOME", &adopted_user));
    assert!(adopted.status.success());
    assert_eq!(stdout_path(&adopted), normalized_config(&legacy));
    assert_eq!(warning_lines(&adopted), 1);
    assert!(!adopted_user.join(".taskfleet").exists());

    let split_user = sandbox.path().join("split-user");
    for root in [".taskfleet", ".orchestratectl"] {
        let root = split_user.join(root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("managed"), root.as_os_str().as_encoded_bytes()).unwrap();
    }
    let split = config_path(clean_command().env("HOME", &split_user));
    assert!(!split.status.success());
    assert!(String::from_utf8_lossy(&split.stderr).contains("conflicting_state_homes"));
    assert!(!split_user.join(".taskfleet/logs").exists());
    assert!(!split_user.join(".orchestratectl/logs").exists());
}

#[test]
fn empty_legacy_directory_is_not_adopted() {
    let sandbox = TempDir::new().unwrap();
    let user = sandbox.path().join("user");
    std::fs::create_dir_all(user.join(".orchestratectl")).unwrap();
    let output = config_path(clean_command().env("HOME", &user));
    assert!(output.status.success());
    assert_eq!(stdout_path(&output), user.join(".taskfleet/config.toml"));
}

#[test]
fn nonexistent_suffix_equivalence_is_lexical_and_case_sensitive() {
    let sandbox = TempDir::new().unwrap();
    let output = config_path(
        clean_command()
            .env("TASKFLEET_HOME", sandbox.path().join("State"))
            .env("ORCHESTRATECTL_HOME", sandbox.path().join("state")),
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("conflicting_home"));
}

#[test]
fn explicit_home_intentionally_overrides_populated_defaults() {
    let sandbox = TempDir::new().unwrap();
    let user = sandbox.path().join("user");
    let legacy = user.join(".orchestratectl");
    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::write(legacy.join("state"), b"old truth").unwrap();
    let explicit = sandbox.path().join("isolated");

    let output = config_path(
        clean_command()
            .env("HOME", &user)
            .env("TASKFLEET_HOME", &explicit),
    );
    assert!(output.status.success());
    assert_eq!(stdout_path(&output), normalized_config(&explicit));
    assert_eq!(std::fs::read(legacy.join("state")).unwrap(), b"old truth");
}

#[cfg(unix)]
#[test]
fn existing_symlink_paths_compare_by_physical_identity() {
    use std::os::unix::fs::symlink;
    let sandbox = TempDir::new().unwrap();
    let target = sandbox.path().join("target");
    std::fs::create_dir(&target).unwrap();
    let alias = sandbox.path().join("alias");
    symlink(&target, &alias).unwrap();

    let output = config_path(
        clean_command()
            .env("TASKFLEET_HOME", &target)
            .env("ORCHESTRATECTL_HOME", &alias),
    );
    assert!(output.status.success());
    assert_eq!(stdout_path(&output), normalized_config(&target));
    assert_eq!(warning_lines(&output), 1);
}

#[cfg(unix)]
#[test]
fn symlink_parent_traversal_uses_kernel_semantics() {
    use std::os::unix::fs::symlink;
    let sandbox = TempDir::new().unwrap();
    let lexical_parent = sandbox.path().join("lexical");
    let physical_parent = sandbox.path().join("physical");
    std::fs::create_dir(&lexical_parent).unwrap();
    std::fs::create_dir_all(physical_parent.join("child")).unwrap();
    symlink(physical_parent.join("child"), lexical_parent.join("link")).unwrap();
    let spelling = lexical_parent.join("link/..");

    let output = config_path(
        clean_command()
            .env("TASKFLEET_HOME", &spelling)
            .env("ORCHESTRATECTL_HOME", &physical_parent),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(physical_parent
        .join("logs/orchestratectl.log.jsonl")
        .exists());
    assert!(!lexical_parent.join("logs").exists());
}

#[test]
fn explicit_non_directory_fails_before_logging() {
    let sandbox = TempDir::new().unwrap();
    let file = sandbox.path().join("not-a-home");
    std::fs::write(&file, b"data").unwrap();
    let output = config_path(clean_command().env("TASKFLEET_HOME", &file));
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid_home"));
    assert_eq!(std::fs::read(&file).unwrap(), b"data");
}

#[cfg(unix)]
#[test]
fn dangling_symlink_home_fails_before_logging() {
    use std::os::unix::fs::symlink;
    let sandbox = TempDir::new().unwrap();
    let link = sandbox.path().join("dangling");
    symlink(sandbox.path().join("missing"), &link).unwrap();
    let output = config_path(clean_command().env("TASKFLEET_HOME", &link));
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("home_unreadable"));
}

#[test]
fn all_legacy_alias_warnings_are_one_stderr_line_and_never_stdout_jsonl() {
    let sandbox = TempDir::new().unwrap();
    let output = clean_command()
        .env("ORCHESTRATECTL_HOME", sandbox.path().join("state"))
        .env("ORCHESTRATECTL_PROFILE", "missing")
        .env("ORCHESTRATECTL_HARNESS", "pi")
        .env("ORCHESTRATECTL_LOG", "warn")
        .args(["version", "--output", "jsonl"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(warning_lines(&output), 1);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout.lines().count(), 1);
    serde_json::from_str::<serde_json::Value>(stdout.trim()).expect("stdout stays JSONL");
}

#[test]
fn hostile_warning_path_stays_one_physical_line() {
    let sandbox = TempDir::new().unwrap();
    let output = clean_command()
        .env(
            "ORCHESTRATECTL_HOME",
            sandbox.path().join("line\nbreak\u{1b}"),
        )
        .arg("version")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(warning_lines(&output), 1);
    assert_eq!(String::from_utf8_lossy(&output.stderr).lines().count(), 1);
}

#[test]
fn hidden_self_exec_suppresses_legacy_warning() {
    let sandbox = TempDir::new().unwrap();
    let output = clean_command()
        .env("ORCHESTRATECTL_HOME", sandbox.path().join("state"))
        .env("OCTL_INTERNAL_SELF_EXEC", "1")
        .arg("version")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(warning_lines(&output), 0);
}

#[test]
fn normalized_selector_aliases_are_equal() {
    let sandbox = TempDir::new().unwrap();
    let output = clean_command()
        .env("TASKFLEET_HOME", sandbox.path().join("state"))
        .env("TASKFLEET_HARNESS", " pi ")
        .env("ORCHESTRATECTL_HARNESS", "pi")
        .arg("version")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(warning_lines(&output), 1);
}

#[test]
fn differing_selector_aliases_fail_before_log_creation() {
    let sandbox = TempDir::new().unwrap();
    let root = sandbox.path().join("state");
    let output = clean_command()
        .env("TASKFLEET_HOME", &root)
        .env("TASKFLEET_HARNESS", "pi")
        .env("ORCHESTRATECTL_HARNESS", "claude")
        .arg("version")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("conflicting_environment"));
    assert!(!root.exists());
}

#[test]
fn semantic_output_conflict_is_filesystem_pure() {
    let sandbox = TempDir::new().unwrap();
    let root = sandbox.path().join("state");
    let output = clean_command()
        .env("TASKFLEET_HOME", &root)
        .args(["--json", "--output", "json", "version"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!root.exists());
}

#[test]
fn split_roots_refuse_metadata_commands_before_logging() {
    let sandbox = TempDir::new().unwrap();
    for name in [".taskfleet", ".orchestratectl"] {
        let root = sandbox.path().join(name);
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("managed"), b"state").unwrap();
    }
    let output = clean_command()
        .env("HOME", sandbox.path())
        .arg("version")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("conflicting_state_homes"));
}

#[test]
fn structured_and_text_help_are_filesystem_pure_even_with_conflicting_inputs() {
    let sandbox = TempDir::new().unwrap();
    let left = sandbox.path().join("left");
    let right = sandbox.path().join("right");
    for args in [vec!["--help"], vec!["--help", "--output", "json"]] {
        let output = clean_command()
            .env("TASKFLEET_HOME", &left)
            .env("ORCHESTRATECTL_HOME", &right)
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success());
    }
    assert!(!left.exists());
    assert!(!right.exists());
}

#[test]
fn repository_config_old_equal_and_conflict_semantics_precede_writes() {
    let sandbox = TempDir::new().unwrap();
    let repo = sandbox.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    std::fs::create_dir(repo.join(".git")).unwrap();
    let state = sandbox.path().join("state");
    let base = || {
        let mut command = clean_command();
        command.env("TASKFLEET_HOME", &state).args([
            "run",
            "create",
            "--kind",
            "spinoff",
            "--title",
            "resolver-test",
            "--source-repo",
            repo.to_str().unwrap(),
            "--task",
            "test",
            "--harness",
            "pi",
            "--dry-run",
        ]);
        command
    };

    std::fs::write(repo.join(".orchestratectl.toml"), b"").unwrap();
    let old = base().output().unwrap();
    assert!(
        old.status.success(),
        "{}",
        String::from_utf8_lossy(&old.stderr)
    );
    assert_eq!(warning_lines(&old), 1);

    std::fs::write(repo.join(".taskfleet.toml"), b"").unwrap();
    let equal = base().output().unwrap();
    assert!(equal.status.success());
    assert_eq!(warning_lines(&equal), 1);

    std::fs::write(repo.join(".taskfleet.toml"), b"[profile]\ndefault='a'\n").unwrap();
    let before = state.join("logs/orchestratectl.log.jsonl");
    let previous_len = std::fs::metadata(&before).map_or(0, |m| m.len());
    let conflict = base().output().unwrap();
    assert!(!conflict.status.success());
    assert!(String::from_utf8_lossy(&conflict.stderr).contains("conflicting_repository_config"));
    assert_eq!(
        std::fs::metadata(&before).map_or(0, |m| m.len()),
        previous_len,
        "repository conflict must be detected before logging"
    );
}

#[test]
fn frozen_051_state_is_read_from_a_default_adopted_legacy_root_without_byte_changes() {
    let sandbox = TempDir::new().unwrap();
    let user = sandbox.path().join("user");
    let legacy = user.join(".orchestratectl");
    copy_tree(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/orchestratectl-0.5.1/home/orchestratectl")
            .as_path(),
        &legacy,
    );
    let before = file_snapshot(&legacy);
    let output = clean_command()
        .env("HOME", &user)
        .args(["run", "list", "--output", "json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["data"]["runs"].as_array().unwrap().len(), 4);
    assert_eq!(file_snapshot(&legacy), before);
    assert!(
        !user.join(".taskfleet").exists(),
        "adoption never moves state"
    );
}

fn file_snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, current: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(current).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                visit(root, &path, out);
            } else if !path.strip_prefix(root).unwrap().starts_with("logs") {
                out.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    std::fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(root, root, &mut out);
    out
}

fn copy_tree(source: &Path, destination: &Path) {
    std::fs::create_dir_all(destination).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}
