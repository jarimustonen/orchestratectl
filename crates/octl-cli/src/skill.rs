//! `skill` subcommand — list / show / install companion AI-skills.
//!
//! Skill files (`SKILL.md`) live under `crates/octl-cli/skills/<name>/`
//! and are embedded into the binary at compile time via `include_str!`,
//! so they version with the CLI. See `AGENTS-AI-FIRST-CLI.md` §15.

use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;
use tempfile::NamedTempFile;

use crate::error::CliError;
use crate::output;

/// One embedded skill: name and full SKILL.md text. The description is
/// parsed lazily from the body's frontmatter so the catalog stays a
/// single source of truth (the SKILL.md file).
struct EmbeddedSkill {
    name: &'static str,
    body: &'static str,
}

const SKILLS: &[EmbeddedSkill] = &[
    EmbeddedSkill {
        name: "octl-run-overview",
        body: include_str!("../skills/octl-run-overview/SKILL.md"),
    },
    EmbeddedSkill {
        name: "octl-spawn-spinoff",
        body: include_str!("../skills/octl-spawn-spinoff/SKILL.md"),
    },
];

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

pub fn cmd_list(json: bool, warnings: &[String]) -> Result<(), CliError> {
    let skills: Vec<SkillSummary> = SKILLS
        .iter()
        .map(|s| SkillSummary {
            name: s.name,
            description: parse_description(s.body).unwrap_or_default(),
        })
        .collect();
    if json {
        output::emit_json(&ListPayload { skills }, warnings)
            .map_err(|e| CliError::system("internal_serialize", e.to_string()))?;
    } else {
        for s in &skills {
            println!("{}\t{}", s.name, s.description);
        }
        for w in warnings {
            eprintln!("warning: {}", w);
        }
    }
    Ok(())
}

pub fn cmd_show(name: &str, json: bool, warnings: &[String]) -> Result<(), CliError> {
    let skill = lookup(name)?;
    if json {
        #[derive(Serialize)]
        struct ShowPayload<'a> {
            name: &'a str,
            content: &'a str,
        }
        output::emit_json(
            &ShowPayload {
                name: skill.name,
                content: skill.body,
            },
            warnings,
        )
        .map_err(|e| CliError::system("internal_serialize", e.to_string()))?;
    } else {
        print!("{}", skill.body);
        if !skill.body.ends_with('\n') {
            println!();
        }
        for w in warnings {
            eprintln!("warning: {}", w);
        }
    }
    Ok(())
}

pub fn cmd_install(
    name: Option<&str>,
    agent: AgentTarget,
    dest: Option<PathBuf>,
    force: bool,
    json: bool,
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

    preflight(&plan, force)?;

    let mut installed = Vec::with_capacity(plan.len());
    for (skill, agent_name, path) in plan {
        write_atomic(&path, skill.body, force)?;
        installed.push(InstalledFile {
            name: skill.name,
            agent: agent_name,
            path: path.display().to_string(),
        });
    }

    let payload = InstallPayload { installed };
    if json {
        output::emit_json(&payload, warnings)
            .map_err(|e| CliError::system("internal_serialize", e.to_string()))?;
    } else {
        for f in &payload.installed {
            println!("installed {} ({}) -> {}", f.name, f.agent, f.path);
        }
        for w in warnings {
            eprintln!("warning: {}", w);
        }
    }
    Ok(())
}

/// Reject the whole install plan before touching the filesystem when any
/// destination already exists (without `--force`) or appears twice in
/// the plan. Catches the partial-install retry trap noted by the review:
/// without preflight, writing N targets sequentially can leave the user
/// with a half-installed catalog and an ambiguous error on retry.
fn preflight(
    plan: &[(&'static EmbeddedSkill, &'static str, PathBuf)],
    force: bool,
) -> Result<(), CliError> {
    let mut seen: HashSet<&Path> = HashSet::new();
    for (_, _, path) in plan {
        if !seen.insert(path.as_path()) {
            return Err(CliError::user(
                "duplicate_destination",
                format!("destination appears more than once: {}", path.display()),
            )
            .with_invalid_value(path.display().to_string()));
        }
        if path.exists() && !force {
            return Err(CliError::system(
                "refused_overwrite",
                format!(
                    "{} already exists; pass --force to overwrite",
                    path.display()
                ),
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
    }
    Ok(())
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
    // Tolerate a UTF-8 BOM at the start of the file.
    let body = body.strip_prefix('\u{feff}').unwrap_or(body);
    let mut lines = body.lines();
    if lines.next()?.trim_end() != "---" {
        return None;
    }
    for line in lines {
        if line.trim_end() == "---" {
            return None;
        }
        let (key, value) = line.split_once(':')?;
        if key.trim() == "description" {
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
    let body = body.strip_prefix('\u{feff}').unwrap_or(body);
    let mut lines = body.lines();
    if lines.next()?.trim_end() != "---" {
        return None;
    }
    for line in lines {
        if line.trim_end() == "---" {
            return None;
        }
        let (key, value) = line.split_once(':')?;
        if key.trim() == "name" {
            return Some(value.trim().to_string());
        }
    }
    None
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
        }
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
