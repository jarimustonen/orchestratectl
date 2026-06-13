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
];

/// Binary version embedded at build time. `build.rs` substitutes this
/// into every shipped SKILL.md's `cli_version:` frontmatter, so `skill
/// print` always returns a body whose declared `cli_version` matches the
/// binary that emitted it (AGENTS-AI-FIRST-CLI §17).
const CLI_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Skill-format schema version (the version of the SKILL.md frontmatter +
/// body contract itself, distinct from the envelope `schema_version`).
const SKILL_SCHEMA_VERSION: u32 = 1;

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

    // Build the full (skill, agent, path) plan first, then preflight, then
    // write. This avoids the partial-install retry trap where one of N
    // writes succeeds and a re-run hits refused_overwrite on a different
    // path than the original failure.
    let mut plan: Vec<(&'static EmbeddedSkill, &'static str, PathBuf)> = Vec::new();
    for skill in &skills {
        match (&agent, dest.as_ref()) {
            (AgentTarget::Claude, Some(p)) => plan.push((skill, "claude", p.clone())),
            (AgentTarget::Codex, Some(p)) => plan.push((skill, "codex", p.clone())),
            (AgentTarget::Claude, None) => {
                plan.push((skill, "claude", default_path("claude", skill.name)?))
            }
            (AgentTarget::Codex, None) => {
                plan.push((skill, "codex", default_path("codex", skill.name)?))
            }
            (AgentTarget::All, _) => {
                plan.push((skill, "claude", default_path("claude", skill.name)?));
                plan.push((skill, "codex", default_path("codex", skill.name)?));
            }
        }
    }

    let preflight_result = preflight(&plan, force)?;

    // Combine caller-provided warnings (logging init, etc.) with
    // drift-detected ones so the success envelope surfaces both.
    let mut all_warnings: Vec<String> = warnings.to_vec();
    all_warnings.extend(preflight_result.warnings);

    let mut installed = Vec::with_capacity(plan.len());
    for (skill, agent_name, path) in plan {
        // The set of paths approved for overwrite is decided exclusively
        // by preflight — never recomputed from `path.exists()` in this
        // loop. That keeps the persist_noclobber TOCTOU guarantee intact:
        // a file that did not exist at preflight time will refuse to
        // overwrite, even if a concurrent process created it in the
        // window. (Review finding #1.)
        let allow_overwrite = preflight_result.overwrite_allowed.contains(&path);
        write_atomic(&path, skill.body, allow_overwrite)?;
        installed.push(InstalledFile {
            name: skill.name,
            agent: agent_name,
            path: path.display().to_string(),
        });
    }

    let payload = InstallPayload { installed };
    match spec.format {
        OutputFormat::Json | OutputFormat::Jsonl => {
            output::emit_envelope(&payload, spec, &all_warnings)?;
        }
        OutputFormat::Text => {
            for f in &payload.installed {
                println!("installed {} ({}) -> {}", f.name, f.agent, f.path);
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

/// Reject the whole install plan before touching the filesystem when any
/// destination already exists (without `--force`) or appears twice in
/// the plan. Catches the partial-install retry trap noted by the review:
/// without preflight, writing N targets sequentially can leave the user
/// with a half-installed catalog and an ambiguous error on retry.
fn preflight(
    plan: &[(&'static EmbeddedSkill, &'static str, PathBuf)],
    force: bool,
) -> Result<PreflightResult, CliError> {
    use std::cmp::Ordering;
    let mut seen: HashSet<&Path> = HashSet::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut overwrite_allowed: HashSet<PathBuf> = HashSet::new();
    for (skill, _, path) in plan {
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
                    "skill_version_drift: {} on disk is {}; binary ships {}; overwriting",
                    skill.name, v, CLI_VERSION
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
                    "skill_version_drift: {} on disk is {} (newer than binary {}); --force overwriting",
                    skill.name, v, CLI_VERSION
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
    tmp.write_all(content.as_bytes()).map_err(|e| {
        CliError::system("write_failed", format!("could not write tempfile: {}", e))
    })?;
    tmp.as_file_mut().sync_all().map_err(|e| {
        CliError::system("fsync_failed", format!("could not fsync tempfile: {}", e))
    })?;

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
