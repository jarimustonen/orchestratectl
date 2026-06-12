//! `skill` subcommand — list / show / install companion AI-skills.
//!
//! Skill files (`SKILL.md`) live under `crates/octl-cli/skills/<name>/`
//! and are embedded into the binary at compile time via `include_str!`,
//! so they version with the CLI. See `AGENTS-AI-FIRST-CLI.md` §15.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;
use tempfile::NamedTempFile;

use crate::error::CliError;
use crate::output;

/// One embedded skill: name, full SKILL.md text, and its parsed
/// frontmatter description for `skill list`.
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
    name: &'static str,
    installed: Vec<InstalledFile>,
}

#[derive(Serialize)]
struct InstalledFile {
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
    name: &str,
    agent: AgentTarget,
    dest: Option<PathBuf>,
    force: bool,
    json: bool,
    warnings: &[String],
) -> Result<(), CliError> {
    let skill = lookup(name)?;

    // `--dest` is incompatible with `--agent all`: a single path cannot
    // host both installations. Reject early with a user-facing error.
    if dest.is_some() && matches!(agent, AgentTarget::All) {
        return Err(CliError::user(
            "invalid_arguments",
            "--dest cannot be combined with --agent all",
        ));
    }

    let targets: Vec<(&'static str, PathBuf)> = match (&agent, dest) {
        (AgentTarget::Claude, Some(p)) => vec![("claude", p)],
        (AgentTarget::Codex, Some(p)) => vec![("codex", p)],
        (AgentTarget::All, _) => vec![
            ("claude", default_path("claude", skill.name)?),
            ("codex", default_path("codex", skill.name)?),
        ],
        (AgentTarget::Claude, None) => vec![("claude", default_path("claude", skill.name)?)],
        (AgentTarget::Codex, None) => vec![("codex", default_path("codex", skill.name)?)],
    };

    let mut installed = Vec::with_capacity(targets.len());
    for (agent_name, path) in targets {
        write_atomic(&path, skill.body, force)?;
        installed.push(InstalledFile {
            agent: agent_name,
            path: path.display().to_string(),
        });
    }

    let payload = InstallPayload {
        name: skill.name,
        installed,
    };
    if json {
        output::emit_json(&payload, warnings)
            .map_err(|e| CliError::system("internal_serialize", e.to_string()))?;
    } else {
        for f in &payload.installed {
            println!("installed {} ({}) -> {}", payload.name, f.agent, f.path);
        }
        for w in warnings {
            eprintln!("warning: {}", w);
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
            "HOME is not set; cannot resolve default install path",
        )
    })?;
    let base = PathBuf::from(home);
    Ok(match agent {
        "claude" => base.join(".claude/skills").join(name).join("SKILL.md"),
        "codex" => base.join(".codex/prompts").join(format!("{name}.md")),
        // unreachable in practice — match the explicit literals above.
        other => {
            return Err(CliError::user(
                "invalid_agent",
                format!("unknown agent '{other}'"),
            ))
        }
    })
}

fn write_atomic(path: &Path, content: &str, force: bool) -> Result<(), CliError> {
    if path.exists() && !force {
        return Err(CliError::system(
            "refused-overwrite",
            format!(
                "{} already exists; pass --force to overwrite",
                path.display()
            ),
        )
        .with_invalid_value(path.display().to_string()));
    }

    let parent = path.parent().ok_or_else(|| {
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

    // Atomic via tempfile in the same directory + rename.
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
    tmp.persist(path).map_err(|e| {
        CliError::system(
            "rename_failed",
            format!("could not rename into place {}: {}", path.display(), e),
        )
    })?;
    Ok(())
}

/// Extract the `description:` field from YAML-ish frontmatter at the top
/// of a SKILL.md. Frontmatter is everything between the first `---` and
/// the next `---`. We only handle simple `key: value` lines (no multi-line
/// scalars, no nested maps) — sufficient for the SKILL.md schema.
fn parse_description(body: &str) -> Option<String> {
    let rest = body.strip_prefix("---\n")?;
    let end = rest.find("\n---")?;
    let block = &rest[..end];
    for line in block.lines() {
        if let Some(value) = line.strip_prefix("description:") {
            return Some(value.trim().to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_embedded_skill_has_a_description() {
        // Guards against frontmatter drift: if someone edits a SKILL.md
        // and breaks the `description:` line, `skill list` would silently
        // emit an empty string. Fail the build instead.
        for s in SKILLS {
            let d = parse_description(s.body)
                .unwrap_or_else(|| panic!("skill {} missing description", s.name));
            assert!(!d.is_empty(), "skill {} has empty description", s.name);
        }
    }

    #[test]
    fn parse_description_extracts_value() {
        let body = "---\nname: foo\ndescription: a short blurb\nversion: 1\n---\n\n# body\n";
        assert_eq!(parse_description(body).as_deref(), Some("a short blurb"));
    }

    #[test]
    fn parse_description_returns_none_without_frontmatter() {
        assert_eq!(parse_description("# just a heading\n"), None);
    }
}
