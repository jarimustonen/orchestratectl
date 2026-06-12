//! Integration tests for the `skill` subcommand.
//!
//! Locks the AGENTS-AI-FIRST-CLI §15 contract: list shape, refused-overwrite
//! error envelope on exit 2, and `--force` recovery. The shipped skill
//! catalog (names + non-empty descriptions) is pinned here so accidental
//! frontmatter breakage in `crates/octl-cli/skills/` is caught at CI time.

use std::process::Command;

use serde_json::Value;
use tempfile::tempdir;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_orchestratectl"))
}

#[test]
fn skill_list_json_pins_catalog_shape() {
    let out = bin()
        .args(["skill", "list", "--json"])
        .output()
        .expect("spawn");
    assert!(out.status.success(), "exit: {:?}", out.status);
    let v: Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(v["schema_version"], 1);
    let skills = v["data"]["skills"].as_array().expect("skills array");
    assert!(!skills.is_empty(), "no skills shipped");
    let names: Vec<&str> = skills.iter().map(|s| s["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"octl-run-overview"));
    assert!(names.contains(&"octl-spawn-spinoff"));
    for s in skills {
        assert!(
            !s["description"].as_str().unwrap_or("").is_empty(),
            "empty description for {}",
            s["name"]
        );
    }
}

#[test]
fn skill_show_prints_skill_md_contents() {
    let out = bin()
        .args(["skill", "show", "octl-run-overview"])
        .output()
        .expect("spawn");
    assert!(out.status.success(), "exit: {:?}", out.status);
    let stdout = String::from_utf8(out.stdout).expect("utf8");
    assert!(
        stdout.starts_with("---\nname: octl-run-overview"),
        "show did not emit frontmatter: {stdout:?}"
    );
}

#[test]
fn skill_install_refuses_overwrite_then_force_succeeds() {
    let tmp = tempdir().expect("tempdir");
    let dest = tmp.path().join("SKILL.md");

    // First install: succeeds.
    let out = bin()
        .args(["skill", "install", "octl-run-overview", "--dest"])
        .arg(&dest)
        .output()
        .expect("spawn");
    assert!(out.status.success(), "first install failed: {:?}", out);
    assert!(dest.exists(), "destination not created");

    // Second install: refused-overwrite, exit 2.
    let out = bin()
        .args(["skill", "install", "octl-run-overview", "--dest"])
        .arg(&dest)
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(2), "expected exit 2");
    let err: Value = serde_json::from_slice(&out.stderr).expect("json err envelope");
    assert_eq!(err["schema_version"], 1);
    assert_eq!(err["error"]["code"], "refused-overwrite");

    // Third install with --force: succeeds.
    let out = bin()
        .args(["skill", "install", "octl-run-overview", "--dest"])
        .arg(&dest)
        .arg("--force")
        .output()
        .expect("spawn");
    assert!(out.status.success(), "force install failed: {:?}", out);
}

#[test]
fn skill_show_unknown_emits_skill_not_found() {
    let out = bin()
        .args(["skill", "show", "no-such-skill"])
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(1));
    let err: Value = serde_json::from_slice(&out.stderr).expect("json err envelope");
    assert_eq!(err["error"]["code"], "skill_not_found");
    assert_eq!(err["error"]["invalid_value"], "no-such-skill");
}
