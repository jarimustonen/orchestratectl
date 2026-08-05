//! `skill.sync.<name>` — every bundled companion skill's on-disk
//! `cli_version` matches the running binary (AGENTS-AI-FIRST-CLI §17/§18
//! "skill sync").
//!
//! Drift matters because the skill *is* the agent's operating manual for
//! the binary: an out-of-date skill describes commands that may have
//! moved. Classification:
//!
//! - on-disk **older** than the binary → `WARN` + safe `--fix`
//!   (`skill install <name> --force`).
//! - on-disk **newer** than the binary → `WARN`, suggest a binary
//!   upgrade (no autonomous fix — the agent installed ahead of the
//!   binary on purpose, perhaps).
//! - **not installed** → `WARN` (info-as-warn), suggest install; not an
//!   autonomous fix (the §18 safe subset is drift-only).
//! - on-disk version **unreadable / unparseable** → `WARN`, suggest a
//!   forced re-install (no autonomous fix; we refuse to guess).
//! - **in sync** → `OK`.

use std::cmp::Ordering;

use crate::doctor::check::{CheckResult, FixAction};
use crate::skill;

use super::Ctx;

pub fn check(_ctx: &Ctx) -> Vec<CheckResult> {
    let binary = skill::binary_cli_version();
    let mut out = Vec::new();

    for name in skill::bundled_skill_names() {
        let id = format!("skill.sync.{name}");
        let suggest_install = format!("orchestratectl skill install {name} --force");

        let Some(path) = skill::claude_default_path(name) else {
            // HOME unset: cannot locate any install. config.home already
            // reports the root cause as a FAIL — emit a single consolidated
            // skill.sync WARN rather than one noisy duplicate per skill.
            out.push(CheckResult::warn(
                "skill.sync",
                "cannot locate skill installs (HOME unset)",
                "set HOME so the default skill path resolves",
            ));
            break;
        };

        if !path.exists() {
            out.push(CheckResult::warn(
                id,
                format!("skill '{name}' is not installed at {}", path.display()),
                suggest_install,
            ));
            continue;
        }

        let on_disk = skill::read_on_disk_cli_version(&path);
        match on_disk
            .as_deref()
            .and_then(|v| compare(v, binary).map(|o| (v, o)))
        {
            Some((_, Ordering::Equal)) => {
                out.push(CheckResult::ok(
                    id,
                    format!("skill '{name}' in sync at cli_version {binary}"),
                ));
            }
            Some((v, Ordering::Less)) => {
                out.push(
                    CheckResult::warn(
                        id,
                        format!("skill '{name}' is cli_version {v}, binary is {binary}"),
                        suggest_install,
                    )
                    .with_safe_fix(FixAction::InstallSkill(name.to_string())),
                );
            }
            Some((v, Ordering::Greater)) => {
                out.push(CheckResult::warn(
                    id,
                    format!(
                        "skill '{name}' on disk is cli_version {v}, newer than binary {binary}"
                    ),
                    "upgrade the orchestratectl binary to match the installed skill",
                ));
            }
            None => {
                out.push(CheckResult::warn(
                    id,
                    format!(
                        "skill '{name}' has an unreadable/unparseable cli_version at {}",
                        path.display()
                    ),
                    suggest_install,
                ));
            }
        }

        // Companion resource files shipped alongside this skill's SKILL.md
        // (e.g. `stint-start/AGENTS-EXECUTION-DAG.md`). Each installs as a
        // sibling of SKILL.md; a missing, stale, or user-edited companion
        // leaves the skill's in-body link dangling while SKILL.md itself
        // still looks in sync, so audit each one under its own id. Only
        // reached when SKILL.md exists (the not-installed arm above
        // `continue`s — the skill-not-installed WARN already covers the
        // companions a re-install would restore).
        if let Some(skill_dir) = path.parent() {
            for companion in skill::companion_sources(name) {
                let companion_path = skill_dir.join(companion.filename);
                out.push(check_companion(name, &companion, &companion_path, binary));
            }
        }
    }

    // `skill.orphan.<name>` — a claude-layout skill directory that
    // orchestratectl installed (carries the provenance marker) but the
    // running binary no longer ships. This is a renamed/removed bundled
    // skill left stranded as a stale slash-command. `skill install`
    // auto-prunes these on its next full-catalog run, so the fix is a
    // forced re-install; we surface it as a WARN rather than fixing
    // autonomously (deletion stays with the explicit install path).
    for (name, dir) in skill::managed_orphans() {
        out.push(CheckResult::warn(
            format!("skill.orphan.{name}"),
            format!(
                "skill '{name}' at {} is orchestratectl-managed but no longer in the catalog (de-registered)",
                dir.display()
            ),
            "orchestratectl skill install --force",
        ));
    }

    out
}

/// Audit one companion resource against the binary's bundled copy. Content
/// identity is the primary in-sync signal — a freshly installed companion
/// is byte-identical to the embedded source (both rendered through the same
/// `{{CLI_VERSION}}` substitution), so any byte difference means it is
/// stale, ahead of the binary, or edited. When it differs, classify by the
/// declared `cli_version` using the same semver drift model as SKILL.md so
/// the message names which way it drifted. The id embeds the filename so the
/// offending companion is unambiguous.
///
/// Note: only companions the *current* binary bundles are audited (the
/// forward direction). A companion a prior binary installed but this one no
/// longer ships is not detected here — that orphan-companion sweep needs a
/// managed-file manifest + `skill install` prune support; see the
/// `doctor-orphan-companion-files` follow-up.
fn check_companion(
    skill_name: &str,
    companion: &skill::CompanionSource,
    path: &std::path::Path,
    binary: &str,
) -> CheckResult {
    let filename = companion.filename;
    let id = format!("skill.sync.{skill_name}.{filename}");
    let suggest_install = format!("orchestratectl skill install {skill_name} --force");

    // One read serves both existence and content: `read_to_string` returns
    // `NotFound` for a missing companion (never installed) and a distinct
    // error otherwise, so we classify precisely and avoid the
    // exists()-then-read TOCTOU (and its symlink following).
    let on_disk = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return CheckResult::warn(
                id,
                format!(
                    "companion '{filename}' for skill '{skill_name}' is not installed at {}",
                    path.display()
                ),
                suggest_install,
            );
        }
        Err(e) => {
            return CheckResult::warn(
                id,
                format!(
                    "companion '{filename}' for skill '{skill_name}' is unreadable at {}: {e}",
                    path.display()
                ),
                suggest_install,
            );
        }
    };

    // Content identity is the primary in-sync signal: a freshly installed
    // companion is byte-identical to the embedded source. We deliberately
    // report what was checked (a content match) rather than asserting a
    // parsed version, since a companion need not carry `cli_version`
    // frontmatter.
    if on_disk == companion.bundled_body {
        return CheckResult::ok(
            id,
            format!(
                "companion '{filename}' for skill '{skill_name}' matches the bundled content for binary {binary}"
            ),
        );
    }

    // Content differs — classify by the declared `cli_version` (metadata in
    // the differing file, so the message states evidence, not provenance).
    let disk_version = skill::cli_version_of(&on_disk);
    match disk_version
        .as_deref()
        .and_then(|v| compare(v, binary).map(|o| (v, o)))
    {
        Some((v, Ordering::Less)) => CheckResult::warn(
            id,
            format!(
                "companion '{filename}' for skill '{skill_name}' is cli_version {v}, binary is {binary}"
            ),
            suggest_install,
        )
        .with_safe_fix(FixAction::InstallSkill(skill_name.to_string())),
        Some((v, Ordering::Greater)) => CheckResult::warn(
            id,
            format!(
                "companion '{filename}' for skill '{skill_name}' differs from the bundled copy and declares cli_version {v}, newer than binary {binary}"
            ),
            "upgrade the orchestratectl binary, or reinstall with --force to restore the bundled companion",
        ),
        Some((_, Ordering::Equal)) => CheckResult::warn(
            id,
            format!(
                "companion '{filename}' for skill '{skill_name}' differs from the bundled copy while its cli_version matches binary {binary} (possible local edits)"
            ),
            suggest_install,
        ),
        None => CheckResult::warn(
            id,
            format!(
                "companion '{filename}' for skill '{skill_name}' differs from the bundled copy and declares no parseable cli_version at {}",
                path.display()
            ),
            suggest_install,
        ),
    }
}

/// Semver-correct comparison; `None` if either side does not parse (the
/// caller routes that to the "unparseable" arm rather than inventing an
/// ordering).
fn compare(a: &str, b: &str) -> Option<Ordering> {
    let av = semver::Version::parse(a).ok()?;
    let bv = semver::Version::parse(b).ok()?;
    Some(av.cmp(&bv))
}
