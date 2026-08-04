//! Integration tests for the `skill` subcommand.
//!
//! Locks the AGENTS-AI-FIRST-CLI §15 contract: list shape, `refused_overwrite`
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
        .args(["skill", "list", "--output", "json"])
        .output()
        .expect("spawn");
    assert!(out.status.success(), "exit: {:?}", out.status);
    let v: Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(v["schema_version"], 1);
    let skills = v["data"]["skills"].as_array().expect("skills array");
    let mut names: Vec<&str> = skills.iter().map(|s| s["name"].as_str().unwrap()).collect();
    names.sort_unstable();
    // Pin the exact catalog: a silent addition or removal must show up
    // as a test failure so the consumer-facing surface stays explicit.
    assert_eq!(
        names,
        vec![
            "fan-out",
            "octl-run-overview",
            "octl-spawn-spinoff",
            "orchestrate",
            "orchestratectl-overview",
            "stint-handoff",
            "stint-start",
            "worktree",
            "worktree-bug-analysis",
            "worktree-bugfix",
            "worktree-code",
            "worktree-make-skill",
            "worktree-merge",
            "worktree-orchestrated",
            "worktree-research",
            "worktree-spinoff",
            "worktree-status",
            "worktree-technical-decision",
        ]
    );
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
        .args(["--output", "text", "skill", "show", "octl-run-overview"])
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
        .args(["skill", "show", "octl-run-overview", "--output", "json"])
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
    assert!(out.status.success(), "first install failed: {out:?}");
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
    assert!(out.status.success(), "force install failed: {out:?}");
}

#[test]
fn skill_install_with_default_paths_writes_under_home() {
    let home = mk_home();
    let out = bin(&home)
        .args(["skill", "install", "octl-run-overview", "--output", "json"])
        .output()
        .expect("spawn");
    assert!(out.status.success(), "default install failed: {out:?}");
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
            "--output",
            "json",
        ])
        .output()
        .expect("spawn");
    assert!(out.status.success(), "agent=all install failed: {out:?}");
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
        .args(["skill", "install", "--output", "json"])
        .output()
        .expect("spawn");
    assert!(out.status.success(), "install-all failed: {out:?}");
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
    assert!(out.status.success(), "bare-dest install failed: {out:?}");
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

#[test]
fn skill_print_default_streams_skill_md_byte_identically() {
    // §16: `skill print` (default `--output jsonl`) writes the embedded
    // SKILL.md byte-identically — no envelope wrapping. The bytes must
    // equal what `skill install` would persist.
    let home = mk_home();
    let print_out = bin(&home)
        .args(["skill", "print", "orchestratectl-overview"])
        .output()
        .expect("spawn");
    assert!(print_out.status.success(), "exit: {:?}", print_out.status);
    let dest = home.path().join("printed.md");
    let install_out = bin(&home)
        .args(["skill", "install", "orchestratectl-overview", "--dest"])
        .arg(&dest)
        .output()
        .expect("spawn");
    assert!(install_out.status.success());
    let on_disk = std::fs::read(&dest).expect("read installed");
    assert_eq!(
        print_out.stdout, on_disk,
        "skill print stdout must equal skill install on-disk bytes"
    );
}

#[test]
fn skill_print_json_payload_pins_schema() {
    let home = mk_home();
    let out = bin(&home)
        .args([
            "skill",
            "print",
            "orchestratectl-overview",
            "--output",
            "json",
        ])
        .output()
        .expect("spawn");
    assert!(out.status.success(), "exit: {:?}", out.status);
    let v: Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(v["schema_version"], 1);
    let data = &v["data"];
    assert_eq!(data["name"], "orchestratectl-overview");
    assert_eq!(data["schema_version"], 1);
    assert_eq!(data["schema_version_skill"], 1);
    assert_eq!(
        data["cli_version"].as_str().unwrap(),
        env!("CARGO_PKG_VERSION")
    );
    assert!(data["content"]
        .as_str()
        .unwrap()
        .starts_with("---\nname: orchestratectl-overview"));
    assert!(data["path_in_repo"]
        .as_str()
        .unwrap()
        .contains("SKILL.template.md"));
}

#[test]
fn skill_print_unknown_emits_skill_not_found() {
    let home = mk_home();
    let out = bin(&home)
        .args(["skill", "print", "no-such-skill"])
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(1));
    let err: Value = serde_json::from_slice(&out.stderr).expect("err envelope");
    assert_eq!(err["error"]["code"], "skill_not_found");
}

#[test]
fn skill_install_over_older_version_warns_and_succeeds_without_force() {
    // §17 drift: install over an older on-disk skill must proceed
    // without --force and surface a `skill_version_drift` warning so the
    // agent learns the operating manual just moved.
    let home = mk_home();
    let dest = home.path().join("SKILL.md");
    // Hand-write an "older" skill — cli_version 0.0.0 will always be
    // older than the current binary.
    std::fs::write(
        &dest,
        "---\nname: orchestratectl-overview\ndescription: old\ncli_version: \"0.0.0\"\nschema_version: 1\n---\n",
    )
    .unwrap();

    let out = bin(&home)
        .args(["skill", "install", "orchestratectl-overview", "--dest"])
        .arg(&dest)
        .args(["--output", "json"])
        .output()
        .expect("spawn");
    assert!(
        out.status.success(),
        "install over older must succeed: {out:?}"
    );
    let v: Value = serde_json::from_slice(&out.stdout).expect("json");
    let warnings = v["warnings"].as_array().expect("warnings array");
    assert!(
        warnings
            .iter()
            .any(|w| w.as_str().unwrap().contains("skill_version_drift")
                && w.as_str().unwrap().contains("0.0.0")),
        "expected skill_version_drift warning naming 0.0.0; got {warnings:?}"
    );
    // File was overwritten with the bundled body.
    let after = std::fs::read_to_string(&dest).unwrap();
    assert!(after.contains(&format!("cli_version: \"{}\"", env!("CARGO_PKG_VERSION"))));
}

#[test]
fn skill_install_over_newer_version_refuses_with_skill_version_too_new() {
    // §17 drift: install over a newer on-disk skill refuses with exit 2
    // and `skill_version_too_new` unless --force is passed.
    let home = mk_home();
    let dest = home.path().join("SKILL.md");
    std::fs::write(
        &dest,
        "---\nname: orchestratectl-overview\ndescription: future\ncli_version: \"99.0.0\"\nschema_version: 1\n---\n",
    )
    .unwrap();

    let out = bin(&home)
        .args(["skill", "install", "orchestratectl-overview", "--dest"])
        .arg(&dest)
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(2), "expected exit 2");
    let err: Value = serde_json::from_slice(&out.stderr).expect("err envelope");
    assert_eq!(err["error"]["code"], "skill_version_too_new");

    // --force overrides the refusal.
    let out = bin(&home)
        .args(["skill", "install", "orchestratectl-overview", "--dest"])
        .arg(&dest)
        .arg("--force")
        .output()
        .expect("spawn");
    assert!(out.status.success(), "--force install must succeed");
}

const MARKER: &str = ".orchestratectl-managed";

#[test]
fn skill_install_default_stamps_provenance_marker() {
    // Every default claude install must drop the provenance marker beside
    // SKILL.md — it's what makes later pruning safe.
    let home = mk_home();
    assert!(bin(&home)
        .args(["skill", "install", "octl-run-overview"])
        .output()
        .expect("spawn")
        .status
        .success());
    assert!(
        home.path()
            .join(".claude/skills/octl-run-overview")
            .join(MARKER)
            .is_file(),
        "provenance marker not written next to SKILL.md"
    );
}

#[test]
fn skill_install_all_prunes_managed_orphan() {
    // A managed skill dir that is NOT in the catalog (carries the marker)
    // must be pruned by the full-catalog install.
    let home = mk_home();
    let orphan = home.path().join(".claude/skills/gone-skill");
    std::fs::create_dir_all(&orphan).unwrap();
    std::fs::write(orphan.join("SKILL.md"), "---\nname: gone-skill\n---\n").unwrap();
    std::fs::write(orphan.join(MARKER), "managed-by: orchestratectl\n").unwrap();

    let out = bin(&home)
        .args(["skill", "install", "--force", "--output", "json"])
        .output()
        .expect("spawn");
    assert!(out.status.success(), "install-all failed: {out:?}");
    assert!(!orphan.exists(), "managed orphan was not pruned");

    let v: Value = serde_json::from_slice(&out.stdout).expect("json");
    let pruned: Vec<&str> = v["data"]["pruned"]
        .as_array()
        .expect("pruned array")
        .iter()
        .map(|p| p.as_str().unwrap())
        .collect();
    assert!(pruned.contains(&"gone-skill"), "pruned list: {pruned:?}");
    let warnings = v["warnings"].as_array().expect("warnings");
    assert!(
        warnings
            .iter()
            .any(|w| w.as_str().unwrap().contains("skill_pruned")
                && w.as_str().unwrap().contains("gone-skill")),
        "expected skill_pruned warning; got {warnings:?}"
    );
}

#[test]
fn skill_install_all_spares_unmanaged_same_name_dir() {
    // A user's hand-authored skill dir WITHOUT the marker must never be
    // touched, even though it is not in the catalog.
    let home = mk_home();
    let user_skill = home.path().join(".claude/skills/my-own-skill");
    std::fs::create_dir_all(&user_skill).unwrap();
    std::fs::write(
        user_skill.join("SKILL.md"),
        "---\nname: my-own-skill\n---\nmine\n",
    )
    .unwrap();

    let out = bin(&home)
        .args(["skill", "install", "--force", "--output", "json"])
        .output()
        .expect("spawn");
    assert!(out.status.success(), "install-all failed: {out:?}");
    assert!(
        user_skill.join("SKILL.md").exists(),
        "unmanaged user skill was deleted — provenance guard failed"
    );
    let v: Value = serde_json::from_slice(&out.stdout).expect("json");
    let pruned = v["data"]["pruned"].as_array().expect("pruned array");
    assert!(
        pruned.is_empty(),
        "unmanaged dir must not appear in pruned: {pruned:?}"
    );
}

#[test]
fn skill_install_all_keeps_registered_skills() {
    // A still-registered skill (installed with its marker) must survive a
    // subsequent full-catalog install — it is not an orphan.
    let home = mk_home();
    assert!(bin(&home)
        .args(["skill", "install", "--force"])
        .output()
        .expect("spawn")
        .status
        .success());
    let registered = home.path().join(".claude/skills/octl-run-overview");
    assert!(registered.join("SKILL.md").exists());
    assert!(registered.join(MARKER).is_file());

    // Second full install must NOT prune the still-registered skill.
    let out = bin(&home)
        .args(["skill", "install", "--force", "--output", "json"])
        .output()
        .expect("spawn");
    assert!(out.status.success(), "second install failed: {out:?}");
    assert!(
        registered.join("SKILL.md").exists(),
        "still-registered skill was pruned"
    );
    let v: Value = serde_json::from_slice(&out.stdout).expect("json");
    assert!(v["data"]["pruned"].as_array().expect("pruned").is_empty());
}

#[test]
fn skill_install_named_does_not_prune() {
    // A targeted `skill install <name>` must NEVER prune the rest of the
    // catalog — even a managed orphan is left alone.
    let home = mk_home();
    let orphan = home.path().join(".claude/skills/gone-skill");
    std::fs::create_dir_all(&orphan).unwrap();
    std::fs::write(orphan.join("SKILL.md"), "---\nname: gone-skill\n---\n").unwrap();
    std::fs::write(orphan.join(MARKER), "managed-by: orchestratectl\n").unwrap();

    let out = bin(&home)
        .args([
            "skill",
            "install",
            "octl-run-overview",
            "--force",
            "--output",
            "json",
        ])
        .output()
        .expect("spawn");
    assert!(out.status.success(), "named install failed: {out:?}");
    assert!(
        orphan.exists(),
        "targeted install pruned an orphan — must be scoped to install-all"
    );
    let v: Value = serde_json::from_slice(&out.stdout).expect("json");
    assert!(v["data"]["pruned"].as_array().expect("pruned").is_empty());
}

#[test]
fn skill_install_stint_start_writes_companion_resource_for_claude() {
    // stint-start ships a companion reference (AGENTS-EXECUTION-DAG.md);
    // the default claude install must write it as a sibling of SKILL.md so
    // the skill's in-body link resolves at runtime, and report it in the
    // install payload.
    let home = mk_home();
    let out = bin(&home)
        .args(["skill", "install", "stint-start", "--output", "json"])
        .output()
        .expect("spawn");
    assert!(out.status.success(), "install failed: {out:?}");

    let skill_md = home.path().join(".claude/skills/stint-start/SKILL.md");
    let companion = home
        .path()
        .join(".claude/skills/stint-start/AGENTS-EXECUTION-DAG.md");
    assert!(skill_md.exists(), "SKILL.md not installed");
    assert!(
        companion.exists(),
        "companion AGENTS-EXECUTION-DAG.md not installed alongside SKILL.md"
    );

    let v: Value = serde_json::from_slice(&out.stdout).expect("json");
    let installed = v["data"]["installed"].as_array().expect("installed array");
    let paths: Vec<&str> = installed
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert!(
        paths.iter().any(|p| p.ends_with("AGENTS-EXECUTION-DAG.md")),
        "companion not reported in install payload: {paths:?}"
    );
}

#[test]
fn skill_install_stint_start_codex_skips_companion() {
    // The codex layout is a flat prompts dir; a per-skill sibling would
    // land un-namespaced and could collide across skills. Resources are
    // therefore claude-only — codex gets the flat prompt but no companion.
    let home = mk_home();
    let out = bin(&home)
        .args([
            "skill",
            "install",
            "stint-start",
            "--agent",
            "codex",
            "--output",
            "json",
        ])
        .output()
        .expect("spawn");
    assert!(out.status.success(), "codex install failed: {out:?}");
    assert!(
        home.path().join(".codex/prompts/stint-start.md").exists(),
        "flat codex prompt not installed"
    );
    assert!(
        !home
            .path()
            .join(".codex/prompts/AGENTS-EXECUTION-DAG.md")
            .exists(),
        "companion must not leak into the flat codex prompts dir"
    );
}

#[test]
fn skill_install_over_older_companion_upgrades_without_force() {
    // §17 drift must apply to companion resources too: the shipped
    // companion carries `cli_version` frontmatter, so a redeploy over an
    // older on-disk copy overwrites it (with a warning) WITHOUT --force,
    // exactly like SKILL.md. A version-less companion would wrongly force
    // `--force` on every catalog update — this pins the fix.
    let home = mk_home();
    assert!(bin(&home)
        .args(["skill", "install", "stint-start"])
        .output()
        .expect("spawn")
        .status
        .success());

    let skill_md = home.path().join(".claude/skills/stint-start/SKILL.md");
    let companion = home
        .path()
        .join(".claude/skills/stint-start/AGENTS-EXECUTION-DAG.md");
    // Make BOTH on-disk files look older than the binary so the whole
    // plan qualifies for the drift-upgrade (no-force) path.
    std::fs::write(
        &skill_md,
        "---\nname: stint-start\ndescription: old\ncli_version: \"0.0.0\"\nschema_version: 1\n---\n",
    )
    .unwrap();
    std::fs::write(
        &companion,
        "---\ncli_version: \"0.0.0\"\nschema_version: 1\n---\nstale\n",
    )
    .unwrap();

    let out = bin(&home)
        .args(["skill", "install", "stint-start"])
        .output()
        .expect("spawn");
    assert!(
        out.status.success(),
        "redeploy over an older companion must not require --force: {out:?}"
    );
    let after = std::fs::read_to_string(&companion).unwrap();
    assert!(
        after.contains(&format!("cli_version: \"{}\"", env!("CARGO_PKG_VERSION"))),
        "companion was not upgraded to the binary version"
    );
}
