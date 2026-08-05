//! `skill` subcommand — list / show / print / install companion AI-skills.
//!
//! Skill files (`SKILL.template.md`) live under
//! `crates/octl-cli/skills/<name>/`. At build time, `build.rs` substitutes
//! `{{CLI_VERSION}}` with the crate's Cargo version and writes the
//! result to `$OUT_DIR/skills/<name>/SKILL.md`. The generated files are
//! embedded into the binary at compile time via `include_str!`, so they
//! version with the CLI. See `AGENTS-AI-FIRST-CLI.md` §15-§17.

use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;
use tempfile::NamedTempFile;

use crate::error::CliError;
use crate::output::{self, OutputFormat, OutputSpec};

/// One embedded skill: name and full SKILL.md text. The description is
/// parsed lazily from the body's frontmatter so the catalog stays a
/// single source of truth (the SKILL.md file).
struct EmbeddedSkill {
    name: &'static str,
    body: &'static str,
    path_in_repo: &'static str,
}

/// A companion reference file that ships *alongside* a skill's `SKILL.md`
/// and installs into the same directory. Used for shared reference prose
/// that more than one skill links to (e.g. the stint family's
/// `AGENTS-EXECUTION-DAG.md`), so the linked file actually exists on disk
/// next to the installed skill in whatever project the skill runs in.
struct EmbeddedResource {
    filename: &'static str,
    body: &'static str,
}

/// Companion resources for a skill, keyed by skill name. Most skills have
/// none. `build.rs` renders every non-`SKILL.template.md` `*.md` file in a
/// skill's directory into `$OUT_DIR/skills/<name>/`, and the matching
/// `include_str!` below embeds it. `cmd_install` writes each resource as a
/// sibling of the skill's `SKILL.md` destination — for the claude layout
/// only (see the install loop for why codex is skipped).
fn resources_for(name: &str) -> &'static [EmbeddedResource] {
    match name {
        "stint-start" => STINT_START_RESOURCES,
        _ => &[],
    }
}

const STINT_START_RESOURCES: &[EmbeddedResource] = &[EmbeddedResource {
    filename: "AGENTS-EXECUTION-DAG.md",
    body: include_str!(concat!(
        env!("OUT_DIR"),
        "/skills/stint-start/AGENTS-EXECUTION-DAG.md"
    )),
}];

const SKILLS: &[EmbeddedSkill] = &[
    EmbeddedSkill {
        name: "orchestratectl-overview",
        body: include_str!(concat!(
            env!("OUT_DIR"),
            "/skills/orchestratectl-overview/SKILL.md"
        )),
        path_in_repo: "crates/octl-cli/skills/orchestratectl-overview/SKILL.template.md",
    },
    EmbeddedSkill {
        name: "octl-run-overview",
        body: include_str!(concat!(
            env!("OUT_DIR"),
            "/skills/octl-run-overview/SKILL.md"
        )),
        path_in_repo: "crates/octl-cli/skills/octl-run-overview/SKILL.template.md",
    },
    EmbeddedSkill {
        name: "octl-spawn-spinoff",
        body: include_str!(concat!(
            env!("OUT_DIR"),
            "/skills/octl-spawn-spinoff/SKILL.md"
        )),
        path_in_repo: "crates/octl-cli/skills/octl-spawn-spinoff/SKILL.template.md",
    },
    EmbeddedSkill {
        name: "worktree-spinoff",
        body: include_str!(concat!(
            env!("OUT_DIR"),
            "/skills/worktree-spinoff/SKILL.md"
        )),
        path_in_repo: "crates/octl-cli/skills/worktree-spinoff/SKILL.template.md",
    },
    EmbeddedSkill {
        name: "worktree-code",
        body: include_str!(concat!(env!("OUT_DIR"), "/skills/worktree-code/SKILL.md")),
        path_in_repo: "crates/octl-cli/skills/worktree-code/SKILL.template.md",
    },
    EmbeddedSkill {
        name: "worktree-merge",
        body: include_str!(concat!(env!("OUT_DIR"), "/skills/worktree-merge/SKILL.md")),
        path_in_repo: "crates/octl-cli/skills/worktree-merge/SKILL.template.md",
    },
    EmbeddedSkill {
        name: "worktree-orchestrated",
        body: include_str!(concat!(
            env!("OUT_DIR"),
            "/skills/worktree-orchestrated/SKILL.md"
        )),
        path_in_repo: "crates/octl-cli/skills/worktree-orchestrated/SKILL.template.md",
    },
    EmbeddedSkill {
        name: "worktree-research",
        body: include_str!(concat!(
            env!("OUT_DIR"),
            "/skills/worktree-research/SKILL.md"
        )),
        path_in_repo: "crates/octl-cli/skills/worktree-research/SKILL.template.md",
    },
    EmbeddedSkill {
        name: "worktree-make-skill",
        body: include_str!(concat!(
            env!("OUT_DIR"),
            "/skills/worktree-make-skill/SKILL.md"
        )),
        path_in_repo: "crates/octl-cli/skills/worktree-make-skill/SKILL.template.md",
    },
    EmbeddedSkill {
        name: "worktree-bugfix",
        body: include_str!(concat!(env!("OUT_DIR"), "/skills/worktree-bugfix/SKILL.md")),
        path_in_repo: "crates/octl-cli/skills/worktree-bugfix/SKILL.template.md",
    },
    EmbeddedSkill {
        name: "worktree-technical-decision",
        body: include_str!(concat!(
            env!("OUT_DIR"),
            "/skills/worktree-technical-decision/SKILL.md"
        )),
        path_in_repo: "crates/octl-cli/skills/worktree-technical-decision/SKILL.template.md",
    },
    EmbeddedSkill {
        name: "worktree-bug-analysis",
        body: include_str!(concat!(
            env!("OUT_DIR"),
            "/skills/worktree-bug-analysis/SKILL.md"
        )),
        path_in_repo: "crates/octl-cli/skills/worktree-bug-analysis/SKILL.template.md",
    },
    EmbeddedSkill {
        name: "worktree",
        body: include_str!(concat!(env!("OUT_DIR"), "/skills/worktree/SKILL.md")),
        path_in_repo: "crates/octl-cli/skills/worktree/SKILL.template.md",
    },
    EmbeddedSkill {
        name: "worktree-status",
        body: include_str!(concat!(env!("OUT_DIR"), "/skills/worktree-status/SKILL.md")),
        path_in_repo: "crates/octl-cli/skills/worktree-status/SKILL.template.md",
    },
    EmbeddedSkill {
        name: "stint-start",
        body: include_str!(concat!(env!("OUT_DIR"), "/skills/stint-start/SKILL.md")),
        path_in_repo: "crates/octl-cli/skills/stint-start/SKILL.template.md",
    },
    EmbeddedSkill {
        name: "stint-handoff",
        body: include_str!(concat!(env!("OUT_DIR"), "/skills/stint-handoff/SKILL.md")),
        path_in_repo: "crates/octl-cli/skills/stint-handoff/SKILL.template.md",
    },
    EmbeddedSkill {
        name: "fan-out",
        body: include_str!(concat!(env!("OUT_DIR"), "/skills/fan-out/SKILL.md")),
        path_in_repo: "crates/octl-cli/skills/fan-out/SKILL.template.md",
    },
    EmbeddedSkill {
        name: "orchestrate",
        body: include_str!(concat!(env!("OUT_DIR"), "/skills/orchestrate/SKILL.md")),
        path_in_repo: "crates/octl-cli/skills/orchestrate/SKILL.template.md",
    },
];

/// Binary version embedded at build time. `build.rs` substitutes this
/// into every shipped SKILL.md's `cli_version:` frontmatter, so `skill
/// print` always returns a body whose declared `cli_version` matches the
/// binary that emitted it (AGENTS-AI-FIRST-CLI §17).
const CLI_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Skill-format schema version (the version of the SKILL.md frontmatter +
/// body contract itself, distinct from the envelope `schema_version`).
const SKILL_SCHEMA_VERSION: u32 = 1;

/// Provenance marker filename. `cmd_install` drops this hidden file into
/// every claude-layout skill directory it writes (`~/.claude/skills/
/// <name>/.orchestratectl-managed`). Its *presence* is the ONLY signal
/// `prune` and the `skill.orphan.*` doctor check use to decide a directory
/// is safe to delete: a user's own hand-authored skill of the same name
/// never carries it, so it is never touched. See `is_managed_skill_dir`.
const MANAGED_MARKER_FILENAME: &str = ".orchestratectl-managed";

/// Public catalog entry used by `version --json` to expose the bundled
/// skill set (AGENTS-AI-FIRST-CLI §17). The on-disk skill is the source
/// of truth for `cli_version`; emit it directly from the parsed
/// frontmatter so the version payload cannot quietly disagree with what
/// `skill print` returns.
#[derive(Debug, Serialize)]
pub struct SkillCatalogEntry {
    pub name: &'static str,
    pub cli_version: String,
    pub schema_version: u32,
}

pub fn catalog() -> Vec<SkillCatalogEntry> {
    SKILLS
        .iter()
        .map(|s| SkillCatalogEntry {
            name: s.name,
            cli_version: parse_frontmatter_field(s.body, "cli_version")
                .unwrap_or_else(|| CLI_VERSION.to_string()),
            schema_version: parse_frontmatter_field(s.body, "schema_version")
                .and_then(|v| v.parse().ok())
                .unwrap_or(SKILL_SCHEMA_VERSION),
        })
        .collect()
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum AgentTarget {
    Claude,
    Codex,
    All,
}

/// Names of every skill bundled in this binary. Consumed by `doctor`'s
/// `skill.sync` check so it audits the exact catalog the binary ships.
pub fn bundled_skill_names() -> Vec<&'static str> {
    SKILLS.iter().map(|s| s.name).collect()
}

/// The running binary's version — the authority `skill.sync` compares
/// each on-disk skill's `cli_version` against.
pub fn binary_cli_version() -> &'static str {
    CLI_VERSION
}

/// Default `claude` install path for a skill (`~/.claude/skills/<name>/
/// SKILL.md`). `None` when `HOME` is unset. Used by `doctor` to locate
/// the on-disk copy to compare against the binary.
pub fn claude_default_path(name: &str) -> Option<PathBuf> {
    default_path("claude", name).ok()
}

/// Read the `cli_version` frontmatter field from an on-disk SKILL.md.
/// `None` when the file is unreadable or has no parseable `cli_version`.
pub fn read_on_disk_cli_version(path: &Path) -> Option<String> {
    let body = fs::read_to_string(path).ok()?;
    parse_frontmatter_field(&body, "cli_version")
}

/// Parse the `cli_version` frontmatter field from an in-memory body (e.g.
/// a companion resource already read off disk). Sibling of
/// [`read_on_disk_cli_version`] for content the caller has in hand.
pub fn cli_version_of(body: &str) -> Option<String> {
    parse_frontmatter_field(body, "cli_version")
}

/// One companion resource bundled alongside a skill's `SKILL.md`, surfaced
/// for the `doctor` `skill.sync.<name>.<file>` companion sub-check: the
/// filename and the embedded (authoritative) body. The expected install
/// path is a sibling of the skill's `SKILL.md` — the doctor derives it from
/// the resolved `SKILL.md` path it already holds.
pub struct CompanionSource {
    pub filename: &'static str,
    pub bundled_body: &'static str,
}

/// Every companion resource bundled for skill `name` (empty for skills that
/// ship none). Consumed by `doctor` to audit that each companion is present
/// and version-synced with the binary, mirroring the SKILL.md `skill.sync`
/// check.
pub fn companion_sources(name: &str) -> Vec<CompanionSource> {
    resources_for(name)
        .iter()
        .map(|r| CompanionSource {
            filename: r.filename,
            bundled_body: r.body,
        })
        .collect()
}

#[derive(Serialize)]
struct SkillSummary {
    name: &'static str,
    description: String,
}

#[derive(Serialize)]
struct ListPayload {
    skills: Vec<SkillSummary>,
}

#[derive(Serialize)]
struct InstallPayload {
    installed: Vec<InstalledFile>,
    /// Names of de-registered managed skill directories that this install
    /// pruned from `~/.claude/skills/`. Empty in every install form except
    /// the full-catalog default-path claude install (see `cmd_install`).
    pruned: Vec<String>,
}

#[derive(Serialize)]
struct InstalledFile {
    name: &'static str,
    agent: &'static str,
    path: String,
}

pub fn cmd_list(spec: &OutputSpec, warnings: &[String]) -> Result<(), CliError> {
    let skills: Vec<SkillSummary> = SKILLS
        .iter()
        .map(|s| SkillSummary {
            name: s.name,
            description: parse_description(s.body).unwrap_or_default(),
        })
        .collect();
    match spec.format {
        OutputFormat::Json | OutputFormat::Jsonl => {
            output::emit_envelope(&ListPayload { skills }, spec, warnings)?;
        }
        OutputFormat::Text => {
            for s in &skills {
                println!("{}\t{}", s.name, s.description);
            }
            output::emit_text_warnings(warnings);
        }
    }
    Ok(())
}

pub fn cmd_show(name: &str, spec: &OutputSpec, warnings: &[String]) -> Result<(), CliError> {
    let skill = lookup(name)?;
    match spec.format {
        OutputFormat::Json | OutputFormat::Jsonl => {
            #[derive(Serialize)]
            struct ShowPayload<'a> {
                name: &'a str,
                content: &'a str,
            }
            output::emit_envelope(
                &ShowPayload {
                    name: skill.name,
                    content: skill.body,
                },
                spec,
                warnings,
            )?;
        }
        OutputFormat::Text => {
            print!("{}", skill.body);
            if !skill.body.ends_with('\n') {
                println!();
            }
            output::emit_text_warnings(warnings);
        }
    }
    Ok(())
}

/// `skill print <name>` — stream the canonical embedded SKILL.md text
/// to stdout, byte-identical to what `skill install` would persist
/// (AGENTS-AI-FIRST-CLI §16). Pure read; no filesystem mutation.
pub fn cmd_print(name: &str, spec: &OutputSpec, warnings: &[String]) -> Result<(), CliError> {
    let skill = lookup(name)?;
    let cli_version = parse_frontmatter_field(skill.body, "cli_version")
        .unwrap_or_else(|| CLI_VERSION.to_string());
    let schema_version_skill = parse_frontmatter_field(skill.body, "schema_version")
        .and_then(|v| v.parse().ok())
        .unwrap_or(SKILL_SCHEMA_VERSION);
    match spec.format {
        OutputFormat::Json => {
            #[derive(Serialize)]
            struct PrintPayload<'a> {
                /// Skill-print payload schema; bumps independently from
                /// the envelope's `schema_version`.
                schema_version: u32,
                name: &'a str,
                cli_version: &'a str,
                schema_version_skill: u32,
                content: &'a str,
                path_in_repo: &'a str,
            }
            output::emit_envelope(
                &PrintPayload {
                    schema_version: SKILL_SCHEMA_VERSION,
                    name: skill.name,
                    cli_version: &cli_version,
                    schema_version_skill,
                    content: skill.body,
                    path_in_repo: skill.path_in_repo,
                },
                spec,
                warnings,
            )?;
        }
        // §16 contract: text and jsonl both stream the SKILL.md
        // byte-identically. The structured form is opt-in via `--output
        // json`; the default (jsonl) is byte-identity so `skill print`
        // composes with `cat`, `tee`, and shell redirection without any
        // un-wrapping step.
        OutputFormat::Text | OutputFormat::Jsonl => {
            use std::io::Write as _;
            let mut out = std::io::stdout().lock();
            out.write_all(skill.body.as_bytes())
                .map_err(|e| CliError::system("io_error", format!("write stdout: {e}")))?;
            out.flush()
                .map_err(|e| CliError::system("io_error", format!("flush stdout: {e}")))?;
            output::emit_text_warnings(warnings);
        }
    }
    Ok(())
}

pub fn cmd_install(
    name: Option<&str>,
    agent: AgentTarget,
    dest: Option<PathBuf>,
    force: bool,
    spec: &OutputSpec,
    warnings: &[String],
) -> Result<(), CliError> {
    // §15: `install [<name>]` installs all skills when no name is given.
    let skills: Vec<&'static EmbeddedSkill> = match name {
        Some(n) => vec![lookup(n)?],
        None => SKILLS.iter().collect(),
    };

    // `--dest` is incompatible with `--agent all` and with the implicit
    // install-all form: a single path cannot host multiple installations.
    if dest.is_some() && matches!(agent, AgentTarget::All) {
        return Err(CliError::user(
            "invalid_arguments",
            "--dest cannot be combined with --agent all",
        ));
    }
    if dest.is_some() && skills.len() > 1 {
        return Err(CliError::user(
            "invalid_arguments",
            "--dest requires a skill name; omit --dest to install all skills",
        ));
    }

    // Build the full plan first, then preflight, then write. This avoids
    // the partial-install retry trap where one of N writes succeeds and a
    // re-run hits refused_overwrite on a different path than the original
    // failure. Each skill contributes its `SKILL.md` plus any companion
    // resources, installed as siblings of the skill's destination.
    let mut plan: Vec<PlanItem> = Vec::new();
    // Claude-layout skill directories this install touches under the
    // default path — each gets the provenance marker stamped after the
    // writes land. Only the default path (`dest.is_none()`) is a real
    // `~/.claude/skills/<name>/` directory we own; a `--dest` custom path
    // is the caller's to manage, so we never litter a marker there.
    let mut mark_dirs: Vec<(&'static str, PathBuf)> = Vec::new();
    for skill in &skills {
        let targets: Vec<(&'static str, PathBuf)> = match (&agent, dest.as_ref()) {
            (AgentTarget::Claude, Some(p)) => vec![("claude", p.clone())],
            (AgentTarget::Codex, Some(p)) => vec![("codex", p.clone())],
            (AgentTarget::Claude, None) => vec![("claude", default_path("claude", skill.name)?)],
            (AgentTarget::Codex, None) => vec![("codex", default_path("codex", skill.name)?)],
            (AgentTarget::All, _) => vec![
                ("claude", default_path("claude", skill.name)?),
                ("codex", default_path("codex", skill.name)?),
            ],
        };
        for (agent_name, path) in targets {
            // Companion resources are per-skill sibling files, which only
            // works in the claude layout (`~/.claude/skills/<name>/`). The
            // codex layout is a flat prompts directory, where a sibling
            // would land un-namespaced in `~/.codex/prompts/` and could
            // collide across skills — and cross-skill links like
            // `../stint-start/…` cannot resolve there regardless. So we
            // install resources for claude only.
            if agent_name == "claude" {
                if dest.is_none() {
                    if let Some(parent) = path.parent() {
                        mark_dirs.push((skill.name, parent.to_path_buf()));
                    }
                }
                for resource in resources_for(skill.name) {
                    plan.push(PlanItem {
                        name: resource.filename,
                        agent: agent_name,
                        path: sibling_path(&path, resource.filename),
                        content: resource.body,
                    });
                }
            }
            plan.push(PlanItem {
                name: skill.name,
                agent: agent_name,
                path,
                content: skill.body,
            });
        }
    }

    let preflight_result = preflight(&plan, force)?;

    // Combine caller-provided warnings (logging init, etc.) with
    // drift-detected ones so the success envelope surfaces both.
    let mut all_warnings: Vec<String> = warnings.to_vec();
    all_warnings.extend(preflight_result.warnings);

    let mut installed = Vec::with_capacity(plan.len());
    for item in plan {
        // The set of paths approved for overwrite is decided exclusively
        // by preflight — never recomputed from `path.exists()` in this
        // loop. That keeps the persist_noclobber TOCTOU guarantee intact:
        // a file that did not exist at preflight time will refuse to
        // overwrite, even if a concurrent process created it in the
        // window. (Review finding #1.)
        let allow_overwrite = preflight_result.overwrite_allowed.contains(&item.path);
        write_atomic(&item.path, item.content, allow_overwrite)?;
        installed.push(InstalledFile {
            name: item.name,
            agent: item.agent,
            path: item.path.display().to_string(),
        });
    }

    // Stamp every freshly-installed claude-layout directory with the
    // provenance marker (idempotent, always overwritten — it's ours). The
    // marker is what makes later pruning safe, so a write failure here is
    // fatal rather than silent: without it a genuine orphan would never be
    // recognised as managed. Deduplicated because `--agent all` and the
    // install-all form can enumerate the same dir once per skill.
    mark_dirs.sort();
    mark_dirs.dedup();
    for (skill_name, dir) in &mark_dirs {
        write_marker(&dir.join(MANAGED_MARKER_FILENAME), skill_name)?;
    }

    // Prune de-registered managed skills. Scoped to the full-catalog
    // (`name.is_none()`), default-path (`dest.is_none()`), `--force`,
    // claude-targeting install: that is exactly the `skill install --force`
    // redeploy where the caller intends the on-disk catalog to mirror the
    // binary's. `--force` is required so a plain, non-destructive-looking
    // `skill install` can never delete a directory as a side effect. A
    // targeted `skill install <name>` must NEVER nuke the rest of the
    // catalog, so it is excluded too. Only directories carrying a VALID
    // marker (see `is_managed_skill_dir`) AND absent from the shipped
    // catalog are removed — a user's hand-authored or copied skill is
    // spared. A prune failure is a maintenance hiccup, not an install
    // failure: the catalog already landed, so we warn and carry on rather
    // than erroring out with the success payload unreported.
    let mut pruned: Vec<String> = Vec::new();
    let prune_eligible = name.is_none()
        && dest.is_none()
        && force
        && matches!(agent, AgentTarget::Claude | AgentTarget::All);
    if prune_eligible {
        if let Some(root) = claude_skills_root() {
            let registered: HashSet<&str> = SKILLS.iter().map(|s| s.name).collect();
            for (orphan_name, orphan_path) in managed_orphan_dirs(&root, &registered) {
                match fs::remove_dir_all(&orphan_path) {
                    Ok(()) => {
                        all_warnings.push(format!(
                            "skill_pruned: removed de-registered managed skill '{orphan_name}' at {}",
                            orphan_path.display()
                        ));
                        pruned.push(orphan_name);
                    }
                    Err(e) => {
                        all_warnings.push(format!(
                            "skill_prune_failed: could not remove de-registered skill '{orphan_name}' at {}: {e}",
                            orphan_path.display()
                        ));
                    }
                }
            }
        }
    }

    let payload = InstallPayload { installed, pruned };
    match spec.format {
        OutputFormat::Json | OutputFormat::Jsonl => {
            output::emit_envelope(&payload, spec, &all_warnings)?;
        }
        OutputFormat::Text => {
            for f in &payload.installed {
                println!("installed {} ({}) -> {}", f.name, f.agent, f.path);
            }
            for name in &payload.pruned {
                println!("pruned {name} (de-registered)");
            }
            output::emit_text_warnings(&all_warnings);
        }
    }
    Ok(())
}

/// Compare two `cli_version` strings via the `semver` crate. Returns
/// `None` if either side fails to parse as semver — callers treat that
/// as "unversioned / legacy" rather than guessing an ordering.
/// (Review finding #2 — the previous ad-hoc parser inverted prerelease
/// ordering: `1.0.0-alpha > 1.0.0` instead of `<`.)
fn compare_versions(a: &str, b: &str) -> Option<std::cmp::Ordering> {
    let av = semver::Version::parse(a).ok()?;
    let bv = semver::Version::parse(b).ok()?;
    Some(av.cmp(&bv))
}

/// Outcome of the install preflight pass.
///
/// `overwrite_allowed` is the *authoritative* set of paths the write
/// loop is permitted to clobber. Computed once, then never recomputed —
/// see `cmd_install` for the TOCTOU rationale.
struct PreflightResult {
    warnings: Vec<String>,
    overwrite_allowed: HashSet<PathBuf>,
}

/// One file the install will write: a skill's `SKILL.md` or one of its
/// companion resources. `name` is what the install payload and drift
/// warnings report (the skill name, or the resource filename).
struct PlanItem {
    name: &'static str,
    agent: &'static str,
    path: PathBuf,
    content: &'static str,
}

/// Resolve a companion resource's destination: a file named `filename` in
/// the same directory as the skill's `SKILL.md` destination `skill_path`.
/// A bare relative `skill_path` (empty parent, e.g. `--dest SKILL.md`)
/// places the resource in the current directory.
fn sibling_path(skill_path: &Path, filename: &str) -> PathBuf {
    match skill_path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.join(filename),
        _ => PathBuf::from(filename),
    }
}

/// Reject the whole install plan before touching the filesystem when any
/// destination already exists (without `--force`) or appears twice in
/// the plan. Catches the partial-install retry trap noted by the review:
/// without preflight, writing N targets sequentially can leave the user
/// with a half-installed catalog and an ambiguous error on retry.
fn preflight(plan: &[PlanItem], force: bool) -> Result<PreflightResult, CliError> {
    use std::cmp::Ordering;
    let mut seen: HashSet<&Path> = HashSet::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut overwrite_allowed: HashSet<PathBuf> = HashSet::new();
    for PlanItem { name, path, .. } in plan {
        if !seen.insert(path.as_path()) {
            return Err(CliError::user(
                "duplicate_destination",
                format!("destination appears more than once: {}", path.display()),
            )
            .with_invalid_value(path.display().to_string()));
        }
        if path.is_dir() {
            return Err(CliError::user(
                "invalid_dest",
                format!("destination is a directory: {}", path.display()),
            )
            .with_invalid_value(path.display().to_string()));
        }
        if !path.exists() {
            // No file → the write loop will refuse to clobber via
            // persist_noclobber. Do not insert into `overwrite_allowed`.
            continue;
        }
        // Existing target: classify by semver-correct `cli_version`
        // drift relative to the running binary (AGENTS-AI-FIRST-CLI §17).
        // An unreadable or unparseable `cli_version` lands in the
        // "legacy / unversioned" arm so we never invent an ordering.
        let on_disk_raw = fs::read_to_string(path)
            .ok()
            .and_then(|s| parse_frontmatter_field(&s, "cli_version"));
        let drift = on_disk_raw
            .as_deref()
            .and_then(|v| compare_versions(v, CLI_VERSION).map(|ord| (v, ord)));
        match drift {
            Some((v, Ordering::Less)) => {
                // Older on disk: install proceeds with a warning so the
                // agent learns the operating manual just moved.
                overwrite_allowed.insert(path.clone());
                warnings.push(format!(
                    "skill_version_drift: {name} on disk is {v}; binary ships {CLI_VERSION}; overwriting"
                ));
            }
            Some((v, Ordering::Greater)) => {
                if !force {
                    return Err(CliError::system(
                        "skill_version_too_new",
                        format!(
                            "{}: on-disk skill is cli_version {} but binary is {}; pass --force to overwrite anyway",
                            path.display(),
                            v,
                            CLI_VERSION
                        ),
                    )
                    .with_invalid_value(path.display().to_string()));
                }
                overwrite_allowed.insert(path.clone());
                warnings.push(format!(
                    "skill_version_drift: {name} on disk is {v} (newer than binary {CLI_VERSION}); --force overwriting"
                ));
            }
            Some((_, Ordering::Equal)) | None => {
                // Either equal versions (already in sync) or no
                // parseable `cli_version` on disk (legacy / unversioned
                // / unreadable). Both require explicit --force; we
                // refuse to invent an overwrite policy.
                if !force {
                    return Err(CliError::system(
                        "refused_overwrite",
                        format!(
                            "{} already exists; pass --force to overwrite",
                            path.display()
                        ),
                    )
                    .with_invalid_value(path.display().to_string()));
                }
                overwrite_allowed.insert(path.clone());
            }
        }
    }
    Ok(PreflightResult {
        warnings,
        overwrite_allowed,
    })
}

fn lookup(name: &str) -> Result<&'static EmbeddedSkill, CliError> {
    SKILLS.iter().find(|s| s.name == name).ok_or_else(|| {
        let available: Vec<&str> = SKILLS.iter().map(|s| s.name).collect();
        CliError::user(
            "skill_not_found",
            format!(
                "no skill named '{}'; available: {}",
                name,
                available.join(", ")
            ),
        )
        .with_invalid_value(name.to_string())
        .with_expected(serde_json::json!({ "one_of": available }))
    })
}

fn default_path(agent: &str, name: &str) -> Result<PathBuf, CliError> {
    let home = std::env::var("HOME").map_err(|_| {
        CliError::system(
            "home_unset",
            "HOME is not set; cannot resolve default install path (pass --dest)",
        )
    })?;
    let base = PathBuf::from(home);
    Ok(match agent {
        "claude" => base.join(".claude/skills").join(name).join("SKILL.md"),
        "codex" => base.join(".codex/prompts").join(format!("{name}.md")),
        // unreachable in practice — callers only pass the literals above.
        other => {
            return Err(CliError::user(
                "invalid_agent",
                format!("unknown agent '{other}'"),
            ))
        }
    })
}

/// Root of the claude skill-install layout (`~/.claude/skills`). `None`
/// when `HOME` is unset. Both `prune` and the `skill.orphan.*` doctor
/// check scan this directory for managed-but-de-registered skills.
pub fn claude_skills_root() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".claude/skills"))
}

/// True when `dir` is a claude-layout skill directory that orchestratectl
/// installed. The guard is deliberately strict — its `true` is the SOLE
/// authorization for a recursive `remove_dir_all`, so it must never yield
/// a false positive on a user's own directory. Three conditions must ALL
/// hold:
///
/// 1. The marker is a **regular file** (`symlink_metadata`, which does not
///    follow links) — a planted `.orchestratectl-managed` *symlink* cannot
///    make a directory look managed.
/// 2. The marker carries the `managed-by: orchestratectl` stamp.
/// 3. The marker's recorded `skill_name` equals this directory's name.
///    This binding is what makes `cp -r managed-skill my-copy` safe: the
///    copy's marker still names the ORIGINAL skill, so it never matches
///    `my-copy` and the copy is spared.
fn is_managed_skill_dir(dir: &Path) -> bool {
    let Some(dir_name) = dir.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let marker = dir.join(MANAGED_MARKER_FILENAME);
    let Ok(meta) = fs::symlink_metadata(&marker) else {
        return false;
    };
    if !meta.file_type().is_file() {
        return false;
    }
    let Ok(content) = fs::read_to_string(&marker) else {
        return false;
    };
    let mut has_stamp = false;
    let mut name_matches = false;
    for line in content.lines() {
        let line = line.trim();
        if line == "managed-by: orchestratectl" {
            has_stamp = true;
        } else if let Some(rest) = line.strip_prefix("skill_name:") {
            name_matches = rest.trim() == dir_name;
        }
    }
    has_stamp && name_matches
}

/// Write (or overwrite) the provenance marker for skill `skill_name` at
/// `path`. The recorded `skill_name` is what `is_managed_skill_dir` binds
/// against, so a copied-and-renamed skill is never mistaken for an orphan.
/// The parent directory already exists (the SKILL.md write created it), so
/// this is a plain overwrite; the marker is always ours. If a symlink is
/// squatting at the marker path we unlink it first so `fs::write` cannot
/// clobber the link's target (an arbitrary-file overwrite within the
/// user's permissions).
fn write_marker(path: &Path, skill_name: &str) -> Result<(), CliError> {
    if let Ok(meta) = fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            fs::remove_file(path).map_err(|e| {
                CliError::system(
                    "marker_write_failed",
                    format!(
                        "could not clear stale marker symlink {}: {}",
                        path.display(),
                        e
                    ),
                )
            })?;
        }
    }
    fs::write(
        path,
        format!(
            "managed-by: orchestratectl\ncli_version: {CLI_VERSION}\nskill_name: {skill_name}\n"
        ),
    )
    .map_err(|e| {
        CliError::system(
            "marker_write_failed",
            format!(
                "could not write provenance marker {}: {}",
                path.display(),
                e
            ),
        )
    })
}

/// Scan `skills_root` for managed skill directories whose name is NOT in
/// `registered`. These are directories orchestratectl installed (they
/// carry a valid provenance marker naming that same directory) but the
/// running binary no longer ships — renamed or removed bundled skills,
/// safe to prune. Directories without a valid marker (a user's own skills)
/// are never returned. Result is sorted by name for deterministic output.
/// An unreadable root, or an unreadable entry, is skipped rather than
/// guessed at — the prune path must always err toward NOT deleting.
fn managed_orphan_dirs(skills_root: &Path, registered: &HashSet<&str>) -> Vec<(String, PathBuf)> {
    let Ok(entries) = fs::read_dir(skills_root) else {
        return Vec::new();
    };
    let mut orphans: Vec<(String, PathBuf)> = Vec::new();
    for entry in entries {
        let Ok(entry) = entry else { continue };
        // `file_type()` is an lstat: it never follows a symlink. A
        // symlinked entry — even one pointing at a real directory — is
        // rejected so `remove_dir_all` can never traverse a link out of
        // the skills root and delete an unrelated tree.
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() || file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        let Some(dir_name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        // Never prune a directory that matches a registered skill —
        // exactly, OR case-insensitively (a differently-cased alias on a
        // case-insensitive filesystem, e.g. macOS APFS, resolves to the
        // same on-disk directory a fresh install just wrote).
        if registered
            .iter()
            .any(|r| *r == dir_name || r.eq_ignore_ascii_case(dir_name))
        {
            continue;
        }
        if is_managed_skill_dir(&path) {
            orphans.push((dir_name.to_string(), path.clone()));
        }
    }
    orphans.sort();
    orphans
}

/// Managed-but-de-registered skill directories under the claude install
/// root, as `(name, path)` pairs sorted by name. Consumed by the
/// `skill.orphan.*` doctor check. Empty when `HOME` is unset or the root
/// is unreadable.
pub fn managed_orphans() -> Vec<(String, PathBuf)> {
    let Some(root) = claude_skills_root() else {
        return Vec::new();
    };
    let registered: HashSet<&str> = SKILLS.iter().map(|s| s.name).collect();
    managed_orphan_dirs(&root, &registered)
}

/// Empty parent from `PathBuf::parent()` means a bare relative filename
/// (e.g. `--dest SKILL.md`). Treat that as the current directory rather
/// than failing `create_dir_all("")`.
fn normalized_parent(path: &Path) -> Option<&Path> {
    match path.parent() {
        Some(p) if p.as_os_str().is_empty() => Some(Path::new(".")),
        Some(p) => Some(p),
        None => None,
    }
}

fn write_atomic(path: &Path, content: &str, force: bool) -> Result<(), CliError> {
    let parent = normalized_parent(path).ok_or_else(|| {
        CliError::user(
            "invalid_dest",
            format!("destination has no parent directory: {}", path.display()),
        )
    })?;
    fs::create_dir_all(parent).map_err(|e| {
        CliError::system(
            "create_dir_failed",
            format!("could not create {}: {}", parent.display(), e),
        )
    })?;

    let mut tmp = NamedTempFile::new_in(parent).map_err(|e| {
        CliError::system(
            "tempfile_failed",
            format!("could not create tempfile in {}: {}", parent.display(), e),
        )
    })?;
    tmp.write_all(content.as_bytes())
        .map_err(|e| CliError::system("write_failed", format!("could not write tempfile: {e}")))?;
    tmp.as_file_mut()
        .sync_all()
        .map_err(|e| CliError::system("fsync_failed", format!("could not fsync tempfile: {e}")))?;

    // `persist_noclobber` makes the non-force case TOCTOU-safe: the rename
    // refuses to clobber via the kernel rather than via an earlier
    // `path.exists()` check. The preflight pass above still runs so we
    // surface the friendly `refused_overwrite` envelope early; this is
    // the belt-and-braces guard against a race between preflight and
    // persist.
    let persist_result = if force {
        tmp.persist(path).map(|_| ())
    } else {
        tmp.persist_noclobber(path).map(|_| ())
    };
    persist_result.map_err(|e| {
        // tempfile's PersistError wraps EEXIST; surface the canonical
        // refused_overwrite envelope so callers can branch on the code.
        let kind = e.error.kind();
        if !force && kind == std::io::ErrorKind::AlreadyExists {
            CliError::system(
                "refused_overwrite",
                format!(
                    "{} already exists; pass --force to overwrite",
                    path.display()
                ),
            )
            .with_invalid_value(path.display().to_string())
        } else {
            CliError::system(
                "rename_failed",
                format!("could not rename into place {}: {}", path.display(), e),
            )
        }
    })
}

/// Extract the `description:` field from YAML-ish frontmatter at the top
/// of a SKILL.md.
///
/// Frontmatter is everything between a leading `---` line and the next
/// `---` line. We parse line-by-line (handling both `\n` and `\r\n` line
/// endings via `str::lines`) and accept `key: value` / `key : value`
/// pairs. Quoted values have their surrounding `"` or `'` stripped. This
/// is not a full YAML parser — multi-line scalars and nested maps are
/// out of scope — but it covers every shape our SKILL.md frontmatter is
/// allowed to take.
fn parse_description(body: &str) -> Option<String> {
    parse_frontmatter_field(body, "description")
}

/// Generic line-oriented frontmatter field extractor. Same constraints
/// as `parse_description`: top-level `---` fence, `key: value` shape,
/// single-line scalars only. Strips surrounding `"` / `'` quotes from
/// the value.
fn parse_frontmatter_field(body: &str, field: &str) -> Option<String> {
    let body = body.strip_prefix('\u{feff}').unwrap_or(body);
    let mut lines = body.lines();
    if lines.next()?.trim_end() != "---" {
        return None;
    }
    for line in lines {
        if line.trim_end() == "---" {
            return None;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if key.trim() == field {
            let v = value.trim();
            let v = v
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .or_else(|| v.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
                .unwrap_or(v);
            return Some(v.to_string());
        }
    }
    None
}

/// Extract the `name:` field from frontmatter, same rules as
/// `parse_description`. Only consumed by the build-time consistency
/// test that pins catalog name == frontmatter name.
#[cfg(test)]
fn parse_name(body: &str) -> Option<String> {
    parse_frontmatter_field(body, "name")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_embedded_skill_has_a_description_and_matching_name() {
        // Guards against frontmatter drift: if someone edits a SKILL.md
        // and breaks the `description:` line, `skill list` would silently
        // emit an empty string. Fail the build instead. Also pin
        // `name` equality between the catalog entry and the frontmatter
        // so a rename in one place can't quietly desync from the other.
        // (Review finding #5: extended to pin `cli_version` ==
        // CLI_VERSION and parseable `schema_version` so silent fallbacks
        // in `catalog()` and `cmd_print()` can't mask a broken template.)
        for s in SKILLS {
            let d = parse_description(s.body)
                .unwrap_or_else(|| panic!("skill {} missing description", s.name));
            assert!(!d.is_empty(), "skill {} has empty description", s.name);
            let n = parse_name(s.body).unwrap_or_else(|| panic!("skill {} missing name", s.name));
            assert_eq!(
                n, s.name,
                "catalog name {:?} does not match frontmatter name {:?}",
                s.name, n
            );
            let cli = parse_frontmatter_field(s.body, "cli_version")
                .unwrap_or_else(|| panic!("skill {} missing cli_version", s.name));
            assert_eq!(
                cli, CLI_VERSION,
                "skill {} cli_version {:?} does not match binary {:?}",
                s.name, cli, CLI_VERSION
            );
            let schema = parse_frontmatter_field(s.body, "schema_version")
                .unwrap_or_else(|| panic!("skill {} missing schema_version", s.name));
            let parsed: u32 = schema.parse().unwrap_or_else(|_| {
                panic!(
                    "skill {} has unparseable schema_version {:?}",
                    s.name, schema
                )
            });
            assert_eq!(
                parsed, SKILL_SCHEMA_VERSION,
                "skill {} schema_version {} != {}",
                s.name, parsed, SKILL_SCHEMA_VERSION
            );
            assert!(
                !s.body.contains("{{CLI_VERSION}}"),
                "skill {} still contains unrendered {{{{CLI_VERSION}}}} placeholder",
                s.name
            );
        }
    }

    #[test]
    fn every_companion_resource_is_rendered_and_version_pinned() {
        // The doctor `skill.sync.<name>.<file>` OK arm reports a companion
        // as "matching the bundled content for binary <CLI_VERSION>". That
        // claim only holds if the embedded companion body is fully rendered
        // (no leftover `{{CLI_VERSION}}` placeholder) and — when it carries
        // `cli_version` frontmatter at all — pins it to this binary's
        // version. Guard both here so a companion template can't silently
        // ship stale or unrendered.
        for name in SKILLS.iter().map(|s| s.name) {
            for r in resources_for(name) {
                assert!(
                    !r.body.contains("{{CLI_VERSION}}"),
                    "companion {} for skill {} still contains an unrendered {{{{CLI_VERSION}}}} placeholder",
                    r.filename,
                    name
                );
                if let Some(v) = parse_frontmatter_field(r.body, "cli_version") {
                    assert_eq!(
                        v, CLI_VERSION,
                        "companion {} for skill {} declares cli_version {:?}, binary is {:?}",
                        r.filename, name, v, CLI_VERSION
                    );
                }
            }
        }
    }

    #[test]
    fn compare_versions_handles_semver_ordering() {
        use std::cmp::Ordering;
        // Pre-release is *less than* the release.
        assert_eq!(
            compare_versions("1.0.0-alpha", "1.0.0"),
            Some(Ordering::Less)
        );
        assert_eq!(
            compare_versions("1.0.0", "1.0.0-alpha"),
            Some(Ordering::Greater)
        );
        // Standard numeric ordering.
        assert_eq!(compare_versions("1.10.0", "1.9.0"), Some(Ordering::Greater));
        assert_eq!(compare_versions("0.0.1", "0.0.1"), Some(Ordering::Equal));
        // Unparseable → None so callers route to the legacy arm.
        assert_eq!(compare_versions("banana", "1.0.0"), None);
        assert_eq!(compare_versions("1.0.0", "1.x"), None);
        assert_eq!(compare_versions("{{CLI_VERSION}}", "1.0.0"), None);
    }

    #[test]
    fn parse_description_extracts_value() {
        let body = "---\nname: foo\ndescription: a short blurb\nversion: 1\n---\n\n# body\n";
        assert_eq!(parse_description(body).as_deref(), Some("a short blurb"));
    }

    #[test]
    fn parse_description_handles_crlf() {
        let body = "---\r\nname: foo\r\ndescription: blurb\r\n---\r\n";
        assert_eq!(parse_description(body).as_deref(), Some("blurb"));
    }

    #[test]
    fn parse_description_strips_quotes() {
        let body = "---\ndescription: \"quoted blurb\"\n---\n";
        assert_eq!(parse_description(body).as_deref(), Some("quoted blurb"));
    }

    #[test]
    fn parse_description_returns_none_without_frontmatter() {
        assert_eq!(parse_description("# just a heading\n"), None);
    }

    #[test]
    fn parse_description_returns_none_when_field_absent() {
        let body = "---\nname: foo\nversion: 1\n---\n";
        assert_eq!(parse_description(body), None);
    }
}
