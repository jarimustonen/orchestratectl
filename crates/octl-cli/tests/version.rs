//! Integration tests for the `version` subcommand.
//!
//! Locks in the AGENTS-AI-FIRST-CLI §10 contract: the JSON output is a
//! versioned API surface and any field rename / removal here is a
//! breaking change visible to every trained agent. Tests pin the
//! **exact** key set (not just presence) so accidental field additions
//! within `schema_version == 1` surface as test failures.

use std::collections::BTreeSet;
use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_orchestratectl"))
}

#[test]
fn version_text_succeeds_with_clean_stderr() {
    let out = bin()
        .args(["--output", "text", "version"])
        .output()
        .expect("spawn");
    assert!(out.status.success(), "exit: {:?}", out.status);
    let stdout = String::from_utf8(out.stdout).expect("utf8");
    assert!(
        stdout.starts_with("orchestratectl "),
        "stdout did not start with binary name: {stdout:?}"
    );
    assert!(
        stdout.contains("commit:"),
        "missing commit line: {stdout:?}"
    );
    assert!(
        stdout.contains("state schema version:"),
        "missing state schema line: {stdout:?}"
    );
    // Successful runs do not emit on stderr — warnings would land
    // there, but the version path has none.
    assert!(
        out.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn version_json_pins_envelope_and_payload_shape() {
    let out = bin()
        .args(["version", "--output", "json"])
        .output()
        .expect("spawn");
    assert!(out.status.success(), "exit: {:?}", out.status);
    assert!(
        out.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8(out.stdout).expect("utf8");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("valid JSON");

    // Envelope: schema_version + data. `warnings` is currently elided
    // when empty (TODO: tracked spin-off issue to make it always
    // emitted). Pin the *current* behavior so an unintended addition
    // there surfaces immediately.
    let env_keys: BTreeSet<&str> = v
        .as_object()
        .expect("envelope is object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        env_keys,
        BTreeSet::from(["schema_version", "data"]),
        "unexpected envelope keys: {env_keys:?}"
    );

    assert_eq!(v["schema_version"], 1);

    // Payload: §10 + task spec — exact key set.
    let data = &v["data"];
    let data_keys: BTreeSet<&str> = data
        .as_object()
        .expect("data is object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        data_keys,
        BTreeSet::from([
            "version",
            "commit",
            "schema_version",
            "supported_schemas",
            "state_schema_version",
            "supported_state_schemas",
        ]),
        "unexpected data keys: {data_keys:?}"
    );

    // Exact values where stable.
    assert_eq!(data["schema_version"], 1);
    assert_eq!(data["supported_schemas"], serde_json::json!([1]));
    assert_eq!(data["state_schema_version"], 1);
    assert_eq!(data["supported_state_schemas"], serde_json::json!([1]));

    // Version matches the CLI crate's Cargo version. Both the test
    // binary and the CLI binary are built from the same crate, so
    // `CARGO_PKG_VERSION` here equals the value baked into the binary.
    assert_eq!(data["version"], env!("CARGO_PKG_VERSION"));

    // Commit is either "unknown" (no .git) or a 40-char lowercase hex
    // SHA. Without this check, `build.rs` could regress to embedding
    // `"refs/heads/main\n"` or an empty string and tests would still
    // pass.
    let commit = data["commit"].as_str().expect("commit is string");
    let is_unknown = commit == "unknown";
    let is_sha = commit.len() == 40
        && commit
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c));
    assert!(
        is_unknown || is_sha,
        "commit must be 'unknown' or 40-char lowercase hex SHA, got: {commit:?}"
    );
}

#[test]
fn version_jsonl_default_is_single_line_envelope() {
    // `--output jsonl` is the new default. A single-payload subcommand
    // emits the envelope as one compact line (no trailing newline beyond
    // the line terminator), parseable by `serde_json::from_str`.
    let out = bin()
        .args(["--output", "jsonl", "version"])
        .output()
        .expect("spawn");
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout.clone()).expect("utf8");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines.len(),
        1,
        "expected single envelope line, got: {stdout:?}"
    );
    let v: serde_json::Value = serde_json::from_str(lines[0]).expect("valid JSON");
    assert_eq!(v["schema_version"], 1);
    assert!(v["data"].is_object());

    // The bare `version` invocation (no flag) takes the same path.
    let bare = bin().arg("version").output().expect("spawn");
    assert!(bare.status.success());
    assert_eq!(bare.stdout, out.stdout, "default must equal --output jsonl");
}

#[test]
fn version_rejects_legacy_json_flag() {
    // `--json` is gone; passing it must error with `unknown_subcommand_or_flag`.
    let out = bin().args(["--json", "version"]).output().expect("spawn");
    assert!(!out.status.success(), "expected non-zero exit");
    let stderr = String::from_utf8(out.stderr).expect("utf8");
    let last = stderr.lines().last().expect("stderr non-empty");
    let v: serde_json::Value = serde_json::from_str(last).expect("error envelope is valid JSON");
    assert_eq!(v["error"]["code"], "unknown_subcommand_or_flag");
}

#[test]
fn version_rejects_unknown_flag_with_structured_error() {
    let out = bin()
        .args(["version", "--definitely-not-a-real-flag"])
        .output()
        .expect("spawn");

    assert!(!out.status.success(), "expected non-zero exit");
    assert!(
        out.stdout.is_empty(),
        "errors must not write stdout: {}",
        String::from_utf8_lossy(&out.stdout)
    );

    // Error envelope on stderr — must be valid JSON per §10.
    let stderr = String::from_utf8(out.stderr).expect("utf8");
    let last = stderr.lines().last().expect("stderr non-empty");
    let v: serde_json::Value = serde_json::from_str(last).expect("error envelope is valid JSON");
    assert_eq!(v["schema_version"], 1);
    assert!(v["error"].is_object(), "error body missing");
    assert!(v["error"]["code"].is_string(), "error.code missing");
}
