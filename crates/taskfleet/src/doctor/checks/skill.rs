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
        let suggest_install = format!("taskfleet skill install {name} --force");

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
                    "upgrade the taskfleet binary to match the installed skill",
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
        // (for example, a skill's shared reference file). Each installs as a
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

            // `skill.orphan.<name>.<file>` — a companion the skill's
            // provenance marker records as taskfleet-managed but the
            // current binary no longer bundles: installed by a prior binary,
            // dropped by this one, lingering as a stale sibling. Distinct from
            // the `skill.sync.<name>.<file>` cases above (those audit
            // companions this binary DOES ship). The fix is a forced
            // re-install, whose prune loop removes the orphan file; we surface
            // it as a WARN rather than fixing autonomously (deletion stays with
            // the explicit install path, symmetric with `skill.orphan.<name>`).
            for filename in skill::orphan_companions(name, skill_dir) {
                let orphan_path = skill_dir.join(&filename);
                out.push(CheckResult::warn(
                    format!("skill.orphan.{name}.{filename}"),
                    format!(
                        "companion '{filename}' for skill '{name}' at {} is taskfleet-managed but the current binary no longer bundles it (de-registered)",
                        orphan_path.display()
                    ),
                    format!("taskfleet skill install {name} --force"),
                ));
            }
        }
    }

    // `skill.orphan.<name>` — a claude-layout skill directory that
    // taskfleet installed (carries the provenance marker) but the
    // running binary no longer ships. This is a renamed/removed bundled
    // skill left stranded as a stale slash-command. `skill install`
    // auto-prunes these on its next full-catalog run, so the fix is a
    // forced re-install; we surface it as a WARN rather than fixing
    // autonomously (deletion stays with the explicit install path).
    for (name, dir) in skill::managed_orphans() {
        out.push(CheckResult::warn(
            format!("skill.orphan.{name}"),
            format!(
                "skill '{name}' at {} is taskfleet-managed but no longer in the catalog (de-registered)",
                dir.display()
            ),
            "taskfleet skill install --force",
        ));
    }

    check_codex(binary, &mut out);
    check_pi(binary, &mut out);

    out
}

/// pi.dev mirror coverage — the `skill.sync.<name>.pi` / `skill.orphan.<name>.pi`
/// mirror of the claude checks above, keyed to the pi mirror paths
/// (`~/.pi/agent/skills/<name>/SKILL.md`).
///
/// GATED on the out-of-band pi provenance record: the pi dir carries no in-dir
/// `.taskfleet-managed` marker (the `pidev-dual-home-skills` contract
/// forbids one), so the record `<root>/state/pi-installed-skills.json` is the
/// SOLE source of truth for which pi mirrors taskfleet wrote. An empty
/// record (a host that never dual-homed into pi, or `HOME` unset) yields no pi
/// checks and keeps a pi-less tree 0-warn — and, crucially, a user's own
/// hand-authored pi skill is never recorded, so it is never flagged.
///
/// ALL pi arms are advisory — NO autonomous `FixAction::InstallSkill`. Unlike
/// the claude check, the pi OLDER-drift case does NOT attach the fix: the
/// applier runs `skill install <name> --force`, which dual-homes and would
/// force-overwrite the CLAUDE copy too, so autofixing pi drift could silently
/// downgrade a deliberately newer/edited claude copy (whose own check refused an
/// autonomous fix). Symmetric with the codex checks, which omit the fix for the
/// same cross-target reason. Deletion + binary upgrades likewise belong to the
/// explicit install path (see `assessment-pi-lifecycle`, review finding C).
fn check_pi(binary: &str, out: &mut Vec<CheckResult>) {
    let managed = skill::pi_managed_skills();
    if managed.is_empty() {
        return;
    }

    let catalog: std::collections::HashSet<&str> =
        skill::bundled_skill_names().into_iter().collect();

    for m in &managed {
        let name = m.name.as_str();
        // Record-sourced names are validated as a single normal path component
        // before being joined into a filesystem path — a corrupt/hand-edited key
        // like `"../../x"` is skipped, never resolved (review finding E; same
        // guard the prune path applies). Unlike the prune path (which stays
        // silent), doctor SURFACES the corrupt entry — it is the diagnostic
        // command, so a record no one can act on must be visible (review finding
        // F7). The name is never joined into a path.
        if !skill::is_simple_skill_name(name) {
            out.push(CheckResult::warn(
                "skill.provenance.pi",
                format!(
                    "pi provenance record contains an invalid skill name {name:?}; the record is corrupt and should be repaired"
                ),
                "back up and remove the pi provenance record (state/pi-installed-skills.json) to re-initialise",
            ));
            continue;
        }
        let Some(path) = skill::pi_default_path(name) else {
            continue;
        };

        // Still-registered check is case-insensitive as well as exact, symmetric
        // with `managed_orphan_dirs` / the pi prune: on a case-insensitive
        // filesystem (APFS) a corrupt record key that is a case variant of a
        // registered skill must NOT be flagged as a de-registered orphan (review
        // finding F5).
        let registered_now =
            catalog.contains(name) || catalog.iter().any(|c| c.eq_ignore_ascii_case(name));
        if !registered_now {
            // Recorded but de-registered: an orphan, but only if the mirror is
            // still on disk (a record whose file was already removed is nothing
            // to flag). `symlink_metadata` so a planted symlink is not followed.
            if std::fs::symlink_metadata(&path).is_ok() {
                out.push(CheckResult::warn(
                    format!("skill.orphan.{name}.pi"),
                    format!(
                        "pi skill '{name}' at {} is taskfleet-managed but no longer in the catalog (de-registered)",
                        path.display()
                    ),
                    "taskfleet skill install --force",
                ));
            }
            continue;
        }

        let id = format!("skill.sync.{name}.pi");
        let suggest = format!("taskfleet skill install {name} --force");

        if !path.exists() {
            out.push(CheckResult::warn(
                id,
                format!("pi skill '{name}' is not installed at {}", path.display()),
                suggest,
            ));
            continue;
        }

        // Companion resources mirrored beside the pi `SKILL.md` (pi uses a
        // per-skill dir like claude, so companions are plain siblings). Audited
        // here — only reached once the pi `SKILL.md` exists (the not-installed
        // arm above `continue`s; a reinstall restores the companions with it),
        // symmetric with the claude/codex companion checks. `skill.sync.<name>.
        // pi.<file>` (forward drift vs the bundled body) then `skill.orphan.
        // <name>.pi.<file>` (a companion the record tracks that the binary no
        // longer bundles).
        if let Some(pi_dir) = path.parent() {
            // Bind the bundled companion set once (each `companion_sources` call
            // allocates a fresh Vec). `companion.filename` is a compile-time
            // `EmbeddedResource` constant, so — unlike the record-sourced orphan
            // filenames below — it needs no path-component validation before the
            // join.
            let sources = skill::companion_sources(name);
            let bundled: std::collections::HashSet<&str> =
                sources.iter().map(|c| c.filename).collect();
            for companion in &sources {
                let companion_path = pi_dir.join(companion.filename);
                out.push(check_pi_companion(name, companion, &companion_path, binary));
            }
            for filename in &m.companions {
                if bundled.contains(filename.as_str()) {
                    continue; // still bundled → audited by the forward check above
                }
                // Record-sourced filename → guard as a single path component
                // before joining (same rigor as the skill-name guard above).
                // Surface a corrupt entry rather than hiding it (review finding
                // F7); the invalid name is never joined into a path.
                if !skill::is_simple_skill_name(filename) {
                    out.push(CheckResult::warn(
                        "skill.provenance.pi",
                        format!(
                            "pi provenance record lists an invalid companion filename {filename:?} for skill '{name}'; the record is corrupt and should be repaired"
                        ),
                        "back up and remove the pi provenance record (state/pi-installed-skills.json) to re-initialise",
                    ));
                    continue;
                }
                let orphan_path = pi_dir.join(filename);
                if std::fs::symlink_metadata(&orphan_path).is_ok() {
                    out.push(CheckResult::warn(
                        format!("skill.orphan.{name}.pi.{filename}"),
                        format!(
                            "pi companion '{filename}' for skill '{name}' at {} is taskfleet-managed but the current binary no longer bundles it (de-registered)",
                            orphan_path.display()
                        ),
                        format!("taskfleet skill install {name} --force"),
                    ));
                }
            }
        }

        match skill::read_on_disk_cli_version(&path)
            .as_deref()
            .and_then(|v| compare(v, binary).map(|o| (v, o)))
        {
            Some((_, Ordering::Equal)) => {
                // Version-in-sync: use the recorded content hash to catch a
                // same-version local edit (the pi mirror is byte-identical to
                // what we wrote, so any divergence is an edit). When the record
                // tracks NO body hash (`None` — only companions were recorded,
                // the body write was skipped), we have nothing to compare
                // against, so we report version-in-sync rather than fabricating a
                // "differs" claim about a body we never recorded writing.
                match (skill::file_sha256(&path), m.sha256.as_deref()) {
                    (Some(h), Some(recorded)) if h == recorded => out.push(CheckResult::ok(
                        id,
                        format!("pi skill '{name}' in sync at cli_version {binary}"),
                    )),
                    (Some(_), Some(_)) => out.push(CheckResult::warn(
                        id,
                        format!(
                            "pi skill '{name}' differs from the copy taskfleet wrote while its cli_version matches binary {binary} (possible local edits)"
                        ),
                        suggest,
                    )),
                    (Some(_), None) => out.push(CheckResult::ok(
                        id,
                        format!("pi skill '{name}' in sync at cli_version {binary}"),
                    )),
                    (None, _) => out.push(CheckResult::warn(
                        id,
                        format!("pi skill '{name}' is unreadable at {}", path.display()),
                        suggest,
                    )),
                }
            }
            Some((v, Ordering::Less)) => {
                // Advisory only — NO autonomous `FixAction::InstallSkill`. That
                // applier runs `skill install <name> --force`, which dual-homes
                // and would force-overwrite the CLAUDE copy too: if that copy is
                // deliberately newer/edited (its own check refuses an autonomous
                // fix), autofixing pi drift here would silently downgrade it.
                // Symmetric with the codex checks, which omit the fix for the
                // same cross-target reason (review C, `assessment-pi-lifecycle`).
                out.push(CheckResult::warn(
                    id,
                    format!("pi skill '{name}' is cli_version {v}, binary is {binary}"),
                    suggest,
                ));
            }
            Some((v, Ordering::Greater)) => {
                out.push(CheckResult::warn(
                    id,
                    format!(
                        "pi skill '{name}' on disk is cli_version {v}, newer than binary {binary}"
                    ),
                    "upgrade the taskfleet binary to match the installed skill",
                ));
            }
            None => {
                out.push(CheckResult::warn(
                    id,
                    format!(
                        "pi skill '{name}' has an unreadable/unparseable cli_version at {}",
                        path.display()
                    ),
                    suggest,
                ));
            }
        }
    }
}

/// Audit one pi companion sibling (`~/.pi/agent/skills/<name>/<file>`) against
/// the binary's bundled copy. Content identity is the in-sync signal (a freshly
/// installed companion is byte-identical to the embedded source); on a
/// difference, classify by the declared `cli_version` with the same drift model
/// as the claude/codex companion checks. Advisory only — NO autonomous
/// `FixAction`, for the same cross-target reason as the pi `SKILL.md` arm: the
/// applier runs `skill install <name> --force`, which dual-homes and would
/// force-overwrite the claude copy too.
fn check_pi_companion(
    skill_name: &str,
    companion: &skill::CompanionSource,
    path: &std::path::Path,
    binary: &str,
) -> CheckResult {
    let filename = companion.filename;
    let id = format!("skill.sync.{skill_name}.pi.{filename}");
    let suggest = format!("taskfleet skill install {skill_name} --force");

    let on_disk = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return CheckResult::warn(
                id,
                format!(
                    "pi companion '{filename}' for skill '{skill_name}' is not installed at {}",
                    path.display()
                ),
                suggest,
            );
        }
        Err(e) => {
            return CheckResult::warn(
                id,
                format!(
                    "pi companion '{filename}' for skill '{skill_name}' is unreadable at {}: {e}",
                    path.display()
                ),
                suggest,
            );
        }
    };

    if on_disk == companion.bundled_body {
        return CheckResult::ok(
            id,
            format!(
                "pi companion '{filename}' for skill '{skill_name}' matches the bundled content for binary {binary}"
            ),
        );
    }

    match skill::cli_version_of(&on_disk)
        .as_deref()
        .and_then(|v| compare(v, binary).map(|o| (v, o)))
    {
        Some((v, Ordering::Less)) => CheckResult::warn(
            id,
            format!(
                "pi companion '{filename}' for skill '{skill_name}' is cli_version {v}, binary is {binary}"
            ),
            suggest,
        ),
        Some((v, Ordering::Greater)) => CheckResult::warn(
            id,
            format!(
                "pi companion '{filename}' for skill '{skill_name}' differs from the bundled copy and declares cli_version {v}, newer than binary {binary}"
            ),
            "upgrade the taskfleet binary to match the installed skill",
        ),
        Some((_, Ordering::Equal)) => CheckResult::warn(
            id,
            format!(
                "pi companion '{filename}' for skill '{skill_name}' differs from the bundled copy while its cli_version matches binary {binary} (possible local edits)"
            ),
            suggest,
        ),
        None => CheckResult::warn(
            id,
            format!(
                "pi companion '{filename}' for skill '{skill_name}' differs from the bundled copy and declares no parseable cli_version at {}",
                path.display()
            ),
            suggest,
        ),
    }
}

/// Codex flat-layout coverage — the `skill.sync.codex.*` /
/// `skill.orphan.codex.*` mirror of the claude checks above, keyed to the
/// codex paths (`~/.codex/prompts/<name>.md` and the shared companions in
/// `~/.codex/prompts/_shared/<file>`).
///
/// The whole section is GATED on taskfleet actually managing codex on
/// this host: the shared provenance marker records which prompts +
/// companions we installed, so an absent marker (e.g. a claude-only
/// install, where codex is a secondary export the user never targeted)
/// yields no codex checks and keeps a claude-primary tree 0-warn. The
/// marker's recorded set is also the source of truth for what "should" be
/// present, so a bundled skill the user simply chose not to install to
/// codex is never flagged.
///
/// Codex drift carries NO autonomous `--fix`: the `FixAction::InstallSkill`
/// applier re-runs `skill install <name> --force`, which targets the claude
/// (+ pi) layout, not codex. A codex re-install needs `--agent codex`/`all`,
/// so we surface the suggestion and leave the deletion/reinstall to the
/// explicit install path (symmetric with the claude orphan checks, which are
/// advisory too).
fn check_codex(binary: &str, out: &mut Vec<CheckResult>) {
    let managed_prompts = skill::codex_managed_prompts();
    let managed_companions = skill::codex_managed_companions();
    if managed_prompts.is_empty() && managed_companions.is_empty() {
        return;
    }

    let catalog: std::collections::HashSet<&str> =
        skill::bundled_skill_names().into_iter().collect();

    // Codex skill sync (recorded ∩ catalog) + orphan (recorded ∖ catalog).
    for name in &managed_prompts {
        let Some(path) = skill::codex_default_path(name) else {
            continue;
        };
        if catalog.contains(name.as_str()) {
            let id = format!("skill.sync.codex.{name}");
            let suggest = format!("taskfleet skill install {name} --agent codex --force");
            if !path.exists() {
                out.push(CheckResult::warn(
                    id,
                    format!(
                        "codex skill '{name}' is not installed at {}",
                        path.display()
                    ),
                    suggest,
                ));
                continue;
            }
            match skill::read_on_disk_cli_version(&path)
                .as_deref()
                .and_then(|v| compare(v, binary).map(|o| (v, o)))
            {
                Some((_, Ordering::Equal)) => out.push(CheckResult::ok(
                    id,
                    format!("codex skill '{name}' in sync at cli_version {binary}"),
                )),
                Some((v, Ordering::Less)) => out.push(CheckResult::warn(
                    id,
                    format!("codex skill '{name}' is cli_version {v}, binary is {binary}"),
                    suggest,
                )),
                Some((v, Ordering::Greater)) => out.push(CheckResult::warn(
                    id,
                    format!(
                        "codex skill '{name}' on disk is cli_version {v}, newer than binary {binary}"
                    ),
                    "upgrade the taskfleet binary to match the installed skill",
                )),
                None => out.push(CheckResult::warn(
                    id,
                    format!(
                        "codex skill '{name}' has an unreadable/unparseable cli_version at {}",
                        path.display()
                    ),
                    suggest,
                )),
            }
        } else {
            // Recorded but de-registered: an orphan, but only if the flat
            // prompt file is still on disk (a marker record whose file was
            // already removed is nothing to flag).
            if std::fs::symlink_metadata(&path).is_ok() {
                out.push(CheckResult::warn(
                    format!("skill.orphan.codex.{name}"),
                    format!(
                        "codex skill '{name}' at {} is taskfleet-managed but no longer in the catalog (de-registered)",
                        path.display()
                    ),
                    "taskfleet skill install --agent codex --force",
                ));
            }
        }
    }

    // Codex companion sync + orphan, resolved against the shared `_shared/`
    // dir. Companions are byte-identical to the bundled source (only skill
    // bodies get the codex link rewrite), so the same content-identity check
    // the claude companions use applies.
    let Some(shared_root) = skill::codex_shared_root() else {
        return;
    };
    let bundled: std::collections::HashSet<&str> = skill::all_companion_sources()
        .iter()
        .map(|c| c.filename)
        .collect();
    for companion in skill::all_companion_sources() {
        if !managed_companions.iter().any(|c| c == companion.filename) {
            continue; // codex does not manage this companion here
        }
        let path = shared_root.join(companion.filename);
        out.push(check_codex_companion(&companion, &path, binary));
    }
    for filename in &managed_companions {
        if bundled.contains(filename.as_str()) {
            continue; // still bundled → audited by the sync check above
        }
        let path = shared_root.join(filename);
        if std::fs::symlink_metadata(&path).is_ok() {
            out.push(CheckResult::warn(
                format!("skill.orphan.codex._shared.{filename}"),
                format!(
                    "codex companion '_shared/{filename}' at {} is taskfleet-managed but no bundled skill references it any more (de-registered)",
                    path.display()
                ),
                "taskfleet skill install --agent codex --force",
            ));
        }
    }
}

/// Audit one codex `_shared/<file>` companion against the binary's bundled
/// copy. Content identity is the in-sync signal (a freshly installed
/// companion is byte-identical); on a difference, classify by the declared
/// `cli_version` with the same drift model as the claude companion check.
/// No autonomous fix (see [`check_codex`]).
fn check_codex_companion(
    companion: &skill::CompanionSource,
    path: &std::path::Path,
    binary: &str,
) -> CheckResult {
    let filename = companion.filename;
    let id = format!("skill.sync.codex._shared.{filename}");
    let suggest = "taskfleet skill install --agent codex --force".to_string();

    let on_disk = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return CheckResult::warn(
                id,
                format!(
                    "codex companion '_shared/{filename}' is not installed at {}",
                    path.display()
                ),
                suggest,
            );
        }
        Err(e) => {
            return CheckResult::warn(
                id,
                format!(
                    "codex companion '_shared/{filename}' is unreadable at {}: {e}",
                    path.display()
                ),
                suggest,
            );
        }
    };

    if on_disk == companion.bundled_body {
        return CheckResult::ok(
            id,
            format!(
                "codex companion '_shared/{filename}' matches the bundled content for binary {binary}"
            ),
        );
    }

    match skill::cli_version_of(&on_disk)
        .as_deref()
        .and_then(|v| compare(v, binary).map(|o| (v, o)))
    {
        Some((v, Ordering::Less)) => CheckResult::warn(
            id,
            format!(
                "codex companion '_shared/{filename}' is cli_version {v}, binary is {binary}"
            ),
            suggest,
        ),
        Some((v, Ordering::Greater)) => CheckResult::warn(
            id,
            format!(
                "codex companion '_shared/{filename}' differs from the bundled copy and declares cli_version {v}, newer than binary {binary}"
            ),
            "upgrade the taskfleet binary, or reinstall with --agent codex --force to restore the bundled companion",
        ),
        Some((_, Ordering::Equal)) => CheckResult::warn(
            id,
            format!(
                "codex companion '_shared/{filename}' differs from the bundled copy while its cli_version matches binary {binary} (possible local edits)"
            ),
            suggest,
        ),
        None => CheckResult::warn(
            id,
            format!(
                "codex companion '_shared/{filename}' differs from the bundled copy and declares no parseable cli_version at {}",
                path.display()
            ),
            suggest,
        ),
    }
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
/// Note: this audits only companions the *current* binary bundles (the
/// forward direction). A companion a prior binary installed but this one no
/// longer ships is an ORPHAN, detected separately by the
/// `skill.orphan.<name>.<file>` pass in [`check`] (backed by the provenance
/// marker's `companion:` records + `skill::orphan_companions`).
fn check_companion(
    skill_name: &str,
    companion: &skill::CompanionSource,
    path: &std::path::Path,
    binary: &str,
) -> CheckResult {
    let filename = companion.filename;
    let id = format!("skill.sync.{skill_name}.{filename}");
    let suggest_install = format!("taskfleet skill install {skill_name} --force");

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
            "upgrade the taskfleet binary, or reinstall with --force to restore the bundled companion",
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
