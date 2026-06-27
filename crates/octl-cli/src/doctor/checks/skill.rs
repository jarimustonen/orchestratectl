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
            // HOME unset: cannot locate the install. Report once, neutrally.
            out.push(CheckResult::warn(
                id,
                format!("cannot locate install for {name} (HOME unset)"),
                "set HOME so the default skill path resolves",
            ));
            continue;
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
    }
    out
}

/// Semver-correct comparison; `None` if either side does not parse (the
/// caller routes that to the "unparseable" arm rather than inventing an
/// ordering).
fn compare(a: &str, b: &str) -> Option<Ordering> {
    let av = semver::Version::parse(a).ok()?;
    let bv = semver::Version::parse(b).ok()?;
    Some(av.cmp(&bv))
}
