//! Integration tests for the `skill` subcommand.
//!
//! Locks the AGENTS-AI-FIRST-CLI §15 contract: list shape, refused_overwrite
//! error envelope on exit 2, `--force` recovery, the install-all (no-name)
//! form, and `--agent`/`--dest` mutual-exclusion rules. The shipped skill
//! catalog (names + non-empty descriptions) is pinned here so accidental
//! frontmatter breakage in `crates/octl-cli/skills/` is caught at CI time.
//!
//! Every test sets `ORCHESTRATECTL_HOME` (and `HOME` where the install
//! path resolves it) to a tempdir so we never mutate the developer's real
//! `~/.orchestratectl/` or `~/.claude/` directories under CI or local
//! `cargo test`.

use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;
use tempfile::TempDir;

fn bin(home: &TempDir) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_orchestratectl"));
    // Sandbox log writes and any HOME-derived install paths into the
    // tempdir so test runs never touch the real `~/.orchestratectl/` or
    // `~/.claude/`.
    cmd.env("ORCHESTRATECTL_HOME", home.path());
    cmd.env("HOME", home.path());
    cmd
}

fn mk_home() -> TempDir {
    tempfile::tempdir().expect("tempdir")
}

#[test]
fn skill_list_json_pins_catalog_shape() {
    let home = mk_home();
    let out = bin(&home)
        .args(["skill", "list", "--json"])
        .output()
        .expect("spawn");
    assert!(out.status.success(), "exit: {:?}", out.status);
    let v: Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(v["schema_version"], 1);
    let skills = v["data"]["skills"].as_array().expect("skills array");
    let mut names: Vec<&str> = skills.iter().map(|s| s["name"].as_str().unwrap()).collect();
    names.sort();
    // Pin the exact catalog: a silent addition or removal must show up
    // as a test failure so the consumer-facing surface stays explicit.
    assert_eq!(names, vec!["octl-run-overview", "octl-spawn-spinoff"]);
    for s in skills {
        assert!(
            !s["description"].as_str().unwrap_or("").is_empty(),
            "empty description for {}",
            s["name"]
        );
    }
}

#[test]
fn skill_show_text_prints_skill_md_contents() {
    let home = mk_home();
    let out = bin(&home)
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
fn skill_show_json_wraps_content_under_data() {
    let home = mk_home();
    let out = bin(&home)
        .args(["skill", "show", "octl-run-overview", "--json"])
        .output()
        .expect("spawn");
    assert!(out.status.success(), "exit: {:?}", out.status);
    let v: Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(v["schema_version"], 1);
    assert_eq!(v["data"]["name"], "octl-run-overview");
    let content = v["data"]["content"].as_str().expect("content str");
    assert!(content.starts_with("---\nname: octl-run-overview"));
}

#[test]
fn skill_install_refuses_overwrite_then_force_succeeds() {
    let home = mk_home();
    let dest = home.path().join("SKILL.md");

    let out = bin(&home)
        .args(["skill", "install", "octl-run-overview", "--dest"])
        .arg(&dest)
        .output()
        .expect("spawn");
    assert!(out.status.success(), "first install failed: {:?}", out);
    assert!(dest.exists(), "destination not created");

    let out = bin(&home)
        .args(["skill", "install", "octl-run-overview", "--dest"])
        .arg(&dest)
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(2), "expected exit 2");
    let err: Value = serde_json::from_slice(&out.stderr).expect("json err envelope");
    assert_eq!(err["schema_version"], 1);
    // The error code is snake_case per the project-wide convention; the
    // contract is shared with every other subcommand and AI callers
    // branch on the exact string.
    assert_eq!(err["error"]["code"], "refused_overwrite");
    assert_eq!(err["error"]["invalid_value"], dest.display().to_string());

    let out = bin(&home)
        .args(["skill", "install", "octl-run-overview", "--dest"])
        .arg(&dest)
        .arg("--force")
        .output()
        .expect("spawn");
    assert!(out.status.success(), "force install failed: {:?}", out);
}

#[test]
fn skill_install_with_default_paths_writes_under_home() {
    let home = mk_home();
    let out = bin(&home)
        .args(["skill", "install", "octl-run-overview", "--json"])
        .output()
        .expect("spawn");
    assert!(out.status.success(), "default install failed: {:?}", out);
    let v: Value = serde_json::from_slice(&out.stdout).expect("json");
    let installed = v["data"]["installed"].as_array().expect("installed array");
    assert_eq!(installed.len(), 1);
    assert_eq!(installed[0]["agent"], "claude");
    let expected: PathBuf = home
        .path()
        .join(".claude/skills/octl-run-overview/SKILL.md");
    assert_eq!(installed[0]["path"], expected.display().to_string());
    assert!(expected.exists(), "claude install not on disk");
}

#[test]
fn skill_install_agent_all_installs_to_both_default_paths() {
    let home = mk_home();
    let out = bin(&home)
        .args([
            "skill",
            "install",
            "octl-spawn-spinoff",
            "--agent",
            "all",
            "--json",
        ])
        .output()
        .expect("spawn");
    assert!(out.status.success(), "agent=all install failed: {:?}", out);
    let v: Value = serde_json::from_slice(&out.stdout).expect("json");
    let installed = v["data"]["installed"].as_array().expect("installed");
    let agents: Vec<&str> = installed
        .iter()
        .map(|f| f["agent"].as_str().unwrap())
        .collect();
    assert!(agents.contains(&"claude"));
    assert!(agents.contains(&"codex"));
    assert!(home
        .path()
        .join(".claude/skills/octl-spawn-spinoff/SKILL.md")
        .exists());
    assert!(home
        .path()
        .join(".codex/prompts/octl-spawn-spinoff.md")
        .exists());
}

#[test]
fn skill_install_no_name_installs_every_skill() {
    let home = mk_home();
    let out = bin(&home)
        .args(["skill", "install", "--json"])
        .output()
        .expect("spawn");
    assert!(out.status.success(), "install-all failed: {:?}", out);
    let v: Value = serde_json::from_slice(&out.stdout).expect("json");
    let installed = v["data"]["installed"].as_array().expect("installed");
    let names: Vec<&str> = installed
        .iter()
        .map(|f| f["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"octl-run-overview"));
    assert!(names.contains(&"octl-spawn-spinoff"));
}

#[test]
fn skill_install_agent_all_with_dest_is_rejected() {
    let home = mk_home();
    let dest = home.path().join("SKILL.md");
    let out = bin(&home)
        .args([
            "skill",
            "install",
            "octl-run-overview",
            "--agent",
            "all",
            "--dest",
        ])
        .arg(&dest)
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(1), "expected user-error exit 1");
    let err: Value = serde_json::from_slice(&out.stderr).expect("err json");
    assert_eq!(err["error"]["code"], "invalid_arguments");
}

#[test]
fn skill_install_partial_failure_is_preflighted() {
    // Pre-create the codex destination so the second target in the plan
    // would fail. The preflight pass must refuse the whole install before
    // writing the claude target, so the user can retry once they decide
    // to pass --force, instead of being stuck in a half-installed state.
    let home = mk_home();
    let codex = home.path().join(".codex/prompts");
    std::fs::create_dir_all(&codex).unwrap();
    std::fs::write(codex.join("octl-run-overview.md"), "pre-existing").unwrap();

    let out = bin(&home)
        .args(["skill", "install", "octl-run-overview", "--agent", "all"])
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(2));
    let err: Value = serde_json::from_slice(&out.stderr).expect("err json");
    assert_eq!(err["error"]["code"], "refused_overwrite");
    // Crucially, the claude target was *not* touched — preflight saw the
    // codex collision and bailed before any write.
    assert!(!home
        .path()
        .join(".claude/skills/octl-run-overview/SKILL.md")
        .exists());
}

#[test]
fn skill_install_accepts_bare_relative_dest() {
    let home = mk_home();
    // Bare relative path: parent is the empty string; normalized_parent
    // must coerce it to "." so create_dir_all does not blow up.
    let out = bin(&home)
        .current_dir(home.path())
        .args([
            "skill",
            "install",
            "octl-run-overview",
            "--dest",
            "SKILL.md",
        ])
        .output()
        .expect("spawn");
    assert!(out.status.success(), "bare-dest install failed: {:?}", out);
    assert!(home.path().join("SKILL.md").exists());
}

#[test]
fn skill_show_unknown_emits_skill_not_found() {
    let home = mk_home();
    let out = bin(&home)
        .args(["skill", "show", "no-such-skill"])
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(1));
    let err: Value = serde_json::from_slice(&out.stderr).expect("json err envelope");
    assert_eq!(err["error"]["code"], "skill_not_found");
    assert_eq!(err["error"]["invalid_value"], "no-such-skill");
}
