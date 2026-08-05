//! `run` subcommand — top-level lifecycle for orchestratectl runs.
//!
//! Sets the noun-module pattern that `node`, `event`, `discussion`,
//! `spinoff` will follow: one file per verb, shared types in `mod.rs`,
//! single `dispatch` entry point called from `cli.rs`.

pub mod cancel;
pub mod create;
pub mod dto;
pub mod landed;
pub mod list;
pub mod merge;
pub mod reattach;
pub mod show;
pub mod spawn;
pub mod supervisor_readiness;
pub mod supervisor_spawn;
pub mod wait;

use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::{Subcommand, ValueEnum};
use octl_core::{
    is_run_id_prefix, DiscussionId, IdValidationError, Kind, Lifecycle, NodeId, ProposalId, RunId,
    RunPaths,
};

use crate::error::CliError;
use crate::output::OutputSpec;

#[derive(Debug, Clone, Copy, ValueEnum)]
#[clap(rename_all = "kebab-case")]
pub enum KindArg {
    Code,
    Spinoff,
    Orchestrated,
    Research,
    TechnicalDecision,
    MakeSkill,
    FanOut,
    Bugfix,
    Orchestrate,
}

impl From<KindArg> for Kind {
    fn from(k: KindArg) -> Self {
        match k {
            KindArg::Code => Kind::Code,
            KindArg::Spinoff => Kind::Spinoff,
            KindArg::Orchestrated => Kind::Orchestrated,
            KindArg::Research => Kind::Research,
            KindArg::TechnicalDecision => Kind::TechnicalDecision,
            KindArg::MakeSkill => Kind::MakeSkill,
            KindArg::FanOut => Kind::FanOut,
            KindArg::Bugfix => Kind::Bugfix,
            KindArg::Orchestrate => Kind::Orchestrate,
        }
    }
}

#[derive(Subcommand, Debug)]
pub enum RunAction {
    /// Create a new run. Top-level when `--parent-*` flags are absent,
    /// child-spawn when both are set (mutually required).
    Create {
        #[arg(long, value_enum)]
        kind: KindArg,
        #[arg(long)]
        title: String,
        #[arg(long)]
        source_repo: Option<String>,
        #[arg(long)]
        source_branch: Option<String>,
        #[arg(long, conflicts_with = "prompt_file")]
        task: Option<String>,
        /// Path to a prompt file (instead of inlining via --task). Used
        /// as-is and handed to create.sh.
        #[arg(long)]
        prompt_file: Option<String>,
        /// Workmux layout name; forwarded to create.sh as `-l <name>`.
        #[arg(long)]
        layout: Option<String>,
        /// Skip workmux post-create hooks; forwarded to create.sh.
        #[arg(long)]
        no_hooks: bool,
        /// Spawn the worker's tmux window in a detached "headless"
        /// session instead of the foreground one, so a campaign of many
        /// spawns does not clutter the user's window list. Attach later
        /// with `tmux attach -t headless`. Opt-in; default is foreground.
        #[arg(long)]
        headless: bool,
        /// Explicit tmux session name for the worker's window. Implies
        /// headless placement and overrides `--headless`'s default
        /// session name. Forwarded to create.sh / workmux as
        /// `--parent-session <name>`.
        #[arg(long)]
        tmux_session: Option<String>,
        /// Seconds create.sh waits for the freshly launched agent to
        /// become discoverable before giving up (forwarded as
        /// `--agent-startup-timeout <seconds>`). Range 1–600. Defaults to
        /// 90 — higher than create.sh's own 30s default because octl
        /// spawns are frequently part of high-fan-out batches that
        /// self-load the host; bump it further on an already-loaded box.
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=600), default_value_t = 90)]
        agent_startup_timeout: u32,
        #[arg(long, requires = "parent_node_id")]
        parent_run_id: Option<String>,
        #[arg(long, requires = "parent_run_id")]
        parent_node_id: Option<String>,
        /// Shell command the supervisor runs when this run reaches a terminal
        /// state (`done | failed | cancelled`), BEFORE teardown. Runs via
        /// `sh -c <cmd>` with `OCTL_RUN_ID`, `OCTL_STATUS`, `OCTL_SUMMARY`,
        /// `OCTL_RUN_KIND`, and `OCTL_RUN_TITLE` in the environment — so a
        /// spawning session can learn of completion without polling (e.g.
        /// append a line to a file the harness watches, or post a desktop
        /// notification). At-least-once: deduped on a durable `run.notified`
        /// marker (so the healthy path fires once), but a supervisor crash in
        /// the window between firing and recording the marker re-fires on
        /// restart — a duplicate is preferred over a missed notification, so
        /// the command should tolerate running more than once.
        #[arg(long)]
        notify: Option<String>,
        #[arg(long)]
        idempotency_key: Option<String>,
        #[arg(long)]
        dry_run: bool,
        /// **Test-only.** Skip the create.sh shell-out and supervisor
        /// spawn; produce only the on-disk run skeleton (manifest +
        /// run.created event). Hidden from `--help`. Never set this in
        /// production — the run will be missing its worktree, tmux
        /// window, and supervisor.
        #[arg(long, hide = true)]
        skip_materialize: bool,
    },
    /// List runs on disk.
    List {
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        kind: Option<String>,
    },
    /// Show one run's manifest and counters.
    Show { run_id: String },
    /// Cancel a run: synthesize terminal `node.report` for non-terminal
    /// nodes, emit `run.status: cancelled`. Idempotent.
    Cancel {
        run_id: String,
        #[arg(long)]
        note: Option<String>,
    },
    /// Merge a worktree run's branch back to its source, then submit the
    /// terminal `node.report` so the supervisor winds the run down and
    /// tears the worktree/window/branch down. Owns the full merge
    /// lifecycle: rebase + merge (via the bundled merge backend) AND the
    /// report, in one call.
    Merge {
        run_id: String,
        /// Merge target branch. Defaults to the run's recorded
        /// `source_branch`, then to main/master auto-detection.
        #[arg(long)]
        source: Option<String>,
        /// Reporting node id (defaults to `n-0001`).
        #[arg(long)]
        node_id: Option<String>,
        /// Optional §7.3 report payload (JSON file) to submit on a clean
        /// merge. Lets an autonomous kind carry its rich `discussion_items`
        /// / `spinoff_proposals` / `wrap_up_recommendations` in the same
        /// call. `run merge` stamps it `via: "explicit-merge"`. Omit it for
        /// a minimal `{success, summary}` report.
        #[arg(long)]
        report_file: Option<std::path::PathBuf>,
        /// Human-reviewer acknowledgement, required to merge a `code` run (the
        /// `/worktree-merge` workflow supplies it). No-op for other kinds.
        ///
        /// Hidden from `--help`: a `code` run's whole purpose is the human review
        /// gate before landing, so the coding agent must NOT discover and pass
        /// this to self-merge (issue `interactive-code-run-self-merged`). The
        /// human's path documents it in the `worktree-merge` skill.
        #[arg(long, hide = true)]
        confirm_interactive: bool,
        /// Resolve inputs and report the planned merge without running it
        /// or appending any event.
        #[arg(long)]
        dry_run: bool,
    },
    /// Block until one or more runs reach a terminal state
    /// (`done | failed | cancelled`) and emit a structured summary, so
    /// callers stop hand-rolling `run show` poll loops. Read-only: never
    /// mutates run state. Exit codes: `0` condition met, `1` usage/unknown
    /// run, `2` timeout, `3` (`--fail-on-error`) a settled run failed.
    Wait {
        /// One or more run ids to wait on.
        #[arg(required = true, num_args = 1..)]
        run_id: Vec<String>,
        /// Return once *every* listed run is terminal (default).
        #[arg(long, conflicts_with = "any")]
        all: bool,
        /// Return as soon as *one* listed run is terminal.
        #[arg(long)]
        any: bool,
        /// Give up after this duration (e.g. `30s`, `5m`, `1h`); exit code
        /// `2` distinguishes timeout from a met condition. Defaults to `6h`
        /// — a sane ceiling so a wait on a stuck run can never block an
        /// orchestrator forever; pass a larger value for a long campaign.
        #[arg(long, value_parser = wait::parse_duration, default_value = "6h")]
        timeout: Option<Duration>,
        /// Exit `3` if the condition is met but a settled run was
        /// `failed`/`cancelled` (default: exit `0` for any terminal state).
        #[arg(long)]
        fail_on_error: bool,
        /// Emit one JSONL line per run state-transition to stderr for live UIs.
        #[arg(long)]
        progress: bool,
        /// Override the internal poll cadence (default: bounded backoff,
        /// 100ms→2s). Callers shouldn't normally need this.
        #[arg(long, value_parser = wait::parse_duration)]
        poll_interval: Option<Duration>,
    },
    /// Restart the run's supervisor process. Refuses if the recorded
    /// supervisor PID is still alive. Spawns `orchestratectl supervise
    /// <run-id>` detached with stdout/stderr → `supervisor.stderr.log`.
    Reattach {
        run_id: String,
        /// Pass `--once` to the spawned supervisor (test-only).
        #[arg(long, hide = true)]
        once: bool,
        /// Pass `--max-iter <n>` to the spawned supervisor (test-only).
        #[arg(long, hide = true)]
        max_iter: Option<u32>,
    },
}

pub fn dispatch(action: RunAction, spec: &OutputSpec, warnings: &[String]) -> Result<(), CliError> {
    match action {
        RunAction::Create {
            kind,
            title,
            source_repo,
            source_branch,
            task,
            prompt_file,
            layout,
            no_hooks,
            headless,
            tmux_session,
            agent_startup_timeout,
            parent_run_id,
            parent_node_id,
            notify,
            idempotency_key,
            dry_run,
            skip_materialize,
        } => create::run(create::Args {
            skip_materialize,
            kind: kind.into(),
            title,
            source_repo,
            source_branch,
            task,
            prompt_file,
            layout,
            no_hooks,
            headless,
            tmux_session,
            agent_startup_timeout,
            parent_run_id,
            parent_node_id,
            notify,
            idempotency_key,
            dry_run,
            spec,
            warnings,
        }),
        RunAction::List { status, kind } => list::run(list::Args {
            status,
            kind,
            spec,
            warnings,
        }),
        RunAction::Show { run_id } => show::run(&run_id, spec, warnings),
        RunAction::Cancel { run_id, note } => cancel::run(&run_id, note.as_deref(), spec, warnings),
        RunAction::Merge {
            run_id,
            source,
            node_id,
            report_file,
            confirm_interactive,
            dry_run,
        } => merge::run(merge::Args {
            run_id,
            source,
            node_id,
            report_file,
            confirm_interactive,
            dry_run,
            spec,
            warnings,
        }),
        RunAction::Wait {
            run_id,
            all: _,
            any,
            timeout,
            fail_on_error,
            progress,
            poll_interval,
        } => wait::run(wait::Args {
            run_ids: run_id,
            any,
            timeout,
            fail_on_error,
            progress,
            poll_interval,
            spec,
            warnings,
        }),
        RunAction::Reattach {
            run_id,
            once,
            max_iter,
        } => reattach::run(&run_id, once, max_iter, spec, warnings),
    }
}

/// Map a `Kind` to its default `Lifecycle`. Thin alias over
/// [`Kind::lifecycle`] so CLI call sites keep their existing
/// free-function spelling while the source of truth lives in core.
pub fn lifecycle_for(k: Kind) -> Lifecycle {
    k.lifecycle()
}

/// Resolve a run-id argument — which may be an unambiguous *prefix* (like `git`
/// short SHAs) — to a typed, validated [`RunId`].
///
/// - A full-length value must be an exact valid ULID; it is returned verbatim
///   with no directory scan, so the existing exact-id behaviour (and each
///   caller's own valid-but-missing `run_not_found`) is preserved unchanged, and
///   the supervisor's hot lookups (which always pass full child ids) pay no scan.
/// - A shorter value is treated as a prefix and matched against the run
///   directories under `<root>/runs/`: exactly one match resolves; several match
///   surfaces `ambiguous_run_id` (listing the candidates in `expected`); none
///   match surfaces `run_not_found`.
/// - A malformed value (empty, non-Crockford char, impossible leading digit,
///   over-length non-ULID) surfaces `invalid_run_id`, keeping a typo distinct
///   from a well-formed-but-unknown prefix.
///
/// The prefix scan is a best-effort read of the runs directory, deliberately NOT
/// under a namespace lock (no such lock exists — a run is not known until after
/// the scan). It fails *closed*: a `read_dir` iteration error propagates as
/// `io_error` rather than dropping a candidate, so an ambiguous prefix can never
/// be silently narrowed to a single (wrong) match. A run created or torn down
/// concurrently with the scan is an accepted race — a resolved id that is then
/// deleted before the caller locks it surfaces as the caller's own
/// `run_not_found` (see e.g. `cancel`'s `NotFound` handling).
pub fn resolve_run_id_arg(root: &Path, arg: &str) -> Result<RunId, CliError> {
    // Full-length (or longer): must be an exact ULID. A 26-char string that is
    // not a valid ULID (wrong charset, timestamp overflow) stays `invalid_run_id`
    // rather than being reinterpreted as a length-26 prefix that matches nothing.
    if arg.len() >= RunId::LEN {
        return RunId::parse_str(arg).map_err(|e| {
            CliError::user(
                "invalid_run_id",
                format!("run id {arg:?} is not a valid ULID: {e}"),
            )
            .with_invalid_value(arg)
        });
    }
    // Shorter than a ULID: a prefix. Reject a malformed prefix up front so a typo
    // is `invalid_run_id`, not a silent no-match.
    if !is_run_id_prefix(arg) {
        return Err(CliError::user(
            "invalid_run_id",
            format!(
                "run id {arg:?} is not a valid ULID or run-id prefix: \
                 expected up to {} lowercase Crockford base32 characters (leading 0-7)",
                RunId::LEN
            ),
        )
        .with_invalid_value(arg));
    }
    let runs_dir = runs_root(root);
    let entries = match std::fs::read_dir(&runs_dir) {
        Ok(entries) => entries,
        // No runs dir yet ⇒ no run can match ⇒ not-found (not a system error).
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(prefix_not_found(arg)),
        Err(e) => {
            return Err(CliError::system(
                "io_error",
                format!("read_dir {}: {}", runs_dir.display(), e),
            ))
        }
    };
    // Fail closed: propagate a per-entry iteration error instead of dropping the
    // entry (a dropped candidate could turn an ambiguous prefix into a falsely
    // unique one and then act on the wrong run).
    let mut matches: Vec<RunId> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| {
            CliError::system(
                "io_error",
                format!("read_dir {}: {}", runs_dir.display(), e),
            )
        })?;
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        // Match on the entry NAME being a valid run id sharing the prefix; a
        // foreign dir (non-ULID name) can never be resolved to. Entry type is not
        // filtered — an exact-id lookup lets `from_validated` surface a
        // `corrupt_run` for a symlinked run dir, so counting it as a candidate
        // keeps prefix and exact addressing consistent for that corruption case.
        if name.starts_with(arg) {
            if let Ok(rid) = RunId::parse_str(&name) {
                matches.push(rid);
            }
        }
    }
    match matches.len() {
        0 => Err(prefix_not_found(arg)),
        1 => Ok(matches.pop().expect("len checked == 1")),
        n => {
            // Sort only here, where the candidate list is actually emitted, so a
            // deterministic error is presented without paying for a sort on the
            // common unique / not-found paths.
            matches.sort();
            Err(CliError::user(
                "ambiguous_run_id",
                format!(
                    "run id prefix {arg:?} matches {n} runs; use more characters to disambiguate"
                ),
            )
            .with_invalid_value(arg)
            .with_expected(serde_json::Value::Array(
                matches
                    .into_iter()
                    .map(|r| serde_json::Value::String(r.as_str().to_string()))
                    .collect(),
            )))
        }
    }
}

/// `run_not_found` for a well-formed prefix that matched no run.
fn prefix_not_found(arg: &str) -> CliError {
    CliError::user(
        "run_not_found",
        format!("no run matching id prefix {arg:?}"),
    )
    .with_invalid_value(arg)
}

/// Resolve `<root>/runs/<run-id>` and return validated `RunPaths`.
///
/// Accepts an unambiguous run-id prefix as well as a full ULID (see
/// [`resolve_run_id_arg`]) — every run-id-taking subcommand routes through here,
/// so prefix acceptance is uniform across the CLI.
///
/// A malformed run-id is a distinct, machine-actionable error class from a
/// well-formed id that simply names no run, so it surfaces as `invalid_run_id`
/// carrying the core validator's reason (length, charset, ULID range) rather
/// than being collapsed into `run_not_found`. Callers that look a run up by id
/// still emit their own `run_not_found` for the valid-but-missing case.
pub fn run_paths(root: &Path, run_id: &str) -> Result<RunPaths, CliError> {
    // Resolve a prefix (if any) to a typed, already-validated run id; no re-parse
    // is needed — `run_dir` only accepts a `RunId`, so a `..`/absolute component
    // can never reach the filesystem.
    let rid = resolve_run_id_arg(root, run_id)?;
    let dir = octl_core::run_dir(root, &rid);
    // `from_validated` runs the symlink-root guard; a symlinked run dir maps to
    // the `corrupt_run` envelope rather than being silently followed.
    RunPaths::from_validated(dir, rid).map_err(from_core)
}

/// Trim a CLI string argument and reject empty/whitespace-only values.
pub fn require_nonempty(value: &str, field: &str) -> Result<String, CliError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(CliError::user(
            "invalid_value",
            format!("--{field} must not be empty or whitespace-only"),
        )
        .with_invalid_value(value));
    }
    Ok(trimmed.to_string())
}

/// Map an id-validation failure to the CLI's `invalid_id` error envelope,
/// carrying the offending value (`invalid_value`) and the accepted-shape hint
/// (`expected`, e.g. `n-NNNN`). This is the single boundary where a malformed
/// id surfaces to an AI caller; the typed newtype is the only thing a path
/// helper will accept downstream.
pub fn invalid_id(value: &str, err: &IdValidationError) -> CliError {
    CliError::user("invalid_id", err.to_string())
        .with_invalid_value(value)
        .with_expected(serde_json::Value::String(err.expected().to_string()))
}

/// Validate a `run_id` clap or event-data argument into a typed [`RunId`].
/// Most run-id call sites instead go through [`run_paths`], which both
/// validates and builds the [`RunPaths`]; use this when only validation of a
/// bare run-id string is needed (e.g. an event-data `child_run_id`).
pub fn parse_run_id(value: &str) -> Result<RunId, CliError> {
    RunId::parse_str(value).map_err(|e| invalid_id(value, &e))
}

/// Validate a `node_id` clap argument into a typed [`NodeId`] before it can
/// reach any path helper.
pub fn parse_node_id(value: &str) -> Result<NodeId, CliError> {
    NodeId::parse_str(value).map_err(|e| invalid_id(value, &e))
}

/// Validate a `discussion_id` clap argument into a typed [`DiscussionId`].
pub fn parse_discussion_id(value: &str) -> Result<DiscussionId, CliError> {
    DiscussionId::parse_str(value).map_err(|e| invalid_id(value, &e))
}

/// Validate a `proposal_id` clap argument into a typed [`ProposalId`].
pub fn parse_proposal_id(value: &str) -> Result<ProposalId, CliError> {
    ProposalId::parse_str(value).map_err(|e| invalid_id(value, &e))
}

/// Render a `Kind` as its canonical kebab-case wire string. Single
/// source of truth shared by every verb so create/list/show/json/text
/// stay aligned and adding a new kind only requires editing here.
pub fn kind_kebab(k: Kind) -> &'static str {
    match k {
        Kind::Code => "code",
        Kind::Spinoff => "spinoff",
        Kind::Orchestrated => "orchestrated",
        Kind::Research => "research",
        Kind::TechnicalDecision => "technical-decision",
        Kind::MakeSkill => "make-skill",
        Kind::FanOut => "fan-out",
        Kind::Bugfix => "bugfix",
        Kind::Orchestrate => "orchestrate",
    }
}

pub fn lifecycle_kebab(l: Lifecycle) -> &'static str {
    match l {
        Lifecycle::Autonomous => "autonomous",
        Lifecycle::Interactive => "interactive",
    }
}

pub fn status_kebab(s: octl_core::Status) -> &'static str {
    use octl_core::Status::{Blocked, Cancelled, Done, Failed, Pending, Running};
    match s {
        Pending => "pending",
        Running => "running",
        Blocked => "blocked",
        Done => "done",
        Failed => "failed",
        Cancelled => "cancelled",
    }
}

/// `<root>/runs/`.
pub fn runs_root(root: &Path) -> PathBuf {
    root.join("runs")
}

/// Map a `core::Error` into a `CliError`.
///
/// Every flavor of *corrupt persisted state* collapses into one non-retryable
/// `corrupt_state` user error (exit 1): a malformed `events.jsonl` line
/// ([`CorruptEventLog`]), a projection whose embedded id contradicts its path
/// ([`CorruptProjection`]), malformed state-file JSON ([`Json`]/[`JsonBare`]),
/// and a state file written by an unsupported build
/// ([`UnsupportedSchemaVersion`]). These are all data-integrity faults the
/// caller must investigate, not transient I/O to retry — surfacing them under
/// one user code (exit 1) keeps an AI caller's retry loop from hammering a file
/// that will never parse. Where the variant carries them, the two mismatched
/// ids / the bad-vs-supported schema versions ride along in `invalid_value` /
/// `expected` for the operator to diff.
///
/// A symlinked run dir, subdir, or state file is a separate tampered-run fault
/// (`corrupt_run`, exit 1). Everything else — genuine transient I/O — collapses
/// into the generic `io_error` system class (exit 2).
///
/// [`CorruptEventLog`]: octl_core::Error::CorruptEventLog
/// [`CorruptProjection`]: octl_core::Error::CorruptProjection
/// [`Json`]: octl_core::Error::Json
/// [`JsonBare`]: octl_core::Error::JsonBare
/// [`UnsupportedSchemaVersion`]: octl_core::Error::UnsupportedSchemaVersion
pub fn from_core(err: octl_core::Error) -> CliError {
    match err {
        octl_core::Error::CorruptEventLog { .. }
        | octl_core::Error::Json { .. }
        | octl_core::Error::JsonBare(_) => CliError::user("corrupt_state", err.to_string()),
        octl_core::Error::CorruptProjection {
            ref expected_id,
            ref body_id,
            ..
        } => {
            let (expected_id, body_id) = (expected_id.clone(), body_id.clone());
            CliError::user("corrupt_state", err.to_string())
                .with_invalid_value(body_id)
                .with_expected(serde_json::Value::String(expected_id))
        }
        octl_core::Error::UnsupportedSchemaVersion {
            found,
            ref supported,
            ..
        } => {
            let supported = supported.clone();
            CliError::user("corrupt_state", err.to_string())
                .with_invalid_value(found.to_string())
                .with_expected(serde_json::json!({ "supported_schema_versions": supported }))
        }
        // A symlinked run dir, subdir, or state file is a tampered or corrupted
        // run, not a transient I/O fault to retry — it surfaces as a distinct
        // `corrupt_run` user error (exit 1) so a retry loop doesn't chase a path
        // that will never be a regular file. The offending path rides along in
        // `invalid_value` so an operator can go straight to it.
        octl_core::Error::SymlinkRunDir { ref path }
        | octl_core::Error::SymlinkSubdir { ref path, .. }
        | octl_core::Error::SymlinkStateFile { ref path, .. } => {
            let path = path.display().to_string();
            CliError::user("corrupt_run", err.to_string()).with_invalid_value(path)
        }
        // An empty idempotency key is a caller/client error, not a system fault —
        // the CLI boundary normally rejects it first, so this is the core backstop.
        octl_core::Error::EmptyIdempotencyKey => {
            CliError::user("invalid_value", err.to_string()).with_invalid_value("")
        }
        other => CliError::system("io_error", other.to_string()),
    }
}
