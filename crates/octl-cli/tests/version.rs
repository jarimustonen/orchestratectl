//! Integration tests for the `version` subcommand.
//!
//! Locks in the AGENTS-AI-FIRST-CLI §10 contract: the JSON output is a
//! versioned API surface and any field rename / removal here is a
//! breaking change visible to every trained agent.

use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_orchestratectl"))
}

#[test]
fn version_text_succeeds_and_mentions_orchestratectl() {
    let out = bin().arg("version").output().expect("spawn");
    assert!(out.status.success(), "exit: {:?}", out.status);
    let stdout = String::from_utf8(out.stdout).expect("utf8");
    assert!(
        stdout.starts_with("orchestratectl "),
        "stdout did not start with binary name: {stdout:?}"
    );
}

#[test]
fn version_json_envelope_and_required_keys() {
    let out = bin().args(["version", "--json"]).output().expect("spawn");
    assert!(out.status.success(), "exit: {:?}", out.status);

    let stdout = String::from_utf8(out.stdout).expect("utf8");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("valid JSON");

    assert_eq!(v["schema_version"], 1);
    let data = &v["data"];

    // §10 contract + task spec
    for key in [
        "version",
        "commit",
        "supported_schemas",
        "state_schema_version",
        "supported_state_schemas",
    ] {
        assert!(!data[key].is_null(), "missing key in data: {key}");
    }

    assert!(data["version"].is_string());
    assert!(data["commit"].is_string());
    assert!(data["supported_schemas"].is_array());
    assert!(data["state_schema_version"].is_number());
    assert!(data["supported_state_schemas"].is_array());
}
