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
    // A default install dual-homes the skill: the claude copy plus the pi
    // mirror (see `skill_install_default_dual_homes_into_pi`).
    let claude = installed
        .iter()
        .find(|f| f["agent"] == "claude")
        .expect("claude entry");
    let expected: PathBuf = home
        .path()
        .join(".claude/skills/octl-run-overview/SKILL.md");
    assert_eq!(claude["path"], expected.display().to_string());
    assert!(expected.exists(), "claude install not on disk");
}

#[test]
fn skill_install_force_prunes_orphan_companion_file() {
    // A prior binary installed `stint-start` with an extra companion the
    // current binary no longer ships. It survives on disk and its
    // provenance marker still records it. A `--force` re-install must remove
    // the orphan file and report it under `pruned_companions`, while leaving
    // the still-bundled companion(s) in place.
    let home = mk_home();
    // First install lays down the skill dir + marker.
    let first = bin(&home)
        .args(["skill", "install", "stint-start", "--output", "json"])
        .output()
        .expect("spawn");
    assert!(first.status.success(), "first install failed: {first:?}");

    let skill_dir = home.path().join(".claude/skills/stint-start");
    let marker = skill_dir.join(".orchestratectl-managed");
    let orphan = skill_dir.join("OLD-COMPANION.md");

    // Simulate the prior binary: drop the orphan file and record it in the
    // marker alongside whatever the marker already holds.
    std::fs::write(&orphan, "stale companion\n").expect("write orphan");
    let mut marker_body = std::fs::read_to_string(&marker).expect("read marker");
    marker_body.push_str("companion: OLD-COMPANION.md\n");
    std::fs::write(&marker, marker_body).expect("append marker");
    // Sanity: the real bundled companion is present and stays present.
    let real_companion = skill_dir.join("AGENTS-EXECUTION-DAG.md");
    assert!(
        real_companion.exists(),
        "bundled companion missing after install"
    );

    // Forced re-install prunes the orphan.
    let out = bin(&home)
        .args([
            "skill",
            "install",
            "stint-start",
            "--force",
            "--output",
            "json",
        ])
        .output()
        .expect("spawn");
    assert!(out.status.success(), "force reinstall failed: {out:?}");
    let v: Value = serde_json::from_slice(&out.stdout).expect("json");
    let pruned: Vec<&str> = v["data"]["pruned_companions"]
        .as_array()
        .expect("pruned_companions array")
        .iter()
        .map(|e| e.as_str().unwrap())
        .collect();
    assert_eq!(pruned, vec!["stint-start/OLD-COMPANION.md"]);
    assert!(!orphan.exists(), "orphan companion not removed by --force");
    assert!(real_companion.exists(), "bundled companion wrongly removed");
    // The rewritten marker no longer records the pruned orphan.
    let marker_after = std::fs::read_to_string(&marker).expect("read marker after");
    assert!(
        !marker_after.contains("OLD-COMPANION.md"),
        "marker still records the pruned orphan: {marker_after:?}"
    );
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

/// Write a valid provenance marker naming `skill_name` into `dir` — the
/// shape `is_managed_skill_dir` accepts.
fn write_marker(dir: &std::path::Path, skill_name: &str) {
    std::fs::write(
        dir.join(MARKER),
        format!("managed-by: orchestratectl\ncli_version: 9.9.9\nskill_name: {skill_name}\n"),
    )
    .unwrap();
}

#[test]
fn skill_install_default_stamps_provenance_marker() {
    // Every default claude install must drop the provenance marker beside
    // SKILL.md — it's what makes later pruning safe. The marker must name
    // the skill so a copied-and-renamed dir is never mistaken for it.
    let home = mk_home();
    assert!(bin(&home)
        .args(["skill", "install", "octl-run-overview"])
        .output()
        .expect("spawn")
        .status
        .success());
    let marker = home
        .path()
        .join(".claude/skills/octl-run-overview")
        .join(MARKER);
    assert!(
        marker.is_file(),
        "provenance marker not written next to SKILL.md"
    );
    let body = std::fs::read_to_string(&marker).unwrap();
    assert!(
        body.contains("managed-by: orchestratectl"),
        "marker: {body}"
    );
    assert!(
        body.contains("skill_name: octl-run-overview"),
        "marker: {body}"
    );
}

#[test]
fn skill_install_default_dual_homes_into_pi() {
    // A default `skill install` writes the claude copy AND mirrors the same
    // SKILL.md into pi.dev's per-skill dir (`~/.pi/agent/skills/<name>/`),
    // byte-for-byte identical. The claude path is proven unchanged: same
    // location, same bytes it has always written.
    let home = mk_home();
    let out = bin(&home)
        .args(["skill", "install", "stint-start", "--output", "json"])
        .output()
        .expect("spawn");
    assert!(out.status.success(), "default install failed: {out:?}");
    let v: Value = serde_json::from_slice(&out.stdout).expect("json");
    let installed = v["data"]["installed"].as_array().expect("installed array");
    let agents: Vec<&str> = installed
        .iter()
        .map(|f| f["agent"].as_str().unwrap())
        .collect();
    assert!(
        agents.contains(&"claude"),
        "claude entry missing: {agents:?}"
    );
    assert!(agents.contains(&"pi"), "pi entry missing: {agents:?}");

    let claude = home.path().join(".claude/skills/stint-start/SKILL.md");
    let pi = home.path().join(".pi/agent/skills/stint-start/SKILL.md");
    assert!(claude.exists(), "claude SKILL.md not on disk");
    assert!(pi.exists(), "pi SKILL.md not on disk");
    assert_eq!(
        std::fs::read(&claude).unwrap(),
        std::fs::read(&pi).unwrap(),
        "pi mirror must be byte-identical to the claude SKILL.md"
    );

    // Vendored filter: only SKILL.md is mirrored into pi — companion
    // resources (stint-start ships AGENTS-EXECUTION-DAG.md) stay claude-only.
    assert!(
        home.path()
            .join(".claude/skills/stint-start/AGENTS-EXECUTION-DAG.md")
            .exists(),
        "companion missing from claude dir"
    );
    assert!(
        !home
            .path()
            .join(".pi/agent/skills/stint-start/AGENTS-EXECUTION-DAG.md")
            .exists(),
        "companion must NOT be mirrored into the pi dir"
    );
    // The pi mirror is not a managed claude dir — no provenance marker.
    assert!(
        !home
            .path()
            .join(".pi/agent/skills/stint-start")
            .join(MARKER)
            .is_file(),
        "pi mirror must not carry the claude provenance marker"
    );
}

#[test]
fn skill_install_repairs_missing_claude_when_pi_mirror_is_current() {
    // F1: the derived pi mirror must never gate the primary claude install.
    // Divergent state — pi present + current, claude deleted — must still let
    // a plain (no --force) install re-create the claude skill. Before the F1
    // fix this aborted the whole plan with refused_overwrite on the pi path.
    let home = mk_home();
    // First install populates both homes.
    assert!(bin(&home)
        .args(["skill", "install", "stint-start"])
        .output()
        .expect("spawn")
        .status
        .success());
    let claude = home.path().join(".claude/skills/stint-start/SKILL.md");
    let pi = home.path().join(".pi/agent/skills/stint-start/SKILL.md");
    assert!(claude.exists() && pi.exists());
    let pi_bytes_before = std::fs::read(&pi).unwrap();

    // Delete only the claude copy, leaving the current pi mirror behind.
    std::fs::remove_dir_all(home.path().join(".claude/skills/stint-start")).unwrap();

    // A plain re-install (no --force) must succeed and repair claude.
    let out = bin(&home)
        .args(["skill", "install", "stint-start", "--output", "json"])
        .output()
        .expect("spawn");
    assert!(
        out.status.success(),
        "plain re-install must repair claude despite a current pi mirror: {out:?}"
    );
    assert!(claude.exists(), "claude skill was not repaired");
    // The untouched pi mirror is left byte-for-byte in place.
    assert_eq!(
        std::fs::read(&pi).unwrap(),
        pi_bytes_before,
        "pi mirror must be left untouched on a non-force run"
    );
    // The skipped pi copy is not reported as installed; claude is.
    let v: Value = serde_json::from_slice(&out.stdout).expect("json");
    let agents: Vec<&str> = v["data"]["installed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["agent"].as_str().unwrap())
        .collect();
    assert!(
        agents.contains(&"claude"),
        "claude not in installed: {agents:?}"
    );
    assert!(
        !agents.contains(&"pi"),
        "an untouched pi mirror must not be reported as installed: {agents:?}"
    );
}

#[test]
fn skill_install_self_repairs_missing_pi_mirror_without_force() {
    // The inverse partial state: claude current, pi missing. A plain install
    // refuses the equal-version claude copy (existing contract), so use
    // --force — the refresh must (re)create the pi mirror.
    let home = mk_home();
    assert!(bin(&home)
        .args(["skill", "install", "stint-start"])
        .output()
        .expect("spawn")
        .status
        .success());
    let pi = home.path().join(".pi/agent/skills/stint-start/SKILL.md");
    std::fs::remove_dir_all(home.path().join(".pi/agent/skills/stint-start")).unwrap();
    assert!(!pi.exists());
    assert!(bin(&home)
        .args(["skill", "install", "stint-start", "--force"])
        .output()
        .expect("spawn")
        .status
        .success());
    assert!(pi.exists(), "pi mirror was not recreated by --force");
}

#[test]
fn skill_install_force_refreshes_stale_pi_mirror() {
    // A stale/divergent pi copy is refreshed only under --force.
    let home = mk_home();
    assert!(bin(&home)
        .args(["skill", "install", "stint-start"])
        .output()
        .expect("spawn")
        .status
        .success());
    let pi = home.path().join(".pi/agent/skills/stint-start/SKILL.md");
    std::fs::write(
        &pi,
        "---\nname: stint-start\ncli_version: \"0.0.0\"\n---\nstale\n",
    )
    .unwrap();

    // Non-force: pi is left alone (skipped) and a differ-warning is emitted;
    // but claude is equal-version so the plain plan refuses. Force the whole
    // redeploy and confirm pi is brought back in sync with the binary.
    let out = bin(&home)
        .args([
            "skill",
            "install",
            "stint-start",
            "--force",
            "--output",
            "json",
        ])
        .output()
        .expect("spawn");
    assert!(out.status.success(), "force redeploy failed: {out:?}");
    let after = std::fs::read_to_string(&pi).unwrap();
    assert!(
        after.contains(&format!("cli_version: \"{}\"", env!("CARGO_PKG_VERSION"))),
        "pi mirror was not refreshed to the binary version under --force"
    );
}

#[test]
fn skill_install_does_not_clobber_divergent_pi_mirror_without_force() {
    // A pre-existing pi file that differs (e.g. user-authored, older) must be
    // LEFT IN PLACE on a plain run — never silently clobbered just because it
    // looks older. pi has no provenance marker, so we cannot prove ownership.
    let home = mk_home();
    let pi_dir = home.path().join(".pi/agent/skills/stint-start");
    std::fs::create_dir_all(&pi_dir).unwrap();
    let pi = pi_dir.join("SKILL.md");
    let user_content = "---\nname: stint-start\ncli_version: \"0.0.0\"\n---\nMINE — do not touch\n";
    std::fs::write(&pi, user_content).unwrap();

    // Fresh claude (no prior install) + differing pi present, no --force.
    let out = bin(&home)
        .args(["skill", "install", "stint-start", "--output", "json"])
        .output()
        .expect("spawn");
    assert!(out.status.success(), "install must succeed: {out:?}");
    assert_eq!(
        std::fs::read_to_string(&pi).unwrap(),
        user_content,
        "divergent pi file must not be clobbered without --force"
    );
    // The differ is surfaced as a warning, not silently ignored.
    let v: Value = serde_json::from_slice(&out.stdout).expect("json");
    let warnings = v["warnings"].as_array().expect("warnings");
    assert!(
        warnings
            .iter()
            .any(|w| w.as_str().unwrap().contains("pi_mirror_skipped")),
        "expected a pi_mirror_skipped warning; got {warnings:?}"
    );
}

#[test]
fn skill_install_agent_all_also_dual_homes_into_pi() {
    // `--agent all` installs claude + codex, and still mirrors into pi.
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
    assert!(home
        .path()
        .join(".pi/agent/skills/octl-spawn-spinoff/SKILL.md")
        .exists());
}

#[test]
fn skill_install_agent_codex_does_not_dual_home_into_pi() {
    // pi mirrors the claude corpus; a codex-only install must not touch it.
    let home = mk_home();
    let out = bin(&home)
        .args([
            "skill",
            "install",
            "octl-run-overview",
            "--agent",
            "codex",
            "--output",
            "json",
        ])
        .output()
        .expect("spawn");
    assert!(out.status.success(), "codex install failed: {out:?}");
    assert!(
        !home.path().join(".pi/agent/skills").exists(),
        "codex-only install must not create the pi skill dir"
    );
}

#[test]
fn skill_install_dest_does_not_dual_home_into_pi() {
    // A custom `--dest` is caller-managed; the pi mirror is skipped.
    let home = mk_home();
    let dest = home.path().join("custom/SKILL.md");
    let out = bin(&home)
        .args(["skill", "install", "octl-run-overview", "--dest"])
        .arg(&dest)
        .args(["--output", "json"])
        .output()
        .expect("spawn");
    assert!(out.status.success(), "dest install failed: {out:?}");
    assert!(dest.exists(), "dest SKILL.md not on disk");
    assert!(
        !home.path().join(".pi/agent/skills").exists(),
        "--dest install must not create the pi skill dir"
    );
}

#[test]
fn skill_install_all_prunes_managed_orphan() {
    // A managed skill dir that is NOT in the catalog (valid marker naming
    // itself) must be pruned by the full-catalog --force install.
    let home = mk_home();
    let orphan = home.path().join(".claude/skills/gone-skill");
    std::fs::create_dir_all(&orphan).unwrap();
    std::fs::write(orphan.join("SKILL.md"), "---\nname: gone-skill\n---\n").unwrap();
    write_marker(&orphan, "gone-skill");

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
    write_marker(&orphan, "gone-skill");

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
fn skill_install_all_without_force_does_not_prune() {
    // Prune is gated on --force: a plain full-catalog install must never
    // delete a directory as a side effect, even a valid managed orphan.
    let home = mk_home();
    let orphan = home.path().join(".claude/skills/gone-skill");
    std::fs::create_dir_all(&orphan).unwrap();
    std::fs::write(orphan.join("SKILL.md"), "---\nname: gone-skill\n---\n").unwrap();
    write_marker(&orphan, "gone-skill");

    let out = bin(&home)
        .args(["skill", "install", "--output", "json"])
        .output()
        .expect("spawn");
    assert!(out.status.success(), "install-all failed: {out:?}");
    assert!(orphan.exists(), "prune ran without --force");
    let v: Value = serde_json::from_slice(&out.stdout).expect("json");
    assert!(v["data"]["pruned"].as_array().expect("pruned").is_empty());
}

#[test]
fn skill_install_all_spares_copied_and_renamed_managed_skill() {
    // The core data-loss guard: a user copies a managed skill (marker and
    // all) to a new name and edits it. The marker still names the ORIGINAL
    // skill, so it must not match the new dir name — the copy is spared.
    let home = mk_home();
    let copy = home.path().join(".claude/skills/my-worktree");
    std::fs::create_dir_all(&copy).unwrap();
    std::fs::write(copy.join("SKILL.md"), "---\nname: my-worktree\n---\nmine\n").unwrap();
    // Marker copied verbatim from `worktree` — names `worktree`, not the
    // new dir `my-worktree`.
    write_marker(&copy, "worktree");

    let out = bin(&home)
        .args(["skill", "install", "--force", "--output", "json"])
        .output()
        .expect("spawn");
    assert!(out.status.success(), "install-all failed: {out:?}");
    assert!(
        copy.join("SKILL.md").exists(),
        "a copied-and-renamed managed skill was deleted — name-binding guard failed"
    );
    let v: Value = serde_json::from_slice(&out.stdout).expect("json");
    assert!(v["data"]["pruned"].as_array().expect("pruned").is_empty());
}

#[cfg(unix)]
#[test]
fn skill_install_all_does_not_follow_symlinked_orphan() {
    // A symlink under ~/.claude/skills pointing at an outside directory
    // must never be traversed by remove_dir_all, even if the target looks
    // managed. The symlink entry is skipped; the target is untouched.
    let home = mk_home();
    let outside = home.path().join("precious");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("SKILL.md"), "important user data\n").unwrap();
    write_marker(&outside, "evil");

    let skills = home.path().join(".claude/skills");
    std::fs::create_dir_all(&skills).unwrap();
    std::os::unix::fs::symlink(&outside, skills.join("evil")).unwrap();

    let out = bin(&home)
        .args(["skill", "install", "--force", "--output", "json"])
        .output()
        .expect("spawn");
    assert!(out.status.success(), "install-all failed: {out:?}");
    assert!(
        outside.join("SKILL.md").exists(),
        "remove_dir_all followed a symlink and deleted an outside directory"
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
fn skill_install_stint_start_codex_writes_companion_to_shared_and_rewrites_link() {
    // The codex layout is a flat prompts dir where every top-level `.md`
    // surfaces as a slash-command, so the companion is installed into a
    // `_shared/` subdir (never a bogus top-level prompt) and the skill body's
    // sibling link `](AGENTS-EXECUTION-DAG.md)` is rewritten to point there.
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

    let prompt = home.path().join(".codex/prompts/stint-start.md");
    assert!(prompt.exists(), "flat codex prompt not installed");
    // Companion lands in `_shared/`, NOT as a top-level flat prompt.
    assert!(
        home.path()
            .join(".codex/prompts/_shared/AGENTS-EXECUTION-DAG.md")
            .exists(),
        "companion not installed into the codex _shared/ subdir"
    );
    assert!(
        !home
            .path()
            .join(".codex/prompts/AGENTS-EXECUTION-DAG.md")
            .exists(),
        "companion must not leak into the flat codex prompts dir as a bogus prompt"
    );

    // The in-body link resolves to the shared copy, and the claude-layout
    // sibling form is gone.
    let body = std::fs::read_to_string(&prompt).unwrap();
    assert!(
        body.contains("](_shared/AGENTS-EXECUTION-DAG.md)"),
        "codex body link was not rewritten to the _shared/ target"
    );
    assert!(
        !body.contains("](AGENTS-EXECUTION-DAG.md)"),
        "codex body still carries the un-rewritten claude sibling link"
    );

    let v: Value = serde_json::from_slice(&out.stdout).expect("json");
    let installed = v["data"]["installed"].as_array().expect("installed array");
    let paths: Vec<&str> = installed
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert!(
        paths
            .iter()
            .any(|p| p.ends_with("_shared/AGENTS-EXECUTION-DAG.md")),
        "companion not reported in codex install payload: {paths:?}"
    );
}

#[test]
fn skill_install_stint_handoff_codex_rewrites_cross_skill_link() {
    // `stint-handoff` links to the DAG cross-skill via
    // `](../stint-start/AGENTS-EXECUTION-DAG.md)`. On codex that collapses to
    // the same shared `_shared/` target so the reference still resolves in the
    // flat layout.
    let home = mk_home();
    let out = bin(&home)
        .args(["skill", "install", "stint-handoff", "--agent", "codex"])
        .output()
        .expect("spawn");
    assert!(out.status.success(), "codex install failed: {out:?}");

    let body =
        std::fs::read_to_string(home.path().join(".codex/prompts/stint-handoff.md")).unwrap();
    assert!(
        body.contains("](_shared/AGENTS-EXECUTION-DAG.md)"),
        "cross-skill codex link was not rewritten to the _shared/ target"
    );
    assert!(
        !body.contains("](../stint-start/AGENTS-EXECUTION-DAG.md)"),
        "codex body still carries the un-rewritten cross-skill link"
    );
}

#[test]
fn skill_install_all_codex_writes_shared_companion_once() {
    // A full codex catalog install must place the companion in `_shared/`
    // exactly once (owned by stint-start) with no duplicate-destination
    // collision, and leave the claude sibling body link byte-for-byte in the
    // claude install untouched.
    let home = mk_home();
    let out = bin(&home)
        .args(["skill", "install", "--agent", "all"])
        .output()
        .expect("spawn");
    assert!(out.status.success(), "install --agent all failed: {out:?}");

    // codex: shared companion present, no flat leak.
    assert!(
        home.path()
            .join(".codex/prompts/_shared/AGENTS-EXECUTION-DAG.md")
            .exists(),
        "codex shared companion missing after --agent all"
    );
    // claude: unchanged sibling layout, body link NOT rewritten.
    let claude_body =
        std::fs::read_to_string(home.path().join(".claude/skills/stint-start/SKILL.md")).unwrap();
    assert!(
        claude_body.contains("](AGENTS-EXECUTION-DAG.md)"),
        "claude body sibling link must be preserved verbatim"
    );
    assert!(
        home.path()
            .join(".claude/skills/stint-start/AGENTS-EXECUTION-DAG.md")
            .exists(),
        "claude sibling companion missing after --agent all"
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
    // Make BOTH claude on-disk files look older than the binary so the
    // whole plan qualifies for the drift-upgrade (no-force) path. The pi
    // mirror (also written by the first install) does NOT need aging: a
    // derived pi mirror never gates the plan — preflight leaves an existing
    // pi copy in place on a non-force run (see F1 / `pi_mirror_skipped`).
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
