//! `harness bakeoff` — run ONE coding brief through every available
//! [`CodeHarness`] adapter and compare the results (design.md §10: "test four
//! agent loops by running the SAME coding brief through each and comparing").
//!
//! This is the one **bold-to-live** surface of the otherwise behind-the-seam
//! harness module: it invokes the *real* agents (aider, claude, claude-deepseek,
//! pi). It is NOT wired into `run create` / the supervisor — it is a standalone,
//! explicitly-run comparison tool, never part of CI.
//!
//! For each selected adapter it:
//! 1. materialises an **isolated throwaway git repo** seeded with the same
//!    starting state (`--files`, committed as the base) so no adapter can see
//!    another's edits;
//! 2. runs the adapter on the brief with a wall-clock timeout;
//! 3. reads the outcome from git (via the adapter's [`CodeHarness::run_chunk`]),
//!    plus `git diff --numstat`, wall-clock time, best-effort token/cost usage,
//!    and whether the brief's self-checks pass;
//! 4. emits a one-row-per-adapter comparison (text table + `--json`).
//!
//! Adapters whose binary is not installed (or whose credential is absent) are
//! reported as `unavailable` with a reason — never a hard error, so a partial
//! toolbox still yields a useful comparison.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde::Serialize;

use super::aider::{AiderConfig, AiderHarness};
use super::claude::ClaudeHarness;
use super::pi::{PiConfig, PiHarness};
use super::support::git_bin;
use super::{
    CancelToken, Check, ChunkOutcome, ChunkRequest, ChunkResult, CodeHarness, HarnessError,
};
use crate::error::CliError;
use crate::output::{self, OutputFormat, OutputSpec};

/// Default per-adapter wall-clock ceiling when `--timeout` is not given. Generous
/// enough for a real agent to plan+edit+commit a small brief, bounded so a hung
/// agent cannot wedge the bake-off.
pub const DEFAULT_TIMEOUT_SECS: u64 = 600;

/// Parsed `harness bakeoff` arguments (mirrors the clap `BakeoffArgs`; kept as a
/// plain struct so the runner is unit-testable without clap).
pub struct BakeoffConfig {
    /// Path to the brief file (plain text, or JSON `{brief, checks?, files?}`).
    pub brief: PathBuf,
    /// Seed/scope files to copy into each throwaway repo (relative paths honoured).
    pub files: Vec<PathBuf>,
    /// Restrict to these adapter names (empty = all).
    pub only: Vec<String>,
    /// Per-adapter wall-clock ceiling.
    pub timeout: Duration,
}

/// Structured brief file: a plain-text brief, or JSON carrying explicit checks
/// and a declared file scope. Parsed leniently — a file that is not JSON is taken
/// as the brief text verbatim.
#[derive(Debug, Default, serde::Deserialize)]
struct BriefFile {
    brief: String,
    #[serde(default)]
    checks: Vec<CheckSpec>,
    #[serde(default)]
    files: Vec<PathBuf>,
}

/// A check in a JSON brief file (`timeout_ms` optional).
#[derive(Debug, serde::Deserialize)]
struct CheckSpec {
    id: String,
    #[serde(default)]
    desc: String,
    run: String,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

impl CheckSpec {
    fn into_check(self) -> Check {
        Check {
            id: self.id,
            desc: self.desc,
            run: self.run,
            timeout: self.timeout_ms.map(Duration::from_millis),
        }
    }
}

/// The fully-resolved brief: text + checks + declared scope.
struct ResolvedBrief {
    text: String,
    checks: Vec<Check>,
    scope: Vec<PathBuf>,
}

/// One adapter's row in the comparison.
#[derive(Debug, Serialize)]
struct AdapterRow {
    /// Adapter name (`aider`, `claude`, `claude-deepseek`, `pi`).
    name: String,
    /// `committed | no_change | failed | timeout | cancelled | unavailable | error`.
    status: String,
    /// Whether the adapter's binary (and credential) were present to run at all.
    available: bool,
    /// Reason for `unavailable`/`failed`/`error`, else `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    /// Resulting commit oid, when the adapter committed.
    #[serde(skip_serializing_if = "Option::is_none")]
    commit: Option<String>,
    /// Count of files changed (base..HEAD).
    files_changed: usize,
    /// Lines inserted (from `git diff --numstat`), when a commit was produced.
    #[serde(skip_serializing_if = "Option::is_none")]
    insertions: Option<u64>,
    /// Lines deleted (from `git diff --numstat`), when a commit was produced.
    #[serde(skip_serializing_if = "Option::is_none")]
    deletions: Option<u64>,
    /// Wall-clock time the adapter took, in milliseconds (absent when it never
    /// ran — `unavailable`).
    #[serde(skip_serializing_if = "Option::is_none")]
    wall_time_ms: Option<u64>,
    /// Token/cost usage, when the adapter reported it.
    #[serde(skip_serializing_if = "Option::is_none")]
    usage: Option<super::Usage>,
    /// Requested self-checks.
    checks_total: usize,
    /// Self-checks that passed.
    checks_passed: usize,
    /// Whether every requested check passed (false when none ran).
    checks_pass: bool,
}

/// The full comparison payload emitted under the success envelope's `data`.
#[derive(Debug, Serialize)]
struct BakeoffReport {
    /// The brief file that was run.
    brief_file: String,
    /// Number of adapters selected.
    selected: usize,
    /// One row per selected adapter, in registry order.
    adapters: Vec<AdapterRow>,
}

/// A selectable adapter: its stable name and a factory that builds the boxed
/// harness (deferred so an unavailable adapter is never constructed needlessly).
struct AdapterSpec {
    name: &'static str,
    /// The binary the adapter shells out to (resolved via its `OCTL_*_BIN`
    /// override) — probed for availability.
    bin: String,
    /// Credential env var that must be present, if any (aider/pi).
    credential_env: Option<&'static str>,
    /// Build the boxed harness.
    build: Box<dyn Fn() -> Box<dyn CodeHarness>>,
}

/// The canonical adapter registry (registry order = comparison order). Binaries
/// honour the same `OCTL_*_BIN` overrides the adapters use, so a bake-off can be
/// exercised end-to-end against fixture scripts with no network.
fn registry() -> Vec<AdapterSpec> {
    vec![
        AdapterSpec {
            name: "aider",
            bin: env_bin("OCTL_AIDER_BIN", "aider"),
            credential_env: Some("DEEPSEEK_API_KEY"),
            build: Box::new(|| {
                Box::new(AiderHarness::new(AiderConfig::new(
                    "deepseek/deepseek-chat",
                )))
            }),
        },
        AdapterSpec {
            name: "claude",
            bin: env_bin("OCTL_CLAUDE_BIN", "claude"),
            // Ambient Claude Code login — no credential env to probe.
            credential_env: None,
            build: Box::new(|| Box::new(ClaudeHarness::claude(None))),
        },
        AdapterSpec {
            name: "claude-deepseek",
            bin: env_bin("OCTL_CLAUDE_DEEPSEEK_BIN", "claude-deepseek"),
            // The wrapper sources its own key from SOPS — no env to probe.
            credential_env: None,
            build: Box::new(|| Box::new(ClaudeHarness::deepseek("flash"))),
        },
        AdapterSpec {
            name: "pi",
            bin: env_bin("OCTL_PI_BIN", "pi"),
            credential_env: Some("DEEPSEEK_API_KEY"),
            build: Box::new(|| Box::new(PiHarness::new(PiConfig::deepseek("deepseek-v4-flash")))),
        },
    ]
}

/// Resolve a binary name honouring an `OCTL_*_BIN` override.
fn env_bin(var: &str, default: &str) -> String {
    std::env::var(var).unwrap_or_else(|_| default.to_string())
}

/// Entry point for `harness bakeoff`.
pub fn run(cfg: &BakeoffConfig, spec: &OutputSpec, warnings: &[String]) -> Result<(), CliError> {
    let brief = resolve_brief(cfg)?;

    // Filter the registry by `--only` (validating each name so a typo is a clear
    // error, not a silently-empty comparison).
    let all = registry();
    let known: Vec<&str> = all.iter().map(|a| a.name).collect();
    for name in &cfg.only {
        if !known.contains(&name.as_str()) {
            return Err(CliError::user(
                "unknown_adapter",
                format!(
                    "unknown adapter '{name}'; known adapters: {}",
                    known.join(", ")
                ),
            )
            .with_invalid_value(name.clone()));
        }
    }
    let selected: Vec<AdapterSpec> = all
        .into_iter()
        .filter(|a| cfg.only.is_empty() || cfg.only.iter().any(|n| n == a.name))
        .collect();

    let mut rows = Vec::with_capacity(selected.len());
    for adapter in &selected {
        rows.push(run_one(adapter, &brief, cfg.timeout));
    }

    let report = BakeoffReport {
        brief_file: cfg.brief.display().to_string(),
        selected: rows.len(),
        adapters: rows,
    };

    match spec.format {
        OutputFormat::Json | OutputFormat::Jsonl => output::emit_envelope(&report, spec, warnings)?,
        OutputFormat::Text => {
            print_table(&report);
            output::emit_text_warnings(warnings);
        }
    }
    Ok(())
}

/// Read + resolve the brief file into text, checks, and file scope. A JSON file
/// with a `brief` field carries explicit checks/scope; any other content is the
/// plain brief text. `--files` are unioned into the scope either way.
fn resolve_brief(cfg: &BakeoffConfig) -> Result<ResolvedBrief, CliError> {
    let contents = std::fs::read_to_string(&cfg.brief).map_err(|e| {
        CliError::user(
            "brief_unreadable",
            format!("could not read brief file {}: {e}", cfg.brief.display()),
        )
    })?;

    // A file that *looks* structured (starts with `{`, or has a `.json`
    // extension) must parse as the JSON schema — a typo'd field is a hard error,
    // NOT a silent fall-back to feeding the raw JSON text to the agent (which
    // would drop the declared checks + scope without a word).
    let looks_json = contents.trim_start().starts_with('{')
        || cfg
            .brief
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("json"));
    let parsed: BriefFile = match serde_json::from_str::<BriefFile>(&contents) {
        Ok(b) => b,
        Err(e) if looks_json => {
            return Err(CliError::user(
                "invalid_brief",
                format!(
                    "brief file {} looks like JSON but does not match the \
                     {{\"brief\":…, \"checks\":[…], \"files\":[…]}} schema: {e}",
                    cfg.brief.display()
                ),
            ));
        }
        // Not a structured JSON brief → the whole file is the brief text.
        Err(_) => BriefFile {
            brief: contents,
            checks: Vec::new(),
            files: Vec::new(),
        },
    };

    if parsed.brief.trim().is_empty() {
        return Err(CliError::user(
            "empty_brief",
            format!("brief file {} is empty", cfg.brief.display()),
        ));
    }

    // Scope = brief-declared files ∪ CLI `--files`, de-duplicated in first-seen
    // order so the row's file count is stable.
    let mut scope: Vec<PathBuf> = Vec::new();
    for f in parsed.files.iter().chain(cfg.files.iter()) {
        if !scope.contains(f) {
            scope.push(f.clone());
        }
    }

    Ok(ResolvedBrief {
        text: parsed.brief,
        checks: parsed
            .checks
            .into_iter()
            .map(CheckSpec::into_check)
            .collect(),
        scope,
    })
}

/// Probe + run one adapter, mapping every outcome (including "not installed") to
/// an [`AdapterRow`] — never propagates an error, so one adapter's failure never
/// aborts the comparison.
fn run_one(adapter: &AdapterSpec, brief: &ResolvedBrief, timeout: Duration) -> AdapterRow {
    // Availability: the binary must be resolvable, and any required credential
    // present. Either absence is `unavailable` (not an error) — a partial
    // toolbox still produces a useful comparison.
    if !binary_available(&adapter.bin) {
        return unavailable_row(adapter.name, format!("binary '{}' not found", adapter.bin));
    }
    if let Some(var) = adapter.credential_env {
        // Non-empty check (a bare presence check would pass `VAR=""` and then the
        // adapter's own fast-fail would still reject it — inconsistent).
        if !super::support::credential_present(var) {
            return unavailable_row(
                adapter.name,
                format!("credential env var `{var}` is not set"),
            );
        }
    }

    // Materialise a fresh throwaway repo seeded with the scope files.
    let repo = match seed_repo(brief) {
        Ok(r) => r,
        Err(e) => return error_row(adapter.name, format!("could not seed throwaway repo: {e}")),
    };
    let base_commit = match head_oid(repo.path()) {
        Ok(c) => c,
        Err(e) => return error_row(adapter.name, format!("could not read base commit: {e}")),
    };

    let req = ChunkRequest {
        run_id: format!("bakeoff-{}", adapter.name),
        chunk_id: "brief".to_string(),
        attempt_id: "a1".to_string(),
        worktree_path: repo.path().to_path_buf(),
        base_commit: base_commit.clone(),
        plan_rev: "bakeoff".to_string(),
        brief: brief.text.clone(),
        checks: brief.checks.clone(),
        files: brief.scope.clone(),
        timeout: Some(timeout),
    };

    let harness = (adapter.build)();
    let cancel = CancelToken::new();
    let start = Instant::now();
    let result = harness.run_chunk(&req, &cancel);
    let elapsed_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);

    match result {
        Ok(res) => outcome_row(adapter.name, &res, repo.path(), &base_commit, elapsed_ms),
        Err(HarnessError::MissingCredential { var }) => unavailable_row(
            adapter.name,
            format!("credential env var `{var}` is not set"),
        ),
        Err(e) => {
            let mut row = error_row(adapter.name, e.to_string());
            row.wall_time_ms = Some(elapsed_ms);
            row
        }
    }
}

/// Build the row for a completed [`ChunkResult`], enriching it with diff stats.
fn outcome_row(
    name: &str,
    res: &ChunkResult,
    repo: &Path,
    base_commit: &str,
    elapsed_ms: u64,
) -> AdapterRow {
    let (status, reason) = match &res.outcome {
        ChunkOutcome::Committed { .. } => ("committed".to_string(), None),
        ChunkOutcome::NoChange => ("no_change".to_string(), None),
        ChunkOutcome::Failed { reason } => ("failed".to_string(), Some(reason.clone())),
        ChunkOutcome::Timeout => ("timeout".to_string(), None),
        ChunkOutcome::Cancelled => ("cancelled".to_string(), None),
    };

    // Diff stats only make sense (and only diff cleanly) for a committed result.
    let (insertions, deletions) = match &res.resulting_commit {
        Some(commit) => {
            numstat(repo, base_commit, commit).map_or((None, None), |(i, d)| (Some(i), Some(d)))
        }
        None => (None, None),
    };

    let checks_total = res.check_results.len();
    let checks_passed = res.check_results.iter().filter(|c| c.passed).count();

    AdapterRow {
        name: name.to_string(),
        status,
        available: true,
        reason,
        commit: res.resulting_commit.clone(),
        files_changed: res.changed_files.len(),
        insertions,
        deletions,
        wall_time_ms: Some(elapsed_ms),
        usage: res.usage.clone(),
        checks_total,
        checks_passed,
        // "checks pass" means every requested check ran and passed; with no
        // checks there is nothing to certify, so it is not a pass.
        checks_pass: checks_total > 0 && checks_passed == checks_total,
    }
}

/// A row for an adapter that could not run (not installed / missing credential).
fn unavailable_row(name: &str, reason: String) -> AdapterRow {
    AdapterRow {
        name: name.to_string(),
        status: "unavailable".to_string(),
        available: false,
        reason: Some(reason),
        commit: None,
        files_changed: 0,
        insertions: None,
        deletions: None,
        wall_time_ms: None,
        usage: None,
        checks_total: 0,
        checks_passed: 0,
        checks_pass: false,
    }
}

/// A row for an adapter that was available but hit a harness-level error (spawn
/// failure, dirty/invalid worktree, …). Distinct from `unavailable`: the tool was
/// present but the drive failed.
fn error_row(name: &str, reason: String) -> AdapterRow {
    AdapterRow {
        name: name.to_string(),
        status: "error".to_string(),
        available: true,
        reason: Some(reason),
        commit: None,
        files_changed: 0,
        insertions: None,
        deletions: None,
        wall_time_ms: None,
        usage: None,
        checks_total: 0,
        checks_passed: 0,
        checks_pass: false,
    }
}

/// Create a fresh throwaway git repo in a temp dir, copy in the scope files that
/// exist on disk (preserving their relative path), and commit them as the base so
/// every adapter forks from identical state. Files that don't exist on disk are
/// declared scope only (targets the brief will create) — not copied.
fn seed_repo(brief: &ResolvedBrief) -> Result<tempfile::TempDir, String> {
    let dir = tempfile::Builder::new()
        .prefix("octl-bakeoff-")
        .tempdir()
        .map_err(|e| e.to_string())?;
    let root = dir.path();

    git_seed(root, &["init", "-q", "-b", "main"])?;
    git_seed(root, &["config", "user.email", "bakeoff@orchestratectl"])?;
    git_seed(root, &["config", "user.name", "orchestratectl bakeoff"])?;

    let mut seeded_any = false;
    for rel in &brief.scope {
        // Reject any path with a root/prefix/parent component so a brief can't
        // write outside the throwaway repo. `RootDir`/`Prefix` (not just
        // `is_absolute()`) also catch Windows root-anchored (`\x`, `C:x`) forms
        // that `Path::join` would treat as a replacement.
        use std::path::Component;
        if rel.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        }) {
            return Err(format!("scope path {} escapes the repo", rel.display()));
        }
        // Inspect the source WITHOUT following a final symlink: a scope entry that
        // is a symlink to (or a real path pointing at) a file outside the repo
        // would otherwise be dereferenced by `fs::copy` and its content seeded
        // into the repo — and then shipped to a remote provider. Only a real
        // regular file is copied; a directory (which `fs::copy` can't handle) or a
        // symlink is a clear error, not an EISDIR/leak.
        let Ok(meta) = std::fs::symlink_metadata(rel) else {
            // Not present on disk → a target the brief will create; scope-only.
            continue;
        };
        if meta.file_type().is_symlink() {
            return Err(format!(
                "scope path {} is a symlink; refusing to seed it",
                rel.display()
            ));
        }
        if !meta.is_file() {
            return Err(format!(
                "scope path {} is not a regular file (a directory?); seed only files",
                rel.display()
            ));
        }
        let dest = root.join(rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::copy(rel, &dest)
            .map_err(|e| format!("copy {} -> {}: {e}", rel.display(), dest.display()))?;
        seeded_any = true;
    }

    if seeded_any {
        git_seed(root, &["add", "-A"])?;
        git_seed(root, &["commit", "-q", "-m", "bakeoff: seed"])?;
    } else {
        // No seed content: an empty root commit so `base_commit` resolves and the
        // adapters' base-check passes.
        git_seed(
            root,
            &["commit", "-q", "--allow-empty", "-m", "bakeoff: empty seed"],
        )?;
    }
    Ok(dir)
}

/// Run a git subcommand while seeding, mapping failure to a string.
fn git_seed(root: &Path, args: &[&str]) -> Result<(), String> {
    let out = Command::new(git_bin())
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|e| format!("git {}: {e}", args.join(" ")))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// Current HEAD oid of a repo.
fn head_oid(repo: &Path) -> Result<String, String> {
    let out = Command::new(git_bin())
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Summed `(insertions, deletions)` from `git diff --numstat base..commit`.
/// Binary files (numstat `-`/`-`) contribute nothing. Returns `None` on a git
/// failure rather than fabricating zeros.
fn numstat(repo: &Path, base: &str, commit: &str) -> Option<(u64, u64)> {
    let out = Command::new(git_bin())
        .arg("-C")
        .arg(repo)
        .args(["diff", "--numstat", &format!("{base}..{commit}")])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut ins = 0u64;
    let mut del = 0u64;
    for line in text.lines() {
        let mut cols = line.split('\t');
        let a = cols.next().unwrap_or("-");
        let b = cols.next().unwrap_or("-");
        ins += a.parse::<u64>().unwrap_or(0);
        del += b.parse::<u64>().unwrap_or(0);
    }
    Some((ins, del))
}

/// Whether `bin` can be executed: an absolute/relative path is checked directly;
/// a bare name is searched on `PATH`.
fn binary_available(bin: &str) -> bool {
    let p = Path::new(bin);
    if p.components().count() > 1 || p.is_absolute() {
        return is_executable(p);
    }
    let Ok(path) = std::env::var("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| is_executable(&dir.join(bin)))
}

/// Whether `p` is a regular file with an owner/group/other execute bit.
fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// Render the human-readable comparison table (`--output text`).
fn print_table(report: &BakeoffReport) {
    println!("harness bakeoff — {}", report.brief_file);
    println!(
        "{:<16} {:<12} {:>6} {:>10} {:>8} {:>10} {:>8}",
        "ADAPTER", "OUTCOME", "FILES", "+/-", "TIME", "COST", "CHECKS"
    );
    for a in &report.adapters {
        let plusminus = match (a.insertions, a.deletions) {
            (Some(i), Some(d)) => format!("+{i}/-{d}"),
            _ => "-".to_string(),
        };
        let time = a.wall_time_ms.map_or_else(
            || "-".to_string(),
            |ms| format!("{:.1}s", ms as f64 / 1000.0),
        );
        let cost = a
            .usage
            .as_ref()
            .and_then(|u| u.cost_usd)
            .map_or_else(|| "-".to_string(), |c| format!("${c:.4}"));
        let checks = if a.checks_total == 0 {
            "-".to_string()
        } else {
            let mark = if a.checks_pass { "ok" } else { "fail" };
            format!("{}/{} {mark}", a.checks_passed, a.checks_total)
        };
        println!(
            "{:<16} {:<12} {:>6} {:>10} {:>8} {:>10} {:>8}",
            a.name, a.status, a.files_changed, plusminus, time, cost, checks
        );
    }
    for a in &report.adapters {
        if let Some(reason) = &a.reason {
            println!(
                "  {} — {}: {}",
                a.name,
                a.status,
                output::escape_one_line(reason)
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::TempDir;

    fn write_exec(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        let mut perms = std::fs::metadata(&p).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&p, perms).unwrap();
        p
    }

    #[test]
    fn binary_available_finds_path_and_missing() {
        let dir = TempDir::new().unwrap();
        let bin = write_exec(dir.path(), "yes-i-exist", "#!/bin/sh\ntrue\n");
        assert!(binary_available(bin.to_str().unwrap()));
        assert!(!binary_available(
            &dir.path().join("nope").display().to_string()
        ));
    }

    #[test]
    fn resolve_brief_plain_text() {
        let dir = TempDir::new().unwrap();
        let f = dir.path().join("brief.md");
        std::fs::write(&f, "implement the widget").unwrap();
        let cfg = BakeoffConfig {
            brief: f,
            files: vec![PathBuf::from("a.txt")],
            only: vec![],
            timeout: Duration::from_secs(1),
        };
        let b = resolve_brief(&cfg).unwrap();
        assert_eq!(b.text, "implement the widget");
        assert!(b.checks.is_empty());
        assert_eq!(b.scope, vec![PathBuf::from("a.txt")]);
    }

    #[test]
    fn resolve_brief_json_with_checks_and_scope() {
        let dir = TempDir::new().unwrap();
        let f = dir.path().join("brief.json");
        std::fs::write(
            &f,
            r#"{"brief":"do X","checks":[{"id":"c1","desc":"builds","run":"true","timeout_ms":500}],"files":["src/x.rs"]}"#,
        )
        .unwrap();
        let cfg = BakeoffConfig {
            brief: f,
            files: vec![PathBuf::from("src/x.rs"), PathBuf::from("extra.rs")],
            only: vec![],
            timeout: Duration::from_secs(1),
        };
        let b = resolve_brief(&cfg).unwrap();
        assert_eq!(b.text, "do X");
        assert_eq!(b.checks.len(), 1);
        assert_eq!(b.checks[0].id, "c1");
        assert_eq!(b.checks[0].timeout, Some(Duration::from_millis(500)));
        // Union, de-duplicated, brief files first.
        assert_eq!(
            b.scope,
            vec![PathBuf::from("src/x.rs"), PathBuf::from("extra.rs")]
        );
    }

    #[test]
    fn resolve_brief_empty_is_error() {
        let dir = TempDir::new().unwrap();
        let f = dir.path().join("brief.md");
        std::fs::write(&f, "   \n").unwrap();
        let cfg = BakeoffConfig {
            brief: f,
            files: vec![],
            only: vec![],
            timeout: Duration::from_secs(1),
        };
        assert!(resolve_brief(&cfg).is_err());
    }

    #[test]
    fn seed_repo_empty_makes_base_commit() {
        let brief = ResolvedBrief {
            text: "x".into(),
            checks: vec![],
            scope: vec![],
        };
        let repo = seed_repo(&brief).unwrap();
        // A base commit resolves even with no seed files.
        assert!(head_oid(repo.path()).is_ok());
    }

    #[test]
    fn seed_repo_rejects_escaping_scope() {
        let brief = ResolvedBrief {
            text: "x".into(),
            checks: vec![],
            scope: vec![PathBuf::from("../escape.txt")],
        };
        assert!(seed_repo(&brief).is_err());
    }

    #[test]
    fn numstat_sums_insertions_and_deletions() {
        // A real repo with a base + a commit adding two lines.
        let repo = seed_repo(&ResolvedBrief {
            text: "x".into(),
            checks: vec![],
            scope: vec![],
        })
        .unwrap();
        let base = head_oid(repo.path()).unwrap();
        std::fs::write(repo.path().join("f.txt"), "a\nb\n").unwrap();
        git_seed(repo.path(), &["add", "-A"]).unwrap();
        git_seed(repo.path(), &["commit", "-q", "-m", "add"]).unwrap();
        let head = head_oid(repo.path()).unwrap();
        assert_eq!(numstat(repo.path(), &base, &head), Some((2, 0)));
    }
}
