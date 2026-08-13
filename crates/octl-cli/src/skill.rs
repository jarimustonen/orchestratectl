//! `skill` subcommand — list / show / print / install companion AI-skills.
//!
//! Skill files (`SKILL.template.md`) live under
//! `crates/octl-cli/skills/<name>/`. At build time, `build.rs` substitutes
//! `{{CLI_VERSION}}` with the crate's Cargo version and writes the
//! result to `$OUT_DIR/skills/<name>/SKILL.md`. The generated files are
//! embedded into the binary at compile time via `include_str!`, so they
//! version with the CLI. See `AGENTS-AI-FIRST-CLI.md` §15-§17.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;

use crate::error::CliError;
use crate::home;
use crate::output::{self, OutputFormat, OutputSpec};

/// One embedded skill: name and full SKILL.md text. The description is
/// parsed lazily from the body's frontmatter so the catalog stays a
/// single source of truth (the SKILL.md file).
struct EmbeddedSkill {
    name: &'static str,
    body: &'static str,
    path_in_repo: &'static str,
}

/// A companion reference file that ships *alongside* a skill's `SKILL.md`.
/// Used for shared reference prose that more than one skill links to (e.g.
/// the stint family's `AGENTS-EXECUTION-DAG.md`), so the linked file
/// actually exists on disk in whatever project the skill runs in.
///
/// The two supported agent layouts install it differently:
///
/// - **claude** (`~/.claude/skills/<name>/`) — a per-skill directory, so the
///   companion is written as a plain sibling of the owning skill's
///   `SKILL.md` and the in-body links (`AGENTS-EXECUTION-DAG.md` from the
///   owner, `../stint-start/AGENTS-EXECUTION-DAG.md` from a cross-skill
///   linker) resolve as authored.
/// - **codex** (`~/.codex/prompts/`) — a flat prompts dir where every
///   top-level `*.md` becomes a slash-command, so a bare companion would
///   surface as a bogus prompt and collide across skills. The companion is
///   instead written into a shared `_shared/` subdir (ignored by codex's
///   top-level prompt discovery), and `cmd_install` rewrites each
///   `claude_link_target` in the codex skill body to the single
///   `_shared/<filename>` form so the link still resolves.
struct EmbeddedResource {
    filename: &'static str,
    body: &'static str,
    /// The markdown link targets (the payload inside `](…)`) that reference
    /// this companion in the claude-layout skill bodies. On a codex install
    /// each is rewritten to `_shared/<filename>`. Anchoring the rewrite on
    /// the full `](target)` form keeps the shorter sibling target from
    /// matching inside a longer `../owner/target` one.
    claude_link_targets: &'static [&'static str],
}

/// Subdir under the flat codex prompts dir (`~/.codex/prompts/_shared/`)
/// that holds companion reference files. A subdir (not a top-level `.md`)
/// so codex never mistakes a companion for a slash-command prompt, and a
/// single shared location so every skill links to the one copy.
const CODEX_SHARED_SUBDIR: &str = "_shared";

/// Companion resources for a skill, keyed by skill name. Most skills have
/// none. `build.rs` renders every non-`SKILL.template.md` `*.md` file in a
/// skill's directory into `$OUT_DIR/skills/<name>/`, and the matching
/// `include_str!` below embeds it. `cmd_install` writes each resource as a
/// sibling of the skill's `SKILL.md` (claude) or into the `_shared/` subdir
/// (codex) — see `EmbeddedResource` for the layout rationale.
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
    // `stint-start` links to it as a sibling; `stint-handoff` links across
    // to it via `../stint-start/…`. Both collapse to `_shared/…` on codex.
    claude_link_targets: &[
        "AGENTS-EXECUTION-DAG.md",
        "../stint-start/AGENTS-EXECUTION-DAG.md",
    ],
}];

/// The codex-layout link target for a companion: a single shared path every
/// skill body resolves to, relative to the flat prompts dir.
fn codex_link_target(filename: &str) -> String {
    format!("{CODEX_SHARED_SUBDIR}/{filename}")
}

/// Rewrite a skill body for the target agent. Claude bodies are byte-for-byte
/// the embedded source. Codex bodies get every companion's claude-layout link
/// forms rewritten to the shared `_shared/<filename>` target, so the flat
/// prompts layout resolves the same reference the per-skill claude layout
/// does. The rewrite is anchored on the full `](target)` form and is a no-op
/// for any body that references no companion.
///
/// Scope is deliberately the whole catalog, not just the skill being
/// installed: a skill that links *cross-skill* to another's companion (e.g.
/// `stint-handoff` → the DAG owned by `stint-start`) must still resolve to the
/// one shared copy. `claude_link_targets` are distinctive prose strings, so a
/// global replace never touches an unrelated body — pinned by the
/// `every_claude_link_target_appears_in_some_skill_body` test. A standalone
/// install of a cross-linking skill leaves the shared file un-installed until
/// its owner also lands; that dangles the link exactly as the claude layout
/// already does (both resolve once both skills are installed).
fn render_body_for_agent(agent: &str, body: &'static str) -> Cow<'static, str> {
    if agent != "codex" {
        return Cow::Borrowed(body);
    }
    let mut rendered = Cow::Borrowed(body);
    for skill in SKILLS {
        for resource in resources_for(skill.name) {
            let codex_target = codex_link_target(resource.filename);
            for claude_target in resource.claude_link_targets {
                let from = format!("]({claude_target})");
                if rendered.contains(&from) {
                    let to = format!("]({codex_target})");
                    rendered = Cow::Owned(rendered.replace(&from, &to));
                }
            }
        }
    }
    rendered
}

/// Resolve a codex companion's destination: `<prompts-dir>/_shared/<filename>`,
/// where the prompts dir is the parent of the skill's flat prompt file
/// `skill_path`. A bare relative `skill_path` (empty parent) places the
/// `_shared/` subdir in the current directory.
fn codex_companion_path(skill_path: &Path, filename: &str) -> PathBuf {
    let shared_dir = match skill_path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.join(CODEX_SHARED_SUBDIR),
        _ => PathBuf::from(CODEX_SHARED_SUBDIR),
    };
    shared_dir.join(filename)
}

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

/// Default `codex` install path for a skill (`~/.codex/prompts/<name>.md`).
/// `None` when `HOME` is unset. The codex layout is FLAT — a skill is a
/// single top-level prompt file, not a per-skill directory. Used by
/// `doctor` to locate the on-disk codex copy to compare against the binary.
pub fn codex_default_path(name: &str) -> Option<PathBuf> {
    default_path("codex", name).ok()
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

/// Every companion resource any bundled skill ships, deduplicated by
/// filename. Consumed by the codex `doctor` checks: the codex `_shared/`
/// dir is a single shared location every skill's companion lands in, so a
/// companion still referenced by at least one bundled skill is "still
/// bundled" (audited by the forward `skill.sync.codex._shared.<file>`
/// check) rather than an orphan. Sorted by filename for deterministic
/// output.
pub fn all_companion_sources() -> Vec<CompanionSource> {
    let mut seen: HashSet<&'static str> = HashSet::new();
    let mut out: Vec<CompanionSource> = Vec::new();
    for skill in SKILLS {
        for r in resources_for(skill.name) {
            if seen.insert(r.filename) {
                out.push(CompanionSource {
                    filename: r.filename,
                    bundled_body: r.body,
                });
            }
        }
    }
    out.sort_by(|a, b| a.filename.cmp(b.filename));
    out
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
    /// Orphan companion files this `--force` install removed: companions a
    /// prior binary recorded in a still-registered skill's provenance marker
    /// that the current binary no longer bundles. Reported as
    /// `<skill>/<filename>` so the offending sibling is unambiguous. Empty
    /// unless a default-path claude `--force` install found stale companions.
    pruned_companions: Vec<String>,
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

    // Whether this install dual-homes into pi — the same condition that pushes
    // the pi `PlanItem`s below. When it does, load AND validate the out-of-band
    // provenance record NOW, before any file is written, so a corrupt or
    // future-schema record fails the install fast rather than after mutating the
    // tree (review finding A). The loaded record is threaded to the pi lifecycle
    // block after the writes land.
    let pi_dual_home = dest.is_none() && matches!(agent, AgentTarget::Claude | AgentTarget::All);
    let mut pi_provenance: Option<(PathBuf, PiProvenance)> = None;
    if pi_dual_home {
        if let Some(record_path) = pi_provenance_path() {
            let prov = load_pi_provenance_for_write(&record_path)?;
            pi_provenance = Some((record_path, prov));
        }
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
            // Companion resources install per layout (see `EmbeddedResource`):
            // claude gets them as plain siblings of the skill's `SKILL.md`;
            // codex, whose flat prompts dir surfaces every top-level `.md` as
            // a slash-command, gets them in a shared `_shared/` subdir with
            // the skill body's companion links rewritten to point there.
            match agent_name {
                "claude" => {
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
                            content: Cow::Borrowed(resource.body),
                            pi_companion_of: None,
                        });
                    }
                }
                "codex" => {
                    for resource in resources_for(skill.name) {
                        plan.push(PlanItem {
                            name: resource.filename,
                            agent: agent_name,
                            path: codex_companion_path(&path, resource.filename),
                            content: Cow::Borrowed(resource.body),
                            pi_companion_of: None,
                        });
                    }
                }
                _ => {}
            }
            let content = render_body_for_agent(agent_name, skill.body);
            plan.push(PlanItem {
                name: skill.name,
                agent: agent_name,
                path,
                content,
                pi_companion_of: None,
            });
        }

        // Dual-home into pi.dev's skill dir. Whenever the claude layout is
        // installed to its default path, mirror the SAME claude-format
        // `SKILL.md` into `~/.pi/agent/skills/<name>/SKILL.md` so the skill
        // is discoverable under the pi.dev harness (pi loads it and invokes
        // `/skill:name`; bare `/name` cross-references resolve via pi's
        // injected available-skills list, so no link rewrite is needed —
        // only the target). This is an ADDITIONAL target that never alters
        // the claude write.
        //
        // pi uses a PER-SKILL directory, exactly like claude (unlike codex's
        // flat prompts dir), so companion resources install as plain siblings
        // of the pi `SKILL.md` — byte-identical to the claude copy, with NO
        // link rewrite: the body's `](AGENTS-EXECUTION-DAG.md)` sibling link
        // (and `stint-handoff`'s cross-skill `](../stint-start/…)` link)
        // resolves against the mirrored sibling just as it does under claude.
        // Mirroring the companion is what keeps a skill that STOPS on a missing
        // companion (e.g. `stint-start`) from aborting under pi (issue
        // `support-pi-dev`). Skipped for a custom `--dest` (caller-managed
        // path) and for `--agent codex` alone (codex is not a claude-format
        // consumer; pi mirrors the claude corpus).
        //
        // The pi mirror is intentionally UNMANAGED in-tree: no `.orchestratectl-
        // managed` marker (the pi corpus stays a pure body mirror). Its
        // lifecycle — orphan prune + `doctor` drift for both the `SKILL.md`
        // and its companions — is keyed on the out-of-band provenance record
        // (`state/pi-installed-skills.json`); see the pi block after the write
        // loop and `PiSkillRecord`.
        if dest.is_none() && matches!(agent, AgentTarget::Claude | AgentTarget::All) {
            let pi_skill_path = default_path("pi", skill.name)?;
            for resource in resources_for(skill.name) {
                plan.push(PlanItem {
                    name: resource.filename,
                    agent: "pi",
                    path: sibling_path(&pi_skill_path, resource.filename),
                    content: Cow::Borrowed(resource.body),
                    pi_companion_of: Some(skill.name),
                });
            }
            plan.push(PlanItem {
                name: skill.name,
                agent: "pi",
                path: pi_skill_path,
                content: Cow::Borrowed(skill.body),
                pi_companion_of: None,
            });
        }
    }

    let preflight_result = preflight(&plan, force)?;

    // Combine caller-provided warnings (logging init, etc.) with
    // drift-detected ones so the success envelope surfaces both.
    let mut all_warnings: Vec<String> = warnings.to_vec();
    all_warnings.extend(preflight_result.warnings);

    let mut installed = Vec::with_capacity(plan.len());
    // pi files actually written this run. Only files the write loop persisted
    // are recorded in the provenance record below — a `skipped` (present,
    // non-force, divergent) pi file was NOT written, so we carry its prior
    // record forward untouched. A `SKILL.md` write and a companion write are
    // recorded distinctly so the companion lands under its owning skill.
    let mut pi_written: Vec<PiWrite> = Vec::new();
    for item in plan {
        // A pi mirror that preflight chose to leave in place (present, no
        // --force) is skipped outright — NOT written and NOT reported as
        // installed. Falling through to `write_atomic` here would call
        // `persist_noclobber`, hit `EEXIST`, and fail the whole install,
        // which is exactly the divergent-state repair-block F1 fixes.
        if preflight_result.skipped.contains(&item.path) {
            continue;
        }
        // The set of paths approved for overwrite is decided exclusively
        // by preflight — never recomputed from `path.exists()` in this
        // loop. That keeps the persist_noclobber TOCTOU guarantee intact:
        // a file that did not exist at preflight time will refuse to
        // overwrite, even if a concurrent process created it in the
        // window. (Review finding #1.)
        let allow_overwrite = preflight_result.overwrite_allowed.contains(&item.path);
        write_atomic(&item.path, &item.content, allow_overwrite)?;
        if item.agent == "pi" {
            let hash = sha256_hex(item.content.as_bytes());
            match item.pi_companion_of {
                None => {
                    let cli_version = parse_frontmatter_field(&item.content, "cli_version")
                        .unwrap_or_else(|| CLI_VERSION.to_string());
                    pi_written.push(PiWrite::Skill {
                        name: item.name,
                        hash,
                        cli_version,
                    });
                }
                Some(owner) => {
                    pi_written.push(PiWrite::Companion {
                        owner,
                        filename: item.name,
                        hash,
                    });
                }
            }
        }
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
    //
    // The marker also records the exact companion files this binary wrote
    // (`companion:` lines), so a later binary that drops a companion can
    // recognise the lingering file as an ORPHAN it once managed rather than
    // as a user's own note. Before rewriting the marker we read the copy the
    // prior binary left, diff its recorded companions against what this
    // binary bundles, and act on the leftover ORPHANS:
    //
    // - `--force`: remove the orphan companion file (the redeploy intends the
    //   on-disk catalog to mirror the binary) and drop it from the marker.
    // - non-`--force`: leave the file in place but CARRY IT FORWARD in the new
    //   marker, so `doctor` still recognises it as an orphan (and can suggest
    //   the `--force` fix). Rewriting the marker without it would forget the
    //   file forever while it lingers on disk.
    mark_dirs.sort();
    mark_dirs.dedup();
    let mut pruned_companions: Vec<String> = Vec::new();
    for (skill_name, dir) in &mark_dirs {
        let marker_path = dir.join(MANAGED_MARKER_FILENAME);
        let bundled: Vec<&'static str> = resources_for(skill_name)
            .iter()
            .map(|r| r.filename)
            .collect();
        // Start the new record with everything this binary bundles, then
        // reconcile the companions the prior marker recorded.
        let mut recorded: Vec<String> = bundled.iter().copied().map(String::from).collect();
        for prev in read_managed_companions(&marker_path) {
            if bundled.iter().any(|b| *b == prev) {
                continue; // still bundled — already in `recorded`
            }
            // A managed companion this binary no longer ships: an orphan.
            let orphan_path = dir.join(&prev);
            // Only ever touch a regular file we actually wrote — never follow a
            // symlink or recurse into a directory squatting at that name.
            let is_regular =
                fs::symlink_metadata(&orphan_path).is_ok_and(|m| m.file_type().is_file());
            if !is_regular {
                // Nothing safe to clean and nothing on disk to keep tracking:
                // drop it from the marker.
                continue;
            }
            if force {
                match fs::remove_file(&orphan_path) {
                    Ok(()) => {
                        all_warnings.push(format!(
                            "skill_companion_pruned: removed orphan companion '{prev}' for skill '{skill_name}' at {}",
                            orphan_path.display()
                        ));
                        pruned_companions.push(format!("{skill_name}/{prev}"));
                        // dropped from `recorded` → no longer tracked
                    }
                    Err(e) => {
                        all_warnings.push(format!(
                            "skill_companion_prune_failed: could not remove orphan companion '{prev}' for skill '{skill_name}' at {}: {e}",
                            orphan_path.display()
                        ));
                        recorded.push(prev); // still on disk — keep it tracked
                    }
                }
            } else {
                recorded.push(prev); // preserve tracking for doctor
            }
        }
        recorded.sort();
        recorded.dedup();
        write_marker(&marker_path, skill_name, &recorded)?;
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

    // Codex flat-layout provenance + prune. Codex's prompts dir is flat (no
    // per-skill directory), so a single shared marker at
    // `_shared/.orchestratectl-managed` records which prompts + companions
    // orchestratectl installed there — the only signal that makes codex-side
    // pruning + orphan detection safe. Maintained whenever the codex layout
    // is installed to its DEFAULT path (never for a caller-managed `--dest`).
    // The marker MERGES with any prior record (union) so a targeted
    // single-skill install never forgets the rest of the managed set. Orphan
    // pruning (removing a de-registered prompt/companion) is gated to the
    // full-catalog `--force` redeploy, symmetric with the claude dir prune
    // above; a plain `skill install` never deletes anything as a side effect.
    let codex_default = dest.is_none() && matches!(agent, AgentTarget::Codex | AgentTarget::All);
    if codex_default {
        if let (Some(prompts_root), Some(shared_root)) = (codex_prompts_root(), codex_shared_root())
        {
            let marker_path = shared_root.join(MANAGED_MARKER_FILENAME);

            // Everything this install just wrote to the codex layout: one
            // prompt per skill, plus every companion those skills bundle.
            let mut recorded_prompts: HashSet<String> =
                skills.iter().map(|s| s.name.to_string()).collect();
            let mut recorded_companions: HashSet<String> = skills
                .iter()
                .flat_map(|s| resources_for(s.name))
                .map(|r| r.filename.to_string())
                .collect();
            // Union with the prior marker so a targeted install (or a prune
            // that must recognise a de-registered entry) keeps the full set.
            recorded_prompts.extend(read_marker_records(&marker_path, "prompt"));
            recorded_companions.extend(read_marker_records(&marker_path, "companion"));

            let codex_prune_eligible = name.is_none() && force;
            if codex_prune_eligible {
                let registered: HashSet<&str> = SKILLS.iter().map(|s| s.name).collect();
                let bundled_companions: HashSet<&str> = SKILLS
                    .iter()
                    .flat_map(|s| resources_for(s.name))
                    .map(|r| r.filename)
                    .collect();

                // Orphan codex prompts: recorded but no longer in the catalog.
                let orphan_prompts: Vec<String> = recorded_prompts
                    .iter()
                    .filter(|p| !registered.contains(p.as_str()))
                    .cloned()
                    .collect();
                for orphan in orphan_prompts {
                    let prompt_path = prompts_root.join(format!("{orphan}.md"));
                    match prune_codex_file(
                        &prompt_path,
                        &format!("skill_pruned: removed de-registered managed codex prompt '{orphan}'"),
                        &format!("skill_prune_failed: could not remove de-registered codex prompt '{orphan}'"),
                        &mut all_warnings,
                    ) {
                        CodexPruneOutcome::Removed => {
                            pruned.push(orphan.clone());
                            recorded_prompts.remove(&orphan);
                        }
                        CodexPruneOutcome::Dropped => {
                            recorded_prompts.remove(&orphan);
                        }
                        CodexPruneOutcome::Kept => {}
                    }
                }

                // Orphan codex companions: recorded but no bundled skill ships
                // them any more (the last referrer was removed).
                let orphan_companions: Vec<String> = recorded_companions
                    .iter()
                    .filter(|c| !bundled_companions.contains(c.as_str()))
                    .cloned()
                    .collect();
                for orphan in orphan_companions {
                    let companion_path = shared_root.join(&orphan);
                    match prune_codex_file(
                        &companion_path,
                        &format!("skill_companion_pruned: removed orphan codex companion '_shared/{orphan}'"),
                        &format!("skill_companion_prune_failed: could not remove orphan codex companion '_shared/{orphan}'"),
                        &mut all_warnings,
                    ) {
                        CodexPruneOutcome::Removed => {
                            pruned_companions.push(format!("{CODEX_SHARED_SUBDIR}/{orphan}"));
                            recorded_companions.remove(&orphan);
                        }
                        CodexPruneOutcome::Dropped => {
                            recorded_companions.remove(&orphan);
                        }
                        CodexPruneOutcome::Kept => {}
                    }
                }
            }

            // Persist the reconciled marker. Create `_shared/` first: a codex
            // install of a companion-less skill would not otherwise materialise
            // it, but we still need the marker for later pruning of the flat
            // prompt file. A marker-write failure is fatal (like the claude
            // marker): without it a genuine orphan is never recognised.
            let mut prompts: Vec<String> = recorded_prompts.into_iter().collect();
            prompts.sort();
            let mut companions: Vec<String> = recorded_companions.into_iter().collect();
            companions.sort();
            fs::create_dir_all(&shared_root).map_err(|e| {
                CliError::system(
                    "create_dir_failed",
                    format!("could not create {}: {}", shared_root.display(), e),
                )
            })?;
            write_codex_marker(&marker_path, &prompts, &companions)?;
        }
    }

    // pi.dev mirror lifecycle (out-of-band provenance). Maintained whenever
    // this install dual-homed into pi — i.e. the same condition `cmd_install`
    // used to push the pi `PlanItem`s (default path, claude-format target).
    // Two steps, both keyed SOLELY on the out-of-band record (the pi dir has no
    // in-dir marker):
    //
    //   1. Record every pi mirror we just wrote (union-merged with the prior
    //      record so a targeted single-skill install never forgets the rest of
    //      the managed set — symmetric with the codex marker union).
    //   2. On the full-catalog `--force` redeploy (the same gate as the claude
    //      prune), prune pi mirrors of de-registered skills — but only ones the
    //      record names AND whose on-disk bytes still hash to the recorded value
    //      (strong evidence it is our unmodified copy). A user-taken-over or
    //      hand-authored pi dir is never recorded, so it is not touched.
    //
    // Like the claude/codex marker updates, the record read-modify-write is
    // unlocked: two concurrent `skill install` runs can lose one another's
    // additions (parity debt, not introduced here). Mutation commands are not
    // meant to run concurrently — see `crates/octl-cli/AGENTS.md`.
    if let Some((record_path, mut prov)) = pi_provenance {
        prov.schema_version = PI_PROVENANCE_SCHEMA_VERSION;
        // SKILL.md writes first (create/refresh the record IN PLACE, preserving
        // any companion sub-records already tracked), then companion writes file
        // under their owning skill. A companion whose owner was neither written
        // this run nor already recorded has no record to attach to and is
        // skipped (the rare hand-authored-pi-dir edge) rather than minting a
        // record with an empty `sha256`.
        for w in &pi_written {
            if let PiWrite::Skill {
                name,
                hash,
                cli_version,
            } = w
            {
                let rec = prov.skills.entry((*name).to_string()).or_default();
                rec.sha256.clone_from(hash);
                rec.cli_version.clone_from(cli_version);
            }
        }
        for w in &pi_written {
            if let PiWrite::Companion {
                owner,
                filename,
                hash,
            } = w
            {
                if let Some(rec) = prov.skills.get_mut(*owner) {
                    rec.companions.insert((*filename).to_string(), hash.clone());
                } else {
                    // The companion file was written but its owner skill has no
                    // record to attach to (owner `SKILL.md` skipped/absent AND
                    // never previously recorded — the rare hand-authored / reset
                    // pi-dir edge). Do NOT silently drop it: surface the untracked
                    // write so a `doctor`/operator can reconcile it, rather than
                    // leaving a file orchestratectl wrote with no provenance trail
                    // (review finding F4). Structural fix — flat per-file
                    // provenance — is tracked as `pi-provenance-flat-file-model`.
                    all_warnings.push(format!(
                        "pi_companion_unrecorded: wrote pi companion '{filename}' for skill '{owner}' but no provenance record exists for it; it will not be tracked or pruned (reinstall the skill with --force to record it)"
                    ));
                }
            }
        }

        // Reconcile each STILL-REGISTERED installed skill's recorded companions
        // against what this binary now bundles: a companion a prior binary
        // mirrored that the current one dropped is an orphan. Without this, the
        // `skill.orphan.<name>.pi.<file>` doctor check would flag a state its only
        // suggested fix (`skill install <name> --force`) could never clear — a
        // permanent unfixable warning loop (review finding F1). Mirrors the claude
        // `mark_dirs` companion reconciliation. De-registered skills are handled
        // by the prune block below; this handles skills that survive but shed a
        // companion.
        for skill in &skills {
            reconcile_pi_companions(
                skill.name,
                &mut prov,
                force,
                &mut pruned_companions,
                &mut all_warnings,
            );
        }

        if prune_eligible {
            let registered: HashSet<&str> = SKILLS.iter().map(|s| s.name).collect();
            // De-registered names the record still tracks — the only prune
            // candidates. Collected first (from `BTreeMap::keys`, so sorted +
            // deterministic) so we don't mutate `prov.skills` while iterating it.
            // The registered check is case-insensitive as well as exact, symmetric
            // with `managed_orphan_dirs`: on a case-insensitive filesystem (APFS) a
            // corrupt record key that is a case variant of a registered skill would
            // otherwise resolve to that skill's live dir and, if the hash matched,
            // delete a registered mirror (review finding F5).
            let orphan_names: Vec<String> = prov
                .skills
                .keys()
                .filter(|n| {
                    !registered.contains(n.as_str())
                        && !registered.iter().any(|r| r.eq_ignore_ascii_case(n))
                })
                .cloned()
                .collect();
            for orphan in orphan_names {
                // Never let a record-sourced key that is not a single normal path
                // component reach the filesystem (review finding E). It stays in
                // the record (inert — doctor skips it too) but is never acted on.
                if !is_simple_skill_name(&orphan) {
                    all_warnings.push(format!(
                        "pi_provenance_bad_name: ignoring pi provenance entry '{orphan}' (not a simple skill name)"
                    ));
                    continue;
                }
                // `orphan` came straight from `prov.skills.keys()`, so the entry
                // is present — index directly rather than a fallible get that
                // could silently pass an empty hash to the prune.
                let record = prov.skills[&orphan].clone();
                match prune_pi_mirror(
                    &orphan,
                    &record.sha256,
                    &record.companions,
                    &mut all_warnings,
                ) {
                    PiPruneOutcome::Removed => {
                        pruned.push(orphan.clone());
                        prov.skills.remove(&orphan);
                    }
                    // Stop tracking it — either nothing safe is on disk to
                    // delete (absent / symlink / dir), or the on-disk copy
                    // diverged from what we wrote (the user has taken it over,
                    // so we leave it in place and relinquish management rather
                    // than delete a file we no longer recognise as ours).
                    PiPruneOutcome::Dropped | PiPruneOutcome::Diverged => {
                        prov.skills.remove(&orphan);
                    }
                    // Delete failed (still on disk): keep tracking so a later
                    // redeploy retries and `doctor` still flags it.
                    PiPruneOutcome::Kept => {}
                }
            }
        }

        write_pi_provenance(&record_path, &prov)?;
    }

    // De-registered skills can be pruned from more than one layout (claude,
    // codex, pi) in a single `--force` redeploy, each pushing the same name here.
    // Deduplicate so the `pruned` payload is a set (review finding B).
    pruned.sort();
    pruned.dedup();

    let payload = InstallPayload {
        installed,
        pruned,
        pruned_companions,
    };
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
            for entry in &payload.pruned_companions {
                println!("pruned {entry} (orphan companion)");
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
///
/// `skipped` is the set of already-present pi-mirror paths a non-`--force`
/// run must leave untouched (see `preflight`'s pi arm). The write loop
/// skips them entirely — it must NOT fall through to `write_atomic`, whose
/// `persist_noclobber` would hit `EEXIST` and fail the whole install. A
/// skipped path is never reported as `installed`.
struct PreflightResult {
    warnings: Vec<String>,
    overwrite_allowed: HashSet<PathBuf>,
    skipped: HashSet<PathBuf>,
}

/// One file the install will write: a skill's `SKILL.md` or one of its
/// companion resources. `name` is what the install payload and drift
/// warnings report (the skill name, or the resource filename).
struct PlanItem {
    name: &'static str,
    agent: &'static str,
    path: PathBuf,
    /// Bytes to write. Borrowed for the claude layout and companions (the
    /// embedded source verbatim); owned when a codex body needed companion
    /// links rewritten (see `render_body_for_agent`).
    content: Cow<'static, str>,
    /// For a pi companion mirror only: the owning skill's name, so the
    /// out-of-band provenance record files the companion hash under its skill.
    /// `None` for every `SKILL.md` item and every non-pi item (claude/codex
    /// companions are tracked by their own in-tree markers, not this field).
    pi_companion_of: Option<&'static str>,
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
    let mut skipped: HashSet<PathBuf> = HashSet::new();
    for PlanItem {
        name,
        agent,
        path,
        content,
        ..
    } in plan
    {
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
        // The pi mirror is a DERIVED copy of the claude SKILL.md, never a
        // first-class target the user asked for. It must not let its own
        // state gate the primary claude install (issue
        // `pidev-dual-home-skills`, review finding F1): a present-and-current
        // pi copy must NOT block re-creating a deleted claude skill, and an
        // unmanaged pi file (no provenance marker) must NOT be clobbered on a
        // plain run merely because it looks older. So:
        //   - absent            → fall through to the normal write path.
        //   - present, --force  → refresh it (overwrite_allowed).
        //   - present, no force → leave it in place (skipped); warn only when
        //                         the on-disk bytes actually differ, so a
        //                         byte-identical mirror is a silent no-op.
        // Trade-off: a non-force claude drift-upgrade leaves a stale pi copy
        // until the next `--force` redeploy — acceptable because the
        // operating policy always deploys with `--force`. Lifecycle
        // (prune + doctor drift) is tracked separately as
        // `pidev-pi-skill-lifecycle`.
        if *agent == "pi" && path.exists() {
            if force {
                overwrite_allowed.insert(path.clone());
            } else {
                // Warn unless the on-disk bytes are provably identical to
                // the bundled copy — an unreadable file counts as "differs"
                // so the skip is surfaced rather than silently assumed a
                // no-op.
                if !fs::read(path).is_ok_and(|b| b == content.as_bytes()) {
                    warnings.push(format!(
                        "pi_mirror_skipped: {name} already exists at {} and differs from the bundled copy; left unchanged (pass --force to refresh)",
                        path.display()
                    ));
                }
                skipped.insert(path.clone());
            }
            continue;
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
        skipped,
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
        // pi.dev discovers skills from a per-skill directory just like
        // claude, only rooted at `~/.pi/agent/skills/`, and invokes them
        // as `/skill:name`. The dual-home mirror writes the same
        // claude-format `SKILL.md` here (see `cmd_install`).
        "pi" => base.join(".pi/agent/skills").join(name).join("SKILL.md"),
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

/// Root of the codex flat prompts layout (`~/.codex/prompts`), where each
/// skill installs as a single top-level `<name>.md`. `None` when `HOME` is
/// unset. Both the codex prune path and the `skill.orphan.codex.*` doctor
/// check resolve prompt files against this root.
pub fn codex_prompts_root() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".codex/prompts"))
}

/// The codex shared-companion dir (`~/.codex/prompts/_shared`). `None` when
/// `HOME` is unset. Holds the `_shared/<file>` companions AND the single
/// codex provenance marker.
pub fn codex_shared_root() -> Option<PathBuf> {
    codex_prompts_root().map(|p| p.join(CODEX_SHARED_SUBDIR))
}

/// Path to the single codex provenance marker
/// (`~/.codex/prompts/_shared/.orchestratectl-managed`). `None` when `HOME`
/// is unset. Codex's prompts dir is flat, so — unlike claude's per-skill
/// directory marker — ONE shared marker records every prompt + companion
/// orchestratectl installed there. Its presence is what makes codex-side
/// pruning + orphan detection safe: a user's own prompt of the same name is
/// never recorded, so it is never touched.
fn codex_marker_path() -> Option<PathBuf> {
    codex_shared_root().map(|p| p.join(MANAGED_MARKER_FILENAME))
}

/// Codex prompt names the shared provenance marker records as
/// orchestratectl-managed (sorted, deduped). Empty when `HOME` is unset or
/// the marker is absent/unreadable — which is precisely the signal that
/// orchestratectl does not manage codex on this host (e.g. a claude-only
/// install), so `doctor` emits no codex checks and a claude-primary tree
/// stays 0-warn.
pub fn codex_managed_prompts() -> Vec<String> {
    let Some(marker) = codex_marker_path() else {
        return Vec::new();
    };
    let mut v = read_marker_records(&marker, "prompt");
    v.sort();
    v.dedup();
    v
}

/// Companion filenames the shared codex provenance marker records as
/// managed — the `_shared/<file>` companions orchestratectl installed
/// (sorted, deduped). Empty under the same conditions as
/// [`codex_managed_prompts`].
pub fn codex_managed_companions() -> Vec<String> {
    let Some(marker) = codex_marker_path() else {
        return Vec::new();
    };
    let mut v = read_marker_records(&marker, "companion");
    v.sort();
    v.dedup();
    v
}

// ---------------------------------------------------------------------------
// pi.dev mirror lifecycle (out-of-band provenance).
//
// The pi mirror (`~/.pi/agent/skills/<name>/SKILL.md`, written by `cmd_install`)
// deliberately carries NO in-dir `.orchestratectl-managed` marker — the
// `pidev-dual-home-skills` contract forbids one so the pi corpus stays a pure
// skill-body mirror. Without an in-dir provenance signal we cannot tell an
// orchestratectl-written pi dir from a user's own hand-authored pi skill, so a
// naive "pi orphan" prune/warn would false-positive on every user skill.
//
// The provenance therefore lives OUT-OF-BAND, in a single JSON record under the
// orchestratectl state root (`<root>/state/pi-installed-skills.json`), keyed by
// skill name → the content hash + `cli_version` we last wrote. It is the SOLE
// authority for two safety-critical decisions:
//
//   - prune (issue task 2): a pi mirror is a prune candidate only if its name is
//     recorded here AND the on-disk bytes still hash to the recorded value (proof
//     it is our unmodified copy). A user-taken-over or hand-authored pi dir is
//     never recorded, so it is never touched.
//   - `doctor` drift (issue task 3): the recorded set gates the `skill.sync.
//     <name>.pi` / `skill.orphan.<name>.pi` checks, so a host that never dual-
//     homed into pi emits no pi checks and stays 0-warn.

/// Schema version of the pi provenance record. Bumped independently of the
/// SKILL.md and envelope schema versions if the record's shape ever changes.
///
/// **v2** added the per-skill `companions` map. The bump is deliberate even
/// though the field is `serde(default)` (so this binary reads a v1 record
/// fine): keeping it v1 would let an OLDER binary read the new record, silently
/// drop the unknown `companions` field on its next rewrite, and — because it
/// still saw schema 1 — accept and overwrite it, erasing companion tracking for
/// every mirror. Writing v2 makes that older binary reject the record via
/// `load_pi_provenance_for_write`'s `schema_too_new` guard (fail closed) instead
/// of laundering the field away. The `<=` load check keeps old v1 records
/// readable here.
const PI_PROVENANCE_SCHEMA_VERSION: u32 = 2;

/// One pi mirror orchestratectl wrote: the content hash + `cli_version` of the
/// `SKILL.md` bytes last persisted to `~/.pi/agent/skills/<name>/SKILL.md`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
struct PiSkillRecord {
    /// Lowercase-hex SHA-256 of the `SKILL.md` bytes we wrote. Divergence from
    /// the on-disk bytes means the user (or a newer/older binary) has since
    /// changed the file — the prune path refuses to delete such a copy.
    sha256: String,
    /// The `cli_version` frontmatter of the bytes we wrote, retained for
    /// human/debug inspection of the record and possible future use. `doctor`
    /// classifies drift from the ON-DISK `cli_version` (more accurate than the
    /// last-written one), so it does not read this field today.
    cli_version: String,
    /// Companion resources mirrored beside this skill's pi `SKILL.md`
    /// (`~/.pi/agent/skills/<name>/<file>`), keyed filename → lowercase-hex
    /// SHA-256 of the bytes we wrote. Empty for skills that ship no companion.
    /// Lets the prune path clean a de-registered skill's companions (verifying
    /// each is still our unmodified copy) and `doctor` recognise a dropped
    /// companion as an orphan — symmetric with the claude marker's `companion:`
    /// records. `serde(default)` keeps a v1 record (no companion tracking)
    /// readable; the schema is bumped to v2 on write so an older binary refuses
    /// it rather than silently dropping this field (see
    /// `PI_PROVENANCE_SCHEMA_VERSION`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    companions: BTreeMap<String, String>,
}

/// The out-of-band pi provenance record: which pi mirrors orchestratectl wrote
/// and their content hash + version. A `BTreeMap` keeps the on-disk JSON
/// deterministic (sorted keys) so a redeploy that changes nothing produces
/// byte-identical output.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct PiProvenance {
    schema_version: u32,
    skills: BTreeMap<String, PiSkillRecord>,
}

/// Lowercase-hex SHA-256 of `bytes`. Used to fingerprint a pi `SKILL.md` for
/// the provenance record and to verify a prune candidate is still our copy.
fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Root of the pi.dev skill-mirror layout (`~/.pi/agent/skills`). `None` when
/// `HOME` is unset. Sibling of [`claude_skills_root`]; the pi prune uses it to
/// assert a per-skill dir it is about to `remove_dir` really sits directly under
/// the corpus root before touching it.
fn pi_skills_root() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".pi/agent/skills"))
}

/// Default pi mirror path for a skill (`~/.pi/agent/skills/<name>/SKILL.md`).
/// `None` when `HOME` is unset. Used by `doctor` to locate the on-disk pi copy
/// to compare against the binary, and by the prune path to resolve a
/// de-registered mirror.
pub fn pi_default_path(name: &str) -> Option<PathBuf> {
    default_path("pi", name).ok()
}

/// True when `name` is a single normal path component — no `/`, no `.`/`..`, not
/// absolute, not empty. Every real catalog skill name has this shape. The
/// provenance record is persisted, mutable JSON: a corrupt/hand-edited key like
/// `"../../.bashrc"` or an absolute path deserialized and then `join`ed into a
/// filesystem path would let the prune/doctor act OUTSIDE the pi corpus (the one
/// `record-key → fs-path → remove_file` path that has no other binding check —
/// claude/codex instead enumerate real directory entries, which are
/// single-component for free). Validating here gives the pi path the same rigor
/// as `managed_orphan_dirs` (review finding E). Non-matching names are skipped,
/// never acted on.
///
/// Reused for record-sourced companion FILENAMES too (e.g. `AGENTS-EXECUTION-
/// DAG.md`): the contract is the same "single normal path component", so the
/// name reads skill-specific but the check is exactly what a companion filename
/// needs before it is joined into a per-skill dir.
pub fn is_simple_skill_name(name: &str) -> bool {
    let mut components = Path::new(name).components();
    matches!(
        (components.next(), components.next()),
        (Some(std::path::Component::Normal(_)), None)
    )
}

/// Path to the single out-of-band pi provenance record
/// (`<orchestratectl-root>/state/pi-installed-skills.json`). `None` when the
/// root cannot be resolved (neither `$ORCHESTRATECTL_HOME` nor `$HOME` set).
/// Deliberately rooted at the orchestratectl STATE dir, not `~/.pi` — the pi
/// corpus must stay a pure body mirror with no orchestratectl bookkeeping in it.
fn pi_provenance_path() -> Option<PathBuf> {
    home::root_dir()
        .ok()
        .map(|root| root.join("state").join("pi-installed-skills.json"))
}

/// LENIENT read for the READ-ONLY doctor path: a missing, unreadable,
/// unparseable, OR future-schema record yields an empty [`PiProvenance`], so
/// `doctor` simply emits no pi checks rather than auditing a record it cannot
/// trust. NEVER use this on the install mutation path — that must fail loudly
/// instead of laundering a corrupt record into an empty one it then overwrites
/// (see [`load_pi_provenance_for_write`]).
fn read_pi_provenance(path: &Path) -> PiProvenance {
    let Ok(body) = fs::read_to_string(path) else {
        return PiProvenance::default();
    };
    match serde_json::from_str::<PiProvenance>(&body) {
        Ok(p) if p.schema_version <= PI_PROVENANCE_SCHEMA_VERSION => p,
        _ => PiProvenance::default(),
    }
}

/// STRICT load for the install MUTATION path. Distinguishes:
///
///   - absent (`NotFound`) → `Ok(default)` — a first install starts fresh.
///   - unreadable / unparseable / `schema_version` NEWER than this binary
///     understands → `Err`.
///
/// so an install NEVER silently launders a corrupt or future-schema record into
/// an empty one and then overwrites it — which would erase tracking for EVERY
/// other managed pi mirror (the record is the sole authority; there is no in-dir
/// fallback). The record is loaded and validated BEFORE any file is written, so
/// a corrupt record fails the install fast rather than after mutating the tree
/// (review finding A). The trusted state root does not rescue this: a partial
/// write (power loss / ENOSPC mid-persist), a version rollback, or a manual edit
/// are all ordinary causes.
fn load_pi_provenance_for_write(path: &Path) -> Result<PiProvenance, CliError> {
    let body = match fs::read_to_string(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(PiProvenance::default()),
        Err(e) => {
            return Err(CliError::system(
                "pi_provenance_unreadable",
                format!(
                    "could not read pi provenance record {}: {e}; back it up and remove it to re-initialise",
                    path.display()
                ),
            ))
        }
    };
    let prov: PiProvenance = serde_json::from_str(&body).map_err(|e| {
        CliError::system(
            "pi_provenance_corrupt",
            format!(
                "pi provenance record {} is not valid JSON ({e}); refusing to overwrite it (that would erase tracking for every managed pi mirror). Back it up and remove it to re-initialise.",
                path.display()
            ),
        )
    })?;
    if prov.schema_version > PI_PROVENANCE_SCHEMA_VERSION {
        return Err(CliError::system(
            "pi_provenance_schema_too_new",
            format!(
                "pi provenance record {} uses schema {} but this binary understands only {}; refusing to overwrite it. Upgrade orchestratectl.",
                path.display(),
                prov.schema_version,
                PI_PROVENANCE_SCHEMA_VERSION
            ),
        ));
    }
    Ok(prov)
}

/// Write the pi provenance record atomically. The `state/` dir is created if
/// absent. `write_atomic` with `force = true` renames a fresh tempfile over the
/// destination, which atomically replaces whatever name is there — including a
/// squatting symlink — with the regular file, so no symlink target is ever
/// clobbered. A write failure is fatal to the install (symmetric with the
/// claude/codex marker writes): without a current record a genuine pi orphan is
/// never recognised.
fn write_pi_provenance(path: &Path, prov: &PiProvenance) -> Result<(), CliError> {
    let body = serde_json::to_string_pretty(prov).map_err(|e| {
        CliError::system(
            "pi_provenance_serialize_failed",
            format!("could not serialize pi provenance record: {e}"),
        )
    })?;
    write_atomic(path, &body, true)
}

/// The skill names the pi provenance record lists as orchestratectl-managed,
/// each with its recorded content hash (sorted by name). Empty
/// when `HOME`/root is unset or the record is absent — precisely the signal
/// that orchestratectl does not manage a pi mirror on this host, so `doctor`
/// emits no pi checks. Consumed by the `skill.sync.<name>.pi` /
/// `skill.orphan.<name>.pi` doctor checks.
pub fn pi_managed_skills() -> Vec<PiManagedSkill> {
    let Some(path) = pi_provenance_path() else {
        return Vec::new();
    };
    let prov = read_pi_provenance(&path);
    let mut out: Vec<PiManagedSkill> = prov
        .skills
        .into_iter()
        .map(|(name, rec)| {
            let mut companions: Vec<String> = rec.companions.into_keys().collect();
            companions.sort();
            PiManagedSkill {
                name,
                sha256: rec.sha256,
                companions,
            }
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// One managed pi mirror surfaced to `doctor`: the skill name plus the content
/// hash orchestratectl recorded when it last wrote the mirror. The hash lets the
/// drift check detect a same-version local edit (on-disk bytes no longer match
/// what we wrote) without holding the bundled body. (The record also carries the
/// written `cli_version`, but the doctor reads the on-disk `cli_version` for a
/// more accurate drift classification, so it is not surfaced here.)
pub struct PiManagedSkill {
    pub name: String,
    pub sha256: String,
    /// Companion filenames the provenance record lists as mirrored beside this
    /// skill's pi `SKILL.md` (sorted). Used by `doctor` to detect a companion
    /// the binary dropped but the record still tracks (`skill.orphan.<name>.pi.
    /// <file>`); the forward drift check compares on-disk companions to the
    /// bundled bodies directly, so it does not need this list.
    pub companions: Vec<String>,
}

/// SHA-256 (lowercase hex) of the file at `path`, or `None` if it cannot be
/// read. Exposed for `doctor` to compare an on-disk pi `SKILL.md` against the
/// hash the provenance record holds.
pub fn file_sha256(path: &Path) -> Option<String> {
    fs::read(path).ok().map(|b| sha256_hex(&b))
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
/// `companions` are the companion filenames this install manages in the
/// directory, each recorded on its own `companion:` line so a later binary
/// that drops one can recognise the leftover file as an orphan it once
/// installed (see `orphan_companions` and the `cmd_install` prune loop).
/// The parent directory already exists (the SKILL.md write created it), so
/// this is a plain overwrite; the marker is always ours. If a symlink is
/// squatting at the marker path we unlink it first so `fs::write` cannot
/// clobber the link's target (an arbitrary-file overwrite within the
/// user's permissions).
fn write_marker(path: &Path, skill_name: &str, companions: &[String]) -> Result<(), CliError> {
    clear_marker_symlink(path)?;
    let mut body = format!(
        "managed-by: orchestratectl\ncli_version: {CLI_VERSION}\nskill_name: {skill_name}\n"
    );
    for companion in companions {
        body.push_str("companion: ");
        body.push_str(companion);
        body.push('\n');
    }
    fs::write(path, body).map_err(|e| {
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

/// Write (or overwrite) the SINGLE codex provenance marker at `path`
/// (`~/.codex/prompts/_shared/.orchestratectl-managed`). Unlike the claude
/// per-skill marker, this one records the flat layout's whole managed set:
/// a `prompt:` line per installed codex skill and a `companion:` line per
/// installed `_shared/<file>`. That is the only signal that makes codex
/// pruning safe (a user's own prompt is never listed). A squatting symlink
/// is unlinked first so `fs::write` cannot clobber the link's target.
fn write_codex_marker(
    path: &Path,
    prompts: &[String],
    companions: &[String],
) -> Result<(), CliError> {
    clear_marker_symlink(path)?;
    let mut body = format!("managed-by: orchestratectl\ncli_version: {CLI_VERSION}\n");
    for prompt in prompts {
        body.push_str("prompt: ");
        body.push_str(prompt);
        body.push('\n');
    }
    for companion in companions {
        body.push_str("companion: ");
        body.push_str(companion);
        body.push('\n');
    }
    fs::write(path, body).map_err(|e| {
        CliError::system(
            "marker_write_failed",
            format!(
                "could not write codex provenance marker {}: {}",
                path.display(),
                e
            ),
        )
    })
}

/// If a symlink squats at the marker `path`, unlink it so a subsequent
/// `fs::write` cannot clobber the link's target (an arbitrary-file
/// overwrite within the user's permissions). A regular file is left for the
/// write to overwrite in place — the marker is always ours.
fn clear_marker_symlink(path: &Path) -> Result<(), CliError> {
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
    Ok(())
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

/// What [`prune_codex_file`] did with an orphan file, so the caller can
/// keep the marker's recorded set in step: `Removed` (deleted → drop it +
/// report it pruned), `Dropped` (nothing safe on disk to delete → stop
/// tracking it), or `Kept` (delete failed → still on disk → keep tracking).
enum CodexPruneOutcome {
    Removed,
    Dropped,
    Kept,
}

/// Delete one orphaned codex file (a de-registered prompt or companion),
/// mirroring the claude orphan-companion prune's safety: only ever remove a
/// REGULAR file we manage — never follow a symlink or recurse into a
/// directory squatting at that name. An absent/symlink/dir target yields
/// `Dropped` (nothing to clean, so stop tracking it); a successful unlink
/// yields `Removed`; a failed unlink warns and yields `Kept` so the marker
/// keeps tracking the still-present file.
fn prune_codex_file(
    path: &Path,
    removed_warning: &str,
    failed_warning: &str,
    warnings: &mut Vec<String>,
) -> CodexPruneOutcome {
    let is_regular = fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_file());
    if !is_regular {
        return CodexPruneOutcome::Dropped;
    }
    match fs::remove_file(path) {
        Ok(()) => {
            warnings.push(format!("{removed_warning} at {}", path.display()));
            CodexPruneOutcome::Removed
        }
        Err(e) => {
            warnings.push(format!("{failed_warning} at {}: {e}", path.display()));
            CodexPruneOutcome::Kept
        }
    }
}

/// One pi file the install's write loop actually persisted this run, tagged so
/// the provenance update files it correctly: a `SKILL.md` (recorded under the
/// skill name) or a companion (recorded in its owning skill's `companions` map).
/// Only persisted files appear here — a `skipped` (present, non-force) pi file
/// is absent, so its prior record is carried forward untouched.
enum PiWrite {
    Skill {
        name: &'static str,
        hash: String,
        cli_version: String,
    },
    Companion {
        owner: &'static str,
        filename: &'static str,
        hash: String,
    },
}

/// What [`prune_pi_mirror`] did with a de-registered pi mirror, so the caller
/// can keep the provenance record in step: `Removed` (our unmodified copy,
/// deleted → drop it + report it pruned), `Dropped` (nothing safe on disk to
/// delete → stop tracking), `Diverged` (on-disk bytes no longer match what we
/// wrote → the user owns it now, leave it + stop tracking), or `Kept` (delete
/// failed → still on disk → keep tracking so a later redeploy retries).
enum PiPruneOutcome {
    Removed,
    Dropped,
    Diverged,
    Kept,
}

/// Prune the pi mirror for a de-registered skill `name`, keyed SOLELY on the
/// out-of-band provenance record (the pi dir carries no in-dir marker). Safety
/// is layered exactly like the claude/codex orphan prunes:
///
///   - Resolve `~/.pi/agent/skills/<name>/SKILL.md`; a `HOME`-unset root yields
///     `Dropped` (nothing we can locate).
///   - `symlink_metadata` (never follows a link): only a REGULAR file is a
///     candidate — a symlink or a directory squatting at that name is left
///     untouched (`Dropped`), so a planted link can never redirect the delete.
///   - Content identity: the on-disk bytes must hash to `recorded_hash`. A
///     mismatch means the user (or another binary) changed the file since we
///     wrote it — we refuse to delete it and relinquish management
///     (`Diverged`), leaving the whole dir (companions included) to the user.
///   - Then remove each recorded companion sibling that is still our unmodified
///     copy (same regular-file + hash-match guards) FIRST, so a de-registered
///     skill's `AGENTS-EXECUTION-DAG.md`-style companion does not linger and
///     block the empty-dir cleanup; a diverged or user companion is left in
///     place. If ANY companion delete fails while its file is still present, the
///     whole prune returns `Kept` WITHOUT removing the `SKILL.md`, so the next
///     redeploy retries the entire prune from a consistent state rather than
///     stranding companions behind an already-deleted body (review finding F3).
///   - Only then remove the `SKILL.md` we wrote, and best-effort remove the now-
///     empty per-skill dir (never a recursive `remove_dir_all`, since the dir
///     may hold a user file we did not create). Companions are cleaned even when
///     the `SKILL.md` is already absent (a prior partial prune), so they are
///     never stranded.
fn prune_pi_mirror(
    name: &str,
    recorded_hash: &str,
    companions: &BTreeMap<String, String>,
    warnings: &mut Vec<String>,
) -> PiPruneOutcome {
    let Some(path) = pi_default_path(name) else {
        return PiPruneOutcome::Dropped;
    };
    prune_pi_mirror_at(
        name,
        &path,
        pi_skills_root().as_deref(),
        recorded_hash,
        companions,
        warnings,
    )
}

/// Path-taking core of [`prune_pi_mirror`], split out so the safety logic is
/// unit-testable against a tempdir without touching `$HOME`. `skills_root` is
/// the expected pi corpus root; the empty-dir cleanup only fires when the
/// mirror's parent is exactly `<skills_root>/<name>/` (see the caller's doc for
/// the layered safety contract).
fn prune_pi_mirror_at(
    name: &str,
    path: &Path,
    skills_root: Option<&Path>,
    recorded_hash: &str,
    companions: &BTreeMap<String, String>,
    warnings: &mut Vec<String>,
) -> PiPruneOutcome {
    let is_regular = fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_file());

    // Verify the `SKILL.md` is our unmodified copy BEFORE mutating anything. A
    // diverged body means the user owns the dir now — leave everything, including
    // companions, and relinquish tracking. An unreadable regular body defers the
    // whole prune (`Kept`) for a retry.
    if is_regular {
        match fs::read(path) {
            Ok(bytes) => {
                if sha256_hex(&bytes) != recorded_hash {
                    warnings.push(format!(
                        "pi_mirror_diverged: de-registered pi mirror '{name}' at {} was modified since orchestratectl wrote it; leaving it in place and no longer tracking it",
                        path.display()
                    ));
                    return PiPruneOutcome::Diverged;
                }
            }
            Err(e) => {
                warnings.push(format!(
                    "pi_mirror_prune_failed: could not read de-registered pi mirror '{name}' at {}: {e}",
                    path.display()
                ));
                return PiPruneOutcome::Kept;
            }
        }
    }

    // Prune recorded companions FIRST (before the `SKILL.md`), so a failure or
    // crash here leaves the body in place and the next redeploy retries the whole
    // prune rather than seeing an absent body, returning early, and stranding the
    // companions untracked. The filename is validated as a single normal path
    // component before it reaches `join` → `remove_file`, so a corrupt record key
    // can never escape the per-skill dir.
    let mut any_companion_kept = false;
    if let Some(dir) = path.parent() {
        for (filename, chash) in companions {
            if !is_simple_skill_name(filename) {
                warnings.push(format!(
                    "pi_provenance_bad_name: ignoring pi companion entry '{filename}' for skill '{name}' (not a simple filename)"
                ));
                continue;
            }
            if matches!(
                prune_pi_companion(name, &dir.join(filename), chash, warnings),
                PiCompanionOutcome::Kept
            ) {
                any_companion_kept = true;
            }
        }
    }
    // A companion whose delete failed while still on disk keeps the whole record:
    // do not delete the body yet, so a later redeploy retries and `doctor` keeps
    // flagging the leftover through the still-present record.
    if any_companion_kept {
        return PiPruneOutcome::Kept;
    }

    // The body. If it is not our regular file (absent / symlink / squatting dir),
    // there is nothing safe to delete for the body — companions above are handled,
    // so best-effort clean the now-possibly-empty dir and drop tracking.
    if !is_regular {
        remove_empty_pi_skill_dir(path.parent(), skills_root, name);
        return PiPruneOutcome::Dropped;
    }
    match fs::remove_file(path) {
        Ok(()) => {
            remove_empty_pi_skill_dir(path.parent(), skills_root, name);
            warnings.push(format!(
                "pi_mirror_pruned: removed de-registered pi mirror '{name}' at {}",
                path.display()
            ));
            PiPruneOutcome::Removed
        }
        Err(e) => {
            warnings.push(format!(
                "pi_mirror_prune_failed: could not remove de-registered pi mirror '{name}' at {}: {e}",
                path.display()
            ));
            PiPruneOutcome::Kept
        }
    }
}

/// Best-effort removal of a now-orphaned per-skill pi dir if it is empty. We only
/// ever wrote `SKILL.md` + our companions into it, but a user may have added
/// their own sibling (or a diverged companion may remain) — `remove_dir`
/// (non-recursive) fails on a non-empty dir, so anything we did not clean is
/// preserved and the dir left be. Guarded on `parent` sitting DIRECTLY under the
/// pi skills root (`<root>/<name>/`), so even an unexpected `pi_default_path`
/// result can never point `remove_dir` at an arbitrary directory — the pi
/// analogue of claude's `is_managed_skill_dir` name-binding (review finding D).
fn remove_empty_pi_skill_dir(parent: Option<&Path>, skills_root: Option<&Path>, name: &str) {
    if let (Some(parent), Some(root)) = (parent, skills_root) {
        if parent.parent() == Some(root)
            && parent.file_name().and_then(|n| n.to_str()) == Some(name)
        {
            let _ = fs::remove_dir(parent);
        }
    }
}

/// What [`prune_pi_companion`] did with ONE recorded companion sibling, so the
/// caller can decide whether the enclosing prune must defer: `Removed` (our
/// unmodified copy, deleted), `Absent` (nothing on disk), `NonRegular` (a
/// symlink / squatting dir left in place), `Diverged` (a user-edited copy left
/// in place), or `Kept` (delete failed while the file is still present → the
/// caller must NOT delete the body yet, so a later redeploy retries).
#[derive(PartialEq, Eq)]
enum PiCompanionOutcome {
    Removed,
    Absent,
    NonRegular,
    Diverged,
    Kept,
}

/// Best-effort removal of ONE recorded pi companion sibling during a
/// de-registered skill's prune. Mirrors the SKILL.md safety: only a REGULAR
/// file whose bytes hash to `recorded_hash` is deleted — a symlink, a squatting
/// dir, an unreadable file, or a user-edited copy is left untouched (and thus
/// keeps the parent dir non-empty so `remove_dir` spares it). Warnings narrate
/// every case where a file is LEFT behind (a squatting non-regular path, a
/// diverged copy, or a failed delete) so a leftover is visible; a plain-absent
/// companion is silent (nothing to narrate). Returns the outcome so the caller
/// can defer the body delete on `Kept`.
fn prune_pi_companion(
    skill: &str,
    path: &Path,
    recorded_hash: &str,
    warnings: &mut Vec<String>,
) -> PiCompanionOutcome {
    match fs::symlink_metadata(path) {
        Err(_) => return PiCompanionOutcome::Absent,
        Ok(m) if !m.file_type().is_file() => {
            warnings.push(format!(
                "pi_companion_left: companion of de-registered pi skill '{skill}' at {} is not a regular file (symlink or directory); leaving it in place",
                path.display()
            ));
            return PiCompanionOutcome::NonRegular;
        }
        Ok(_) => {}
    }
    match fs::read(path) {
        Ok(bytes) if sha256_hex(&bytes) == recorded_hash => match fs::remove_file(path) {
            Ok(()) => {
                warnings.push(format!(
                    "pi_companion_pruned: removed companion of de-registered pi skill '{skill}' at {}",
                    path.display()
                ));
                PiCompanionOutcome::Removed
            }
            Err(e) => {
                warnings.push(format!(
                    "pi_companion_prune_failed: could not remove companion of de-registered pi skill '{skill}' at {}: {e}",
                    path.display()
                ));
                PiCompanionOutcome::Kept
            }
        },
        Ok(_) => {
            warnings.push(format!(
                "pi_companion_diverged: companion of de-registered pi skill '{skill}' at {} was modified since orchestratectl wrote it; leaving it in place",
                path.display()
            ));
            PiCompanionOutcome::Diverged
        }
        Err(e) => {
            warnings.push(format!(
                "pi_companion_prune_failed: could not read companion of de-registered pi skill '{skill}' at {}: {e}",
                path.display()
            ));
            PiCompanionOutcome::Kept
        }
    }
}

/// Reconcile ONE still-registered skill's recorded pi companions against what
/// this binary now bundles: a companion a prior binary mirrored that the current
/// one no longer ships is an orphan. Without this, the `skill.orphan.<name>.pi.
/// <file>` doctor check would flag a state its only suggested fix (`skill install
/// <name> --force`) could never clear — a permanent, unfixable warning loop
/// (review finding F1). Symmetric with the claude `mark_dirs` companion
/// reconciliation, but keyed on the out-of-band record (no in-dir marker):
///
///   - non-`--force`: the stale entry is LEFT in the record so `doctor` keeps
///     surfacing it (and its `--force` fix now genuinely clears it).
///   - `--force`: remove the on-disk file only when it is our unmodified copy
///     (regular + hash match); report it in `pruned_companions` and drop it from
///     the record. A diverged / non-regular / absent file is left on disk but
///     dropped from tracking (we relinquish a copy we no longer recognise). A
///     failed delete keeps the entry tracked so a later redeploy retries.
fn reconcile_pi_companions(
    skill_name: &str,
    prov: &mut PiProvenance,
    force: bool,
    pruned_companions: &mut Vec<String>,
    warnings: &mut Vec<String>,
) {
    let Some(rec) = prov.skills.get_mut(skill_name) else {
        return;
    };
    let Some(dir) = pi_default_path(skill_name).and_then(|p| p.parent().map(Path::to_path_buf))
    else {
        return;
    };
    reconcile_pi_companions_at(skill_name, rec, &dir, force, pruned_companions, warnings);
}

/// Path-taking core of [`reconcile_pi_companions`], split out so the logic is
/// unit-testable against a tempdir without touching `$HOME`. `dir` is the pi
/// per-skill directory the companions live in.
fn reconcile_pi_companions_at(
    skill_name: &str,
    rec: &mut PiSkillRecord,
    dir: &Path,
    force: bool,
    pruned_companions: &mut Vec<String>,
    warnings: &mut Vec<String>,
) {
    let bundled: HashSet<&str> = resources_for(skill_name)
        .iter()
        .map(|r| r.filename)
        .collect();
    let stale: Vec<String> = rec
        .companions
        .keys()
        .filter(|f| !bundled.contains(f.as_str()))
        .cloned()
        .collect();
    if stale.is_empty() {
        return;
    }
    // Non-force: keep every stale entry so `doctor` keeps flagging it; the
    // `--force` fix it suggests is what actually clears it.
    if !force {
        return;
    }
    for filename in stale {
        if !is_simple_skill_name(&filename) {
            warnings.push(format!(
                "pi_provenance_bad_name: ignoring pi companion entry '{filename}' for skill '{skill_name}' (not a simple filename)"
            ));
            rec.companions.remove(&filename);
            continue;
        }
        let recorded_hash = rec.companions[&filename].clone();
        let path = dir.join(&filename);
        let is_our_copy = fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_file())
            && fs::read(&path).is_ok_and(|b| sha256_hex(&b) == recorded_hash);
        if is_our_copy {
            match fs::remove_file(&path) {
                Ok(()) => {
                    warnings.push(format!(
                        "skill_companion_pruned: removed orphan pi companion '{filename}' for skill '{skill_name}' at {}",
                        path.display()
                    ));
                    pruned_companions.push(format!("{skill_name}/{filename}"));
                    rec.companions.remove(&filename);
                }
                Err(e) => {
                    // Still on disk — keep tracking so a later redeploy retries and
                    // `doctor` keeps flagging it.
                    warnings.push(format!(
                        "skill_companion_prune_failed: could not remove orphan pi companion '{filename}' for skill '{skill_name}' at {}: {e}",
                        path.display()
                    ));
                }
            }
        } else {
            // Absent / non-regular / diverged: not our unmodified copy. Leave any
            // file in place and relinquish tracking rather than delete something we
            // no longer recognise as ours.
            warnings.push(format!(
                "pi_companion_relinquished: orphan pi companion '{filename}' for skill '{skill_name}' at {} is not our unmodified copy; no longer tracking it",
                path.display()
            ));
            rec.companions.remove(&filename);
        }
    }
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

/// The companion filenames a provenance marker records as managed (the
/// `companion:` lines). Empty when the marker is unreadable or records
/// none. Blank values are dropped. Sibling of the `skill_name:`/stamp
/// parsing in `is_managed_skill_dir`, but for the companion sub-records.
fn read_managed_companions(marker_path: &Path) -> Vec<String> {
    read_marker_records(marker_path, "companion")
}

/// The trimmed values of every `<key>:` line in a marker file (blanks
/// dropped). Shared by the claude marker's `companion:` reader and the
/// codex marker's `prompt:` / `companion:` readers, so all three parse the
/// same line shape identically. Unreadable / absent marker → empty.
fn read_marker_records(marker_path: &Path, key: &str) -> Vec<String> {
    let Ok(content) = fs::read_to_string(marker_path) else {
        return Vec::new();
    };
    let prefix = format!("{key}:");
    content
        .lines()
        .filter_map(|line| line.trim().strip_prefix(prefix.as_str()))
        .map(|rest| rest.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect()
}

/// Companion files recorded as managed in `<skill_dir>`'s provenance marker
/// that the current binary no longer bundles AND that still exist on disk —
/// the orphan companions a prior binary installed and this one dropped.
/// Filenames only (sorted, deduped); the caller joins `skill_dir` to report
/// or remove them. Consumed by the `skill.orphan.<name>.<file>` doctor
/// check. A still-bundled companion is never returned (it is audited by the
/// `skill.sync.<name>.<file>` forward check instead), and a user's own file
/// that the marker never recorded is never returned (that is what keeps this
/// from false-positiving on a hand-dropped note). Presence is probed with
/// `symlink_metadata` so a planted symlink is not followed.
pub fn orphan_companions(skill_name: &str, skill_dir: &Path) -> Vec<String> {
    let bundled: HashSet<&str> = resources_for(skill_name)
        .iter()
        .map(|r| r.filename)
        .collect();
    let marker_path = skill_dir.join(MANAGED_MARKER_FILENAME);
    let mut orphans: Vec<String> = read_managed_companions(&marker_path)
        .into_iter()
        .filter(|name| !bundled.contains(name.as_str()))
        .filter(|name| fs::symlink_metadata(skill_dir.join(name)).is_ok())
        .collect();
    orphans.sort();
    orphans.dedup();
    orphans
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
    fn every_claude_link_target_appears_in_some_skill_body() {
        // The codex rewrite is a literal `](target)` string replacement. If a
        // companion's `claude_link_target` ever drifts from the actual link
        // text in the skill bodies (a rename, a reflow), the rewrite silently
        // no-ops and codex ships a body still pointing at the un-resolvable
        // claude-layout path. Pin every declared target to real link text so
        // that drift fails the build instead of shipping a broken prompt.
        for skill in SKILLS {
            for r in resources_for(skill.name) {
                for target in r.claude_link_targets {
                    let needle = format!("]({target})");
                    assert!(
                        SKILLS.iter().any(|s| s.body.contains(&needle)),
                        "no skill body contains link {needle:?} declared for companion {} of {}",
                        r.filename,
                        skill.name
                    );
                }
            }
        }
    }

    #[test]
    fn render_body_for_claude_is_byte_identical_and_borrowed() {
        // The claude layout must be entirely unaffected by the codex rewrite:
        // every claude body is the embedded source, returned borrowed (no
        // reallocation, no byte change).
        for s in SKILLS {
            let rendered = render_body_for_agent("claude", s.body);
            assert!(
                matches!(rendered, Cow::Borrowed(_)),
                "claude body for {} was reallocated",
                s.name
            );
            assert_eq!(
                &*rendered, s.body,
                "claude body for {} was modified",
                s.name
            );
        }
    }

    #[test]
    fn render_body_for_codex_rewrites_both_companion_link_forms() {
        // The owning skill's sibling link and a cross-skill `../owner/` link
        // both collapse to the single shared `_shared/` target, and neither
        // claude form survives in the rendered codex body.
        let start = SKILLS.iter().find(|s| s.name == "stint-start").unwrap();
        let handoff = SKILLS.iter().find(|s| s.name == "stint-handoff").unwrap();
        let start_codex = render_body_for_agent("codex", start.body);
        let handoff_codex = render_body_for_agent("codex", handoff.body);

        assert!(start_codex.contains("](_shared/AGENTS-EXECUTION-DAG.md)"));
        assert!(!start_codex.contains("](AGENTS-EXECUTION-DAG.md)"));
        assert!(handoff_codex.contains("](_shared/AGENTS-EXECUTION-DAG.md)"));
        assert!(!handoff_codex.contains("](../stint-start/AGENTS-EXECUTION-DAG.md)"));
    }

    #[test]
    fn render_body_for_codex_without_companion_links_is_noop() {
        // A codex body that references no companion is returned borrowed and
        // unchanged — the global rewrite table only touches bodies that carry
        // a declared link form.
        let no_links = SKILLS.iter().find(|s| s.name == "worktree-code").unwrap();
        let rendered = render_body_for_agent("codex", no_links.body);
        assert!(matches!(rendered, Cow::Borrowed(_)));
        assert_eq!(&*rendered, no_links.body);
    }

    #[test]
    fn codex_companion_path_derives_shared_subdir() {
        // Default layout: sibling `_shared/` next to the flat prompt file.
        assert_eq!(
            codex_companion_path(Path::new("/home/u/.codex/prompts/stint-start.md"), "X.md"),
            PathBuf::from("/home/u/.codex/prompts/_shared/X.md")
        );
        // Nested relative dest.
        assert_eq!(
            codex_companion_path(Path::new("out/prompts/s.md"), "X.md"),
            PathBuf::from("out/prompts/_shared/X.md")
        );
        // Bare-relative dest (empty parent): `_shared/` in the current dir,
        // which is where the flat prompt file itself lands — the rewritten
        // `_shared/X.md` link resolves relative to it.
        assert_eq!(
            codex_companion_path(Path::new("s.md"), "X.md"),
            PathBuf::from("_shared/X.md")
        );
    }

    #[test]
    fn write_marker_records_companions_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join(MANAGED_MARKER_FILENAME);
        write_marker(
            &marker,
            "stint-start",
            &["A.md".to_string(), "B.md".to_string()],
        )
        .unwrap();
        let recorded = read_managed_companions(&marker);
        assert_eq!(recorded, vec!["A.md".to_string(), "B.md".to_string()]);
        // A markerless dir records nothing.
        assert!(read_managed_companions(&dir.path().join("nope")).is_empty());
    }

    #[test]
    fn orphan_companions_flags_dropped_but_not_bundled() {
        // Simulate a prior binary that installed `stint-start` with an extra
        // companion `OLD-COMPANION.md` this binary no longer ships, alongside
        // the still-bundled real one. The dropped file lingers on disk and the
        // marker still records it.
        let skill = "stint-start";
        let bundled: Vec<&str> = resources_for(skill).iter().map(|r| r.filename).collect();
        assert!(
            !bundled.is_empty(),
            "test assumes stint-start ships >=1 companion"
        );
        let dir = tempfile::tempdir().unwrap();
        // Marker records both the still-bundled companions AND the orphan.
        let mut recorded: Vec<String> = bundled.iter().copied().map(String::from).collect();
        recorded.push("OLD-COMPANION.md".to_string());
        write_marker(&dir.path().join(MANAGED_MARKER_FILENAME), skill, &recorded).unwrap();
        // Both files present on disk.
        for f in &bundled {
            fs::write(dir.path().join(f), "x").unwrap();
        }
        fs::write(dir.path().join("OLD-COMPANION.md"), "stale").unwrap();

        let orphans = orphan_companions(skill, dir.path());
        assert_eq!(
            orphans,
            vec!["OLD-COMPANION.md".to_string()],
            "only the dropped-but-recorded companion is an orphan"
        );
    }

    #[test]
    fn orphan_companions_ignores_unrecorded_user_file() {
        // A file a user dropped into the managed dir that the marker never
        // recorded must NOT be flagged — that is the false-positive the
        // marker-based design exists to avoid.
        let skill = "stint-start";
        let bundled: Vec<String> = resources_for(skill)
            .iter()
            .map(|r| r.filename.to_string())
            .collect();
        let dir = tempfile::tempdir().unwrap();
        // Marker records only the bundled companions (a clean install).
        write_marker(&dir.path().join(MANAGED_MARKER_FILENAME), skill, &bundled).unwrap();
        // User drops their own note — never recorded in the marker.
        fs::write(dir.path().join("my-note.md"), "mine").unwrap();

        assert!(
            orphan_companions(skill, dir.path()).is_empty(),
            "an unrecorded user file is not an orphan"
        );
    }

    #[test]
    fn orphan_companions_ignores_recorded_but_absent_file() {
        // The marker records an orphan whose file was already removed: nothing
        // to clean, so it is not reported (a WARN with no fixable target would
        // be noise).
        let skill = "stint-start";
        let bundled: Vec<String> = resources_for(skill)
            .iter()
            .map(|r| r.filename.to_string())
            .collect();
        let dir = tempfile::tempdir().unwrap();
        let mut recorded = bundled.clone();
        recorded.push("GONE.md".to_string());
        write_marker(&dir.path().join(MANAGED_MARKER_FILENAME), skill, &recorded).unwrap();
        // `GONE.md` is NOT written to disk.
        assert!(orphan_companions(skill, dir.path()).is_empty());
    }

    #[test]
    fn codex_marker_records_prompts_and_companions_read_back() {
        // The single codex marker records both `prompt:` and `companion:`
        // lines; each key's reader returns only its own records.
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join(MANAGED_MARKER_FILENAME);
        write_codex_marker(
            &marker,
            &["stint-start".to_string(), "worktree-code".to_string()],
            &["AGENTS-EXECUTION-DAG.md".to_string()],
        )
        .unwrap();
        assert_eq!(
            read_marker_records(&marker, "prompt"),
            vec!["stint-start".to_string(), "worktree-code".to_string()]
        );
        assert_eq!(
            read_marker_records(&marker, "companion"),
            vec!["AGENTS-EXECUTION-DAG.md".to_string()]
        );
        // An absent marker records nothing for either key.
        let missing = dir.path().join("nope");
        assert!(read_marker_records(&missing, "prompt").is_empty());
        assert!(read_marker_records(&missing, "companion").is_empty());
    }

    #[test]
    fn all_companion_sources_dedupes_by_filename() {
        // Every declared companion filename is unique and the list is sorted;
        // the shared `_shared/` layout depends on one entry per filename.
        let sources = all_companion_sources();
        let mut names: Vec<&str> = sources.iter().map(|c| c.filename).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(
            names, sorted,
            "companion sources must be sorted by filename"
        );
        names.dedup();
        assert_eq!(
            names.len(),
            sources.len(),
            "companion filenames must be unique across skills"
        );
        // stint-start's companion is present (guards the doctor codex checks
        // have at least one companion to audit).
        assert!(sources
            .iter()
            .any(|c| c.filename == "AGENTS-EXECUTION-DAG.md"));
    }

    #[test]
    fn prune_codex_file_outcomes() {
        let dir = tempfile::tempdir().unwrap();
        let mut warnings: Vec<String> = Vec::new();

        // A regular file is removed.
        let regular = dir.path().join("gone.md");
        fs::write(&regular, "stale").unwrap();
        assert!(matches!(
            prune_codex_file(&regular, "removed", "failed", &mut warnings),
            CodexPruneOutcome::Removed
        ));
        assert!(!regular.exists(), "file must be gone after Removed");
        assert!(warnings[0].starts_with("removed at "));

        // An absent file yields Dropped (nothing to clean).
        let absent = dir.path().join("never.md");
        assert!(matches!(
            prune_codex_file(&absent, "removed", "failed", &mut warnings),
            CodexPruneOutcome::Dropped
        ));

        // A directory squatting at the path is never removed (Dropped, not a
        // recursive delete).
        let squat = dir.path().join("squat.md");
        fs::create_dir(&squat).unwrap();
        assert!(matches!(
            prune_codex_file(&squat, "removed", "failed", &mut warnings),
            CodexPruneOutcome::Dropped
        ));
        assert!(squat.is_dir(), "a squatting dir must be left intact");
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

    #[test]
    fn sha256_hex_is_deterministic_and_lowercase() {
        let a = sha256_hex(b"hello");
        assert_eq!(a, sha256_hex(b"hello"));
        assert_ne!(a, sha256_hex(b"world"));
        assert_eq!(a.len(), 64);
        assert!(a
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        // Known vector for "hello".
        assert_eq!(
            a,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn pi_provenance_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state").join("pi-installed-skills.json");
        let mut prov = PiProvenance {
            schema_version: PI_PROVENANCE_SCHEMA_VERSION,
            skills: BTreeMap::new(),
        };
        prov.skills.insert(
            "stint-start".to_string(),
            PiSkillRecord {
                sha256: sha256_hex(b"body-a"),
                cli_version: "0.1.7".to_string(),
                companions: BTreeMap::new(),
            },
        );
        // Parent dir does not exist yet — the write must create `state/`.
        write_pi_provenance(&path, &prov).unwrap();
        let read_back = read_pi_provenance(&path);
        assert_eq!(read_back.schema_version, PI_PROVENANCE_SCHEMA_VERSION);
        assert_eq!(
            read_back
                .skills
                .get("stint-start")
                .map(|r| r.sha256.clone()),
            Some(sha256_hex(b"body-a"))
        );
    }

    #[test]
    fn read_pi_provenance_tolerates_missing_and_garbage() {
        let dir = tempfile::tempdir().unwrap();
        // Missing file → empty default.
        let missing = dir.path().join("nope.json");
        assert!(read_pi_provenance(&missing).skills.is_empty());
        // Unparseable content → empty default (err toward NOT managing).
        let garbage = dir.path().join("garbage.json");
        fs::write(&garbage, "{ not json").unwrap();
        assert!(read_pi_provenance(&garbage).skills.is_empty());
        // Future-schema record → lenient reader treats it as empty (doctor
        // must not audit a record a newer binary wrote).
        let future = dir.path().join("future.json");
        fs::write(&future, r#"{"schema_version":999,"skills":{}}"#).unwrap();
        assert!(read_pi_provenance(&future).skills.is_empty());
    }

    #[test]
    fn load_pi_provenance_for_write_fails_closed_on_corruption() {
        let dir = tempfile::tempdir().unwrap();
        // Missing → Ok(empty): a first install starts fresh.
        let missing = dir.path().join("nope.json");
        assert!(load_pi_provenance_for_write(&missing)
            .unwrap()
            .skills
            .is_empty());
        // Unparseable → Err (never launder + overwrite, which would erase all
        // tracking).
        let garbage = dir.path().join("garbage.json");
        fs::write(&garbage, "{ not json").unwrap();
        let err = load_pi_provenance_for_write(&garbage).unwrap_err();
        assert_eq!(err.code, "pi_provenance_corrupt");
        // Future schema → Err (an older binary must not downgrade-overwrite it).
        let future = dir.path().join("future.json");
        fs::write(&future, r#"{"schema_version":999,"skills":{}}"#).unwrap();
        let err = load_pi_provenance_for_write(&future).unwrap_err();
        assert_eq!(err.code, "pi_provenance_schema_too_new");
    }

    #[test]
    fn is_simple_skill_name_rejects_traversal_and_absolute() {
        assert!(is_simple_skill_name("stint-start"));
        assert!(is_simple_skill_name("worktree-code"));
        assert!(!is_simple_skill_name("../../.bashrc"));
        assert!(!is_simple_skill_name("a/b"));
        assert!(!is_simple_skill_name("/etc/passwd"));
        assert!(!is_simple_skill_name(".."));
        assert!(!is_simple_skill_name(""));
    }

    #[test]
    fn write_pi_provenance_replaces_squatting_symlink() {
        // A symlink squatting at the record path must be replaced by the atomic
        // rename, never followed to clobber its target.
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("victim.txt");
        fs::write(&target, "precious").unwrap();
        let record = dir.path().join("pi-installed-skills.json");
        std::os::unix::fs::symlink(&target, &record).unwrap();

        let prov = PiProvenance {
            schema_version: PI_PROVENANCE_SCHEMA_VERSION,
            skills: BTreeMap::new(),
        };
        write_pi_provenance(&record, &prov).unwrap();

        // The victim is untouched; the record path is now a regular file.
        assert_eq!(fs::read_to_string(&target).unwrap(), "precious");
        assert!(fs::symlink_metadata(&record).unwrap().file_type().is_file());
    }

    #[test]
    fn prune_pi_mirror_removes_our_unmodified_copy_and_empty_dir() {
        let root = tempfile::tempdir().unwrap();
        let skill_dir = root.path().join("old-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        let mirror = skill_dir.join("SKILL.md");
        let body = b"---\nname: old-skill\n---\nbody\n";
        fs::write(&mirror, body).unwrap();

        let mut warnings = Vec::new();
        let outcome = prune_pi_mirror_at(
            "old-skill",
            &mirror,
            Some(root.path()),
            &sha256_hex(body),
            &BTreeMap::new(),
            &mut warnings,
        );
        assert!(matches!(outcome, PiPruneOutcome::Removed));
        assert!(!mirror.exists(), "mirror file must be gone");
        assert!(
            !skill_dir.exists(),
            "empty per-skill dir must be cleaned up"
        );
        assert!(warnings[0].starts_with("pi_mirror_pruned:"));
    }

    #[test]
    fn prune_pi_mirror_removes_recorded_companions_then_empty_dir() {
        // A de-registered skill's recorded companions are removed alongside its
        // SKILL.md so the per-skill dir empties out. A companion that has since
        // diverged from what we wrote is LEFT in place (and keeps the dir).
        let root = tempfile::tempdir().unwrap();
        let skill_dir = root.path().join("old-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        let mirror = skill_dir.join("SKILL.md");
        let body = b"---\nname: old-skill\n---\nbody\n";
        fs::write(&mirror, body).unwrap();
        let comp_body = b"companion payload\n";
        fs::write(skill_dir.join("AGENTS-EXECUTION-DAG.md"), comp_body).unwrap();
        // A second recorded companion the user has since edited: must survive.
        fs::write(skill_dir.join("EDITED.md"), b"user changed this").unwrap();

        let mut companions = BTreeMap::new();
        companions.insert("AGENTS-EXECUTION-DAG.md".to_string(), sha256_hex(comp_body));
        companions.insert("EDITED.md".to_string(), sha256_hex(b"original edited body"));

        let mut warnings = Vec::new();
        let outcome = prune_pi_mirror_at(
            "old-skill",
            &mirror,
            Some(root.path()),
            &sha256_hex(body),
            &companions,
            &mut warnings,
        );
        assert!(matches!(outcome, PiPruneOutcome::Removed));
        assert!(!mirror.exists(), "SKILL.md removed");
        assert!(
            !skill_dir.join("AGENTS-EXECUTION-DAG.md").exists(),
            "our unmodified companion is removed"
        );
        assert!(
            skill_dir.join("EDITED.md").exists(),
            "a diverged companion is preserved"
        );
        // The dir is NOT empty (the diverged companion remains), so it survives.
        assert!(skill_dir.exists(), "dir with a surviving companion is kept");
        assert!(warnings
            .iter()
            .any(|w| w.starts_with("pi_companion_pruned:")));
        assert!(warnings
            .iter()
            .any(|w| w.starts_with("pi_companion_diverged:")));
    }

    #[test]
    fn prune_pi_mirror_removes_all_matching_companions_and_empty_dir() {
        // When every recorded companion is our unmodified copy, both it and the
        // SKILL.md are removed and the now-empty dir is cleaned up.
        let root = tempfile::tempdir().unwrap();
        let skill_dir = root.path().join("old-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        let mirror = skill_dir.join("SKILL.md");
        let body = b"body\n";
        fs::write(&mirror, body).unwrap();
        let c1 = b"c1\n";
        let c2 = b"c2\n";
        fs::write(skill_dir.join("A.md"), c1).unwrap();
        fs::write(skill_dir.join("B.md"), c2).unwrap();
        let mut companions = BTreeMap::new();
        companions.insert("A.md".to_string(), sha256_hex(c1));
        companions.insert("B.md".to_string(), sha256_hex(c2));

        let mut warnings = Vec::new();
        let outcome = prune_pi_mirror_at(
            "old-skill",
            &mirror,
            Some(root.path()),
            &sha256_hex(body),
            &companions,
            &mut warnings,
        );
        assert!(matches!(outcome, PiPruneOutcome::Removed));
        assert!(!skill_dir.exists(), "fully-cleaned dir must be removed");
    }

    #[test]
    fn prune_pi_mirror_cleans_companions_when_skill_md_absent() {
        // A prior partial prune left the SKILL.md gone but a recorded companion
        // behind. The prune must still clean the companion (never strand it) and
        // remove the now-empty dir, returning Dropped.
        let root = tempfile::tempdir().unwrap();
        let skill_dir = root.path().join("old-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        let mirror = skill_dir.join("SKILL.md"); // never written — absent
        let comp = b"companion\n";
        fs::write(skill_dir.join("C.md"), comp).unwrap();
        let mut companions = BTreeMap::new();
        companions.insert("C.md".to_string(), sha256_hex(comp));

        let mut warnings = Vec::new();
        let outcome = prune_pi_mirror_at(
            "old-skill",
            &mirror,
            Some(root.path()),
            "irrelevant-body-hash",
            &companions,
            &mut warnings,
        );
        assert!(matches!(outcome, PiPruneOutcome::Dropped));
        assert!(
            !skill_dir.join("C.md").exists(),
            "recorded companion cleaned even with an absent SKILL.md"
        );
        assert!(!skill_dir.exists(), "now-empty dir removed");
        assert!(warnings
            .iter()
            .any(|w| w.starts_with("pi_companion_pruned:")));
    }

    #[test]
    fn prune_pi_mirror_diverged_body_leaves_companions_untouched() {
        // A user-edited SKILL.md transfers the whole dir to the user: companions
        // are NOT inspected or removed, and the outcome is Diverged.
        let root = tempfile::tempdir().unwrap();
        let skill_dir = root.path().join("old-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        let mirror = skill_dir.join("SKILL.md");
        fs::write(&mirror, b"user edited body").unwrap();
        let comp = b"companion\n";
        fs::write(skill_dir.join("C.md"), comp).unwrap();
        let mut companions = BTreeMap::new();
        companions.insert("C.md".to_string(), sha256_hex(comp));

        let mut warnings = Vec::new();
        let outcome = prune_pi_mirror_at(
            "old-skill",
            &mirror,
            Some(root.path()),
            &sha256_hex(b"the body we originally wrote"),
            &companions,
            &mut warnings,
        );
        assert!(matches!(outcome, PiPruneOutcome::Diverged));
        assert!(mirror.exists(), "diverged body is left in place");
        assert!(
            skill_dir.join("C.md").exists(),
            "companions are left untouched when the body diverged"
        );
    }

    #[test]
    fn reconcile_pi_companions_force_removes_dropped_companion() {
        // A still-registered skill whose record tracks a companion the binary no
        // longer bundles: under --force the orphan file is removed and dropped
        // from the record; a still-bundled companion is left tracked.
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path();
        // stint-start ships AGENTS-EXECUTION-DAG.md; OLD.md is the dropped one.
        let dag = b"dag body\n";
        let old = b"old body\n";
        fs::write(dir.join("AGENTS-EXECUTION-DAG.md"), dag).unwrap();
        fs::write(dir.join("OLD.md"), old).unwrap();

        let mut rec = PiSkillRecord {
            sha256: sha256_hex(b"skill body"),
            cli_version: CLI_VERSION.to_string(),
            companions: BTreeMap::new(),
        };
        rec.companions
            .insert("AGENTS-EXECUTION-DAG.md".to_string(), sha256_hex(dag));
        rec.companions.insert("OLD.md".to_string(), sha256_hex(old));

        let mut pruned = Vec::new();
        let mut warnings = Vec::new();
        reconcile_pi_companions_at(
            "stint-start",
            &mut rec,
            dir,
            true,
            &mut pruned,
            &mut warnings,
        );

        assert_eq!(pruned, vec!["stint-start/OLD.md".to_string()]);
        assert!(
            rec.companions.contains_key("AGENTS-EXECUTION-DAG.md"),
            "still-bundled companion stays tracked"
        );
        assert!(
            !rec.companions.contains_key("OLD.md"),
            "dropped companion is removed from the record"
        );
        assert!(
            !dir.join("OLD.md").exists(),
            "orphan file removed on --force"
        );
        assert!(
            dir.join("AGENTS-EXECUTION-DAG.md").exists(),
            "bundled companion file left in place"
        );
    }

    #[test]
    fn reconcile_pi_companions_non_force_keeps_orphan_tracked() {
        // Without --force the stale companion is LEFT tracked so `doctor` keeps
        // flagging it (its --force fix is what clears it).
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path();
        fs::write(dir.join("OLD.md"), b"old\n").unwrap();

        let mut rec = PiSkillRecord {
            sha256: sha256_hex(b"body"),
            cli_version: CLI_VERSION.to_string(),
            companions: BTreeMap::new(),
        };
        rec.companions
            .insert("OLD.md".to_string(), sha256_hex(b"old\n"));

        let mut pruned = Vec::new();
        let mut warnings = Vec::new();
        reconcile_pi_companions_at(
            "stint-start",
            &mut rec,
            dir,
            false,
            &mut pruned,
            &mut warnings,
        );

        assert!(pruned.is_empty(), "non-force prunes nothing");
        assert!(
            rec.companions.contains_key("OLD.md"),
            "orphan stays tracked without --force"
        );
        assert!(dir.join("OLD.md").exists(), "orphan file left on disk");
    }

    #[test]
    fn pi_provenance_v1_record_reads_and_upgrades_to_v2() {
        // A v1 record (no `companions` field) reads with an empty companions map
        // (serde default), and a fresh write stamps the current schema (v2).
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state").join("pi-installed-skills.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            r#"{"schema_version":1,"skills":{"stint-start":{"sha256":"aa","cli_version":"0.1.0"}}}"#,
        )
        .unwrap();
        let mut prov = load_pi_provenance_for_write(&path).unwrap();
        assert_eq!(prov.schema_version, 1, "read preserves the on-disk version");
        assert!(prov.skills["stint-start"].companions.is_empty());
        // A write stamps the current (v2) schema.
        prov.schema_version = PI_PROVENANCE_SCHEMA_VERSION;
        write_pi_provenance(&path, &prov).unwrap();
        let reread: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(reread["schema_version"], 2);
    }

    #[test]
    fn prune_pi_mirror_preserves_dir_with_user_sibling() {
        let root = tempfile::tempdir().unwrap();
        let skill_dir = root.path().join("old-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        let mirror = skill_dir.join("SKILL.md");
        let body = b"our body\n";
        fs::write(&mirror, body).unwrap();
        // A user-added sibling in the same dir.
        fs::write(skill_dir.join("notes.md"), "mine").unwrap();

        let mut warnings = Vec::new();
        let outcome = prune_pi_mirror_at(
            "old-skill",
            &mirror,
            Some(root.path()),
            &sha256_hex(body),
            &BTreeMap::new(),
            &mut warnings,
        );
        assert!(matches!(outcome, PiPruneOutcome::Removed));
        assert!(!mirror.exists(), "our SKILL.md is removed");
        assert!(skill_dir.exists(), "non-empty dir is preserved");
        assert!(skill_dir.join("notes.md").exists(), "user sibling survives");
    }

    #[test]
    fn prune_pi_mirror_refuses_diverged_copy() {
        let root = tempfile::tempdir().unwrap();
        let mirror = root.path().join("SKILL.md");
        fs::write(&mirror, b"user has edited this").unwrap();

        let mut warnings = Vec::new();
        // Recorded hash is of the ORIGINAL body we wrote, which no longer matches.
        let outcome = prune_pi_mirror_at(
            "old-skill",
            &mirror,
            Some(root.path()),
            &sha256_hex(b"original body"),
            &BTreeMap::new(),
            &mut warnings,
        );
        assert!(matches!(outcome, PiPruneOutcome::Diverged));
        assert!(
            mirror.exists(),
            "a diverged (user-owned) copy is NOT deleted"
        );
        assert!(warnings[0].starts_with("pi_mirror_diverged:"));
    }

    #[test]
    fn prune_pi_mirror_drops_absent_symlink_and_dir() {
        let root = tempfile::tempdir().unwrap();
        let mut warnings = Vec::new();

        // Absent path.
        let absent = root.path().join("gone/SKILL.md");
        assert!(matches!(
            prune_pi_mirror_at(
                "gone",
                &absent,
                Some(root.path()),
                "anyhash",
                &BTreeMap::new(),
                &mut warnings
            ),
            PiPruneOutcome::Dropped
        ));

        // A directory squatting where SKILL.md should be is never removed.
        let squat = root.path().join("squat");
        fs::create_dir_all(&squat).unwrap();
        assert!(matches!(
            prune_pi_mirror_at(
                "squat",
                &squat,
                Some(root.path()),
                "anyhash",
                &BTreeMap::new(),
                &mut warnings
            ),
            PiPruneOutcome::Dropped
        ));
        assert!(squat.is_dir(), "a squatting dir must be left intact");

        // A symlink at the mirror path is never followed/deleted.
        let real = root.path().join("real.md");
        fs::write(&real, b"body").unwrap();
        let link = root.path().join("link-SKILL.md");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(matches!(
            prune_pi_mirror_at(
                "linked",
                &link,
                Some(root.path()),
                &sha256_hex(b"body"),
                &BTreeMap::new(),
                &mut warnings
            ),
            PiPruneOutcome::Dropped
        ));
        assert!(link.exists(), "the symlink is left intact");
        assert!(real.exists(), "the symlink target is untouched");
    }
}
