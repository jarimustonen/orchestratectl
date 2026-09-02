use std::process::{Command, Output};

fn run(args: &[&str], hidden_self_exec: bool) -> Output {
    let sandbox = tempfile::tempdir().expect("tempdir");
    let mut command = Command::new(env!("CARGO_BIN_EXE_orchestratectl"));
    command
        .args(args)
        .env("HOME", sandbox.path())
        .env("TASKFLEET_HOME", sandbox.path().join("state"))
        .env_remove("ORCHESTRATECTL_HOME")
        .env_remove("OCTL_INTERNAL_SELF_EXEC");
    if hidden_self_exec {
        command.env("OCTL_INTERNAL_SELF_EXEC", "1");
    }
    command.output().expect("run compatibility wrapper")
}

fn deprecation_count(stderr: &[u8]) -> usize {
    String::from_utf8_lossy(stderr)
        .lines()
        .filter(|line| line.contains("`orchestratectl` is deprecated"))
        .count()
}

#[test]
fn wrapper_uses_old_help_identity_and_emits_one_stderr_only_deprecation() {
    let output = run(&["--help"], false);
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    assert!(stdout.contains("Usage: orchestratectl [OPTIONS] <COMMAND>"));
    assert!(!stdout.contains("deprecated"));
    assert_eq!(deprecation_count(&output.stderr), 1);
}

#[test]
fn wrapper_keeps_jsonl_stdout_machine_parseable() {
    let output = run(&["--output", "jsonl", "version"], false);
    assert!(output.status.success());
    assert_eq!(deprecation_count(&output.stderr), 1);
    let stdout = std::str::from_utf8(&output.stdout).expect("utf8 JSONL");
    assert_eq!(stdout.lines().count(), 1);
    assert!(stdout.ends_with('\n'));
    let value: serde_json::Value = serde_json::from_str(stdout).expect("valid JSONL");
    assert_eq!(value["schema_version"], 1);
}

#[test]
fn hidden_self_exec_suppresses_wrapper_deprecation() {
    let output = run(&["--output", "json", "version"], true);
    assert!(output.status.success());
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice::<serde_json::Value>(&output.stdout).expect("valid JSON");
}
