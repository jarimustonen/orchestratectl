use std::path::{Path, PathBuf};
use std::process::Command;

use assert_cmd::prelude::*;
use fs4::FileExt;
use predicates::prelude::*;
use tempfile::TempDir;

fn bin(home: &Path) -> Command {
    let mut command = Command::cargo_bin("taskfleet").unwrap();
    command
        .env("HOME", home)
        .env_remove("TASKFLEET_HOME")
        .env_remove("ORCHESTRATECTL_HOME")
        .env_remove("OCTL_TEST_MIGRATION_CRASH_AT");
    command
}

fn roots(home: &Path) -> (PathBuf, PathBuf) {
    let home = home.canonicalize().unwrap();
    (home.join("legacy"), home.join("canonical"))
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn terminal_source(home: &Path) -> (PathBuf, PathBuf) {
    let (source, destination) = roots(home);
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/orchestratectl-0.5.1/home/orchestratectl");
    std::fs::create_dir_all(source.join("runs")).unwrap();
    copy_tree(
        &fixture.join("runs/01j00000000000000000000001"),
        &source.join("runs/01j00000000000000000000001"),
    );
    std::fs::copy(fixture.join("config.toml"), source.join("config.toml")).unwrap();
    (source, destination)
}

fn tree_bytes(root: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut std::collections::BTreeMap<PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                walk(root, &entry.path(), out);
            } else {
                out.insert(
                    entry.path().strip_prefix(root).unwrap().to_path_buf(),
                    std::fs::read(entry.path()).unwrap(),
                );
            }
        }
    }
    let mut out = std::collections::BTreeMap::new();
    walk(root, root, &mut out);
    out
}

fn set_receipt_state(home: &Path, state: &str) {
    let receipt = std::fs::read_dir(home.join(".taskfleet-migrations"))
        .unwrap()
        .map(Result::unwrap)
        .map(|entry| entry.path())
        .find(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
        .unwrap();
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&receipt).unwrap()).unwrap();
    value["state"] = serde_json::Value::String(state.to_string());
    std::fs::write(receipt, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
}

fn migrate(command: &mut Command, source: &Path, destination: &Path) {
    command.args([
        "state",
        "migrate",
        "--source",
        source.to_str().unwrap(),
        "--destination",
        destination.to_str().unwrap(),
        "--json",
    ]);
}

#[test]
fn dry_run_is_read_only_and_reports_operator_exclusion() {
    let home = TempDir::new().unwrap();
    let (source, destination) = terminal_source(home.path());
    let before =
        std::fs::read(source.join("runs/01j00000000000000000000001/events.jsonl")).unwrap();
    let mut command = bin(home.path());
    migrate(&mut command, &source, &destination);
    command.arg("--dry-run");
    command
        .assert()
        .success()
        .stdout(predicate::str::contains("\"dry_run\": true"))
        .stdout(predicate::str::contains("Operator-enforced exclusion"));
    assert!(source.exists());
    assert!(!destination.exists());
    assert!(!home.path().join(".taskfleet-migrations").exists());
    assert_eq!(
        std::fs::read(source.join("runs/01j00000000000000000000001/events.jsonl")).unwrap(),
        before
    );
}

#[test]
fn migration_preserves_bytes_and_verified_state_can_roll_back() {
    let home = TempDir::new().unwrap();
    let (source, destination) = terminal_source(home.path());
    let before = tree_bytes(&source);
    let mut command = bin(home.path());
    migrate(&mut command, &source, &destination);
    command
        .assert()
        .success()
        .stdout(predicate::str::contains("\"state\": \"verified\""));
    assert!(!source.exists());
    assert_eq!(tree_bytes(&destination), before);

    bin(home.path())
        .args([
            "state",
            "rollback",
            "--source",
            source.to_str().unwrap(),
            "--destination",
            destination.to_str().unwrap(),
            "--json",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"state\": \"rolled_back\""));
    assert!(source.exists());
    assert!(!destination.exists());
    assert_eq!(tree_bytes(&source), before);
}

#[test]
fn rollback_crash_after_rename_recovers_to_rolled_back() {
    let home = TempDir::new().unwrap();
    let (source, destination) = terminal_source(home.path());
    let mut command = bin(home.path());
    migrate(&mut command, &source, &destination);
    command.assert().success();
    let rollback_args = [
        "state",
        "rollback",
        "--source",
        source.to_str().unwrap(),
        "--destination",
        destination.to_str().unwrap(),
        "--json",
    ];
    // Materialize the durable/post-rename crash state without a shipped
    // fault-injection hook, then verify the release binary recovers it.
    set_receipt_state(home.path(), "rollback_prepared");
    std::fs::rename(&destination, &source).unwrap();
    assert!(source.exists());
    assert!(!destination.exists());
    bin(home.path())
        .args(rollback_args)
        .assert()
        .success()
        .stdout(predicate::str::contains("\"state\": \"rolled_back\""));
}

#[test]
fn existing_destination_refuses_without_replacement() {
    let home = TempDir::new().unwrap();
    let (source, destination) = terminal_source(home.path());
    std::fs::create_dir(&destination).unwrap();
    let mut command = bin(home.path());
    migrate(&mut command, &source, &destination);
    command.assert().failure().stderr(predicate::str::contains(
        "refusing to choose, merge, or overwrite",
    ));
    assert!(source.exists());
    assert!(destination.is_dir());
}

#[test]
fn text_mode_renders_and_admin_output_is_refused() {
    let home = TempDir::new().unwrap();
    let (source, destination) = terminal_source(home.path());
    bin(home.path())
        .args([
            "state",
            "migrate",
            "--source",
            source.to_str().unwrap(),
            "--destination",
            destination.to_str().unwrap(),
            "--dry-run",
            "--output",
            "text",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("operation: migrate"));
    let admin_output = home
        .path()
        .canonicalize()
        .unwrap()
        .join(".taskfleet-migrations/out.json");
    bin(home.path())
        .args([
            "state",
            "migrate",
            "--source",
            source.to_str().unwrap(),
            "--destination",
            destination.to_str().unwrap(),
            "--dry-run",
            "--output",
            admin_output.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("migration_output_inside_admin"));
}

#[cfg(unix)]
#[test]
fn canonical_alias_still_closes_first_write_marker() {
    use std::os::unix::fs::symlink;
    let home = TempDir::new().unwrap();
    let (source, destination) = terminal_source(home.path());
    let mut command = bin(home.path());
    migrate(&mut command, &source, &destination);
    command.assert().success();
    let alias = home.path().canonicalize().unwrap().join("canonical-alias");
    symlink(&destination, &alias).unwrap();
    bin(home.path())
        .env("TASKFLEET_HOME", &alias)
        .args(["version", "--json"])
        .assert()
        .success();
    bin(home.path())
        .args([
            "state",
            "rollback",
            "--source",
            source.to_str().unwrap(),
            "--destination",
            destination.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("permanently forbidden"));
}

#[cfg(unix)]
#[test]
fn symlinked_admin_directory_is_refused() {
    use std::os::unix::fs::symlink;
    let home = TempDir::new().unwrap();
    let (source, destination) = terminal_source(home.path());
    symlink(&source, home.path().join(".taskfleet-migrations")).unwrap();
    let mut command = bin(home.path());
    migrate(&mut command, &source, &destination);
    command
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid_migration_admin"));
}

#[test]
fn first_canonical_log_attempt_permanently_closes_rollback() {
    let home = TempDir::new().unwrap();
    let (source, destination) = terminal_source(home.path());
    let mut command = bin(home.path());
    migrate(&mut command, &source, &destination);
    command.assert().success();

    bin(home.path())
        .env("TASKFLEET_HOME", &destination)
        .args(["version", "--json"])
        .assert()
        .success();

    bin(home.path())
        .args([
            "state",
            "rollback",
            "--source",
            source.to_str().unwrap(),
            "--destination",
            destination.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "rollback is permanently forbidden",
        ));
}

#[test]
fn ordinary_command_refuses_until_renamed_receipt_is_recovered() {
    let home = TempDir::new().unwrap();
    let (source, destination) = terminal_source(home.path());
    let before = tree_bytes(&source);
    let mut command = bin(home.path());
    migrate(&mut command, &source, &destination);
    command.assert().success();
    set_receipt_state(home.path(), "renamed");
    bin(home.path())
        .env("TASKFLEET_HOME", &destination)
        .args(["version", "--json"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("migration_recovery_required"));
    assert_eq!(tree_bytes(&destination), before);
    let mut recover = bin(home.path());
    migrate(&mut recover, &source, &destination);
    recover.assert().success();
}

#[test]
fn node_less_terminal_run_and_output_traversal_are_handled() {
    let home = TempDir::new().unwrap();
    let (source, destination) = terminal_source(home.path());
    let run = source.join("runs/01j00000000000000000000001");
    std::fs::remove_dir_all(run.join("nodes")).unwrap();
    let manifest_path = run.join("manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    manifest["node_count"] = serde_json::json!(0);
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    let mut command = bin(home.path());
    migrate(&mut command, &source, &destination);
    command.arg("--dry-run").assert().success();

    let escaped = destination
        .parent()
        .unwrap()
        .join("outside/../canonical/out.json");
    let mut traversal = bin(home.path());
    traversal
        .args([
            "state",
            "migrate",
            "--source",
            source.to_str().unwrap(),
            "--destination",
            destination.to_str().unwrap(),
            "--dry-run",
            "--output",
            escaped.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid_output_path"));
}

#[test]
fn crash_after_rename_recovers_forward_from_prepared_receipt() {
    let home = TempDir::new().unwrap();
    let (source, destination) = terminal_source(home.path());
    let mut first = bin(home.path());
    migrate(&mut first, &source, &destination);
    first.assert().success();
    set_receipt_state(home.path(), "prepared");
    assert!(!source.exists());
    assert!(destination.exists());

    let mut retry = bin(home.path());
    migrate(&mut retry, &source, &destination);
    retry
        .assert()
        .success()
        .stdout(predicate::str::contains("\"state\": \"verified\""));
}

#[test]
fn active_fixture_and_dual_roots_refuse_without_moving() {
    let home = TempDir::new().unwrap();
    let (source, destination) = roots(home.path());
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/orchestratectl-0.5.1/home/orchestratectl");
    std::fs::create_dir_all(source.join("runs")).unwrap();
    copy_tree(
        &fixture.join("runs/01j00000000000000000000002"),
        &source.join("runs/01j00000000000000000000002"),
    );
    let mut active = bin(home.path());
    migrate(&mut active, &source, &destination);
    active
        .assert()
        .failure()
        .stderr(predicate::str::contains("every run must be terminal"));
    assert!(source.exists());
    assert!(!destination.exists());

    std::fs::create_dir(&destination).unwrap();
    let mut dual = bin(home.path());
    migrate(&mut dual, &source, &destination);
    dual.assert().failure().stderr(predicate::str::contains(
        "refusing to choose, merge, or overwrite",
    ));
}

#[test]
fn held_run_lock_refuses_promptly() {
    let home = TempDir::new().unwrap();
    let (source, destination) = terminal_source(home.path());
    let lock_path = source.join("runs/01j00000000000000000000001/.lock");
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .unwrap();
    FileExt::lock(&lock).unwrap();
    let mut command = bin(home.path());
    migrate(&mut command, &source, &destination);
    command
        .assert()
        .failure()
        .stderr(predicate::str::contains("migration requires quiescence"));
}

#[test]
fn pending_merge_and_live_worker_are_independent_quiescence_refusals() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/orchestratectl-0.5.1/home/orchestratectl");

    let pending_home = TempDir::new().unwrap();
    let (source, destination) = roots(pending_home.path());
    std::fs::create_dir_all(source.join("runs")).unwrap();
    let run = source.join("runs/01j00000000000000000000003");
    copy_tree(&fixture.join("runs/01j00000000000000000000003"), &run);
    for path in [run.join("manifest.json"), run.join("nodes/n-0001.json")] {
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        value["status"] = serde_json::Value::String("done".into());
        std::fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    }
    let pending_before = tree_bytes(&source);
    let mut pending = bin(pending_home.path());
    migrate(&mut pending, &source, &destination);
    pending
        .assert()
        .failure()
        .stderr(predicate::str::contains("pending merge transaction"));
    assert_eq!(tree_bytes(&source), pending_before);

    let live_home = TempDir::new().unwrap();
    let (source, destination) = terminal_source(live_home.path());
    let node = source.join("runs/01j00000000000000000000001/nodes/n-0001.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&node).unwrap()).unwrap();
    value["agent_pid"] = serde_json::json!(std::process::id());
    value["agent_pid_start_time"] = serde_json::Value::Null;
    std::fs::write(&node, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    let mut live = bin(live_home.path());
    migrate(&mut live, &source, &destination);
    live.assert()
        .failure()
        .stderr(predicate::str::contains("live worker pid"));

    let supervisor_home = TempDir::new().unwrap();
    let (source, destination) = terminal_source(supervisor_home.path());
    std::fs::write(
        source.join("runs/01j00000000000000000000001/supervisor.pid"),
        std::process::id().to_string(),
    )
    .unwrap();
    let mut supervisor = bin(supervisor_home.path());
    migrate(&mut supervisor, &source, &destination);
    supervisor
        .assert()
        .failure()
        .stderr(predicate::str::contains("live supervisor pid"));
}

#[test]
fn open_descriptor_does_not_claim_to_be_fenced_and_bytes_remain_readable() {
    use std::io::{Read, Seek, SeekFrom};
    let home = TempDir::new().unwrap();
    let (source, destination) = terminal_source(home.path());
    let mut open =
        std::fs::File::open(source.join("runs/01j00000000000000000000001/events.jsonl")).unwrap();
    let mut command = bin(home.path());
    migrate(&mut command, &source, &destination);
    command
        .assert()
        .success()
        .stdout(predicate::str::contains("open file descriptors"));
    open.seek(SeekFrom::Start(0)).unwrap();
    let mut bytes = Vec::new();
    open.read_to_end(&mut bytes).unwrap();
    assert!(
        !bytes.is_empty(),
        "the pre-rename descriptor remains readable; operator exclusion is explicit"
    );
}

#[test]
fn corrupt_receipt_and_recreated_legacy_root_fail_closed() {
    let home = TempDir::new().unwrap();
    let (source, destination) = terminal_source(home.path());
    let mut command = bin(home.path());
    migrate(&mut command, &source, &destination);
    command.assert().success();
    std::fs::create_dir(&source).unwrap();
    bin(home.path())
        .env("TASKFLEET_HOME", &destination)
        .args(["version", "--json"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("refusing dual roots"));
    // A sole explicit legacy selector must not bypass permanent receipt-based
    // split-truth refusal either.
    bin(home.path())
        .env("ORCHESTRATECTL_HOME", &source)
        .args(["version", "--json"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("refusing dual roots"));
    std::fs::remove_dir(&source).unwrap();

    let receipt = std::fs::read_dir(home.path().join(".taskfleet-migrations"))
        .unwrap()
        .map(Result::unwrap)
        .map(|e| e.path())
        .find(|p| p.extension().and_then(|s| s.to_str()) == Some("json"))
        .unwrap();
    std::fs::write(&receipt, b"{not-json").unwrap();
    let mut retry = bin(home.path());
    migrate(&mut retry, &source, &destination);
    retry
        .assert()
        .failure()
        .stderr(predicate::str::contains("corrupt_migration_receipt"));
}

#[test]
fn external_state_writer_fence_and_non_directory_refuse() {
    let home = TempDir::new().unwrap();
    let (source, destination) = terminal_source(home.path());
    let admin = home
        .path()
        .canonicalize()
        .unwrap()
        .join(".taskfleet-migrations");
    std::fs::create_dir(&admin).unwrap();
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(admin.join("state.lock"))
        .unwrap();
    FileExt::lock_shared(&lock).unwrap();
    let mut command = bin(home.path());
    migrate(&mut command, &source, &destination);
    command
        .assert()
        .failure()
        .stderr(predicate::str::contains("migration_lock_held"));
    FileExt::unlock(&lock).unwrap();

    let bad = home.path().canonicalize().unwrap().join("not-a-directory");
    std::fs::write(&bad, b"x").unwrap();
    let mut invalid = bin(home.path());
    migrate(&mut invalid, &bad, &destination);
    invalid
        .assert()
        .failure()
        .stderr(predicate::str::contains("real directory"));
}

#[cfg(unix)]
#[test]
fn symlink_root_is_refused() {
    use std::os::unix::fs::symlink;
    let home = TempDir::new().unwrap();
    let (real, destination) = terminal_source(home.path());
    let source = home.path().join("legacy-link");
    symlink(&real, &source).unwrap();
    let mut command = bin(home.path());
    migrate(&mut command, &source, &destination);
    command
        .assert()
        .failure()
        .stderr(predicate::str::contains("not a symlink"));
    assert!(real.exists());
}
