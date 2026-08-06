//! `run wait` — block until one or more runs reach a terminal state.
//!
//! A blocking completion primitive so callers stop hand-rolling
//! `while ... run show ... case` poll loops (and re-introducing the
//! shell-portability / wrong-field bugs that motivated this issue). The
//! correct, tested loop lives here in the binary instead of in every
//! caller's shell.
//!
//! ## Implementation: thin poll (v0.1.0 first cut)
//!
//! This is the issue's "smallest viable first cut": an internal poll of
//! each run's `manifest.status` projection with bounded exponential
//! backoff. It is a strictly **read-only** caller of the existing
//! `manifest.status` projection — it never appends events, never spawns or
//! signals a supervisor, and never mutates run state. The supervisor knows
//! precisely when a run goes terminal; a future revision could subscribe to
//! the event stream for zero-lag wakeups, but the poll already removes the
//! whole bug class.
//!
//! Each per-run read wraps `manifest.json` (and, at the end, the terminal
//! `node` projection) in `RunLock::with_shared_lock` so a concurrent reducer
//! can never expose a half-applied projection set (CLAUDE.md state-integrity
//! invariant 3).
//!
//! ## Exit codes
//!
//! - `0` — wait condition satisfied (`--all` → every run terminal; `--any`
//!   → ≥1 terminal). With `--fail-on-error`, every settled run was `done`.
//! - `1` — usage / unknown run id / internal error (a `CliError`).
//! - `2` — `--timeout` reached before the wait condition was met.
//! - `3` — `--fail-on-error` and the wait condition was met but ≥1 settled
//!   run was `failed`/`cancelled`.
//!
//! Exit codes `2` and `3` still emit the normal success-shaped data
//! envelope on stdout (the summary is the answer); they cannot ride the
//! shared `Result<(), CliError>` path (which emits an *error* envelope on
//! stderr), so they flush logs and `std::process::exit` directly, mirroring
//! `event tail`'s signal-exit precedent.

use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;

use octl_core::{read_manifest_opt, read_node_opt, NodeId, RunLock, RunPaths, Status};

use crate::error::CliError;
use crate::output::{self, OutputFormat, OutputSpec};
use crate::run::{from_core, run_paths_from_cli_arg, status_kebab};

/// Reporting node whose terminal `node.report` carries the run's outcome
/// summary. Every single-worker worktree kind has exactly one node
/// (`n-0001`); mirrors `run merge`'s `DEFAULT_NODE_ID`.
const DEFAULT_NODE_ID: &str = "n-0001";

/// Backoff cadence (documented at the call site in [`wait_loop`]): start at
/// 100ms and double each idle poll up to a 2s ceiling. Snappy for the common
/// case (a run that settles within a second or two of the call) without
/// hammering the filesystem on a long wait.
const BACKOFF_START: Duration = Duration::from_millis(100);
const BACKOFF_CAP: Duration = Duration::from_secs(2);

/// Which settle condition ends the wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Condition {
    /// Return once *every* listed run is terminal.
    All,
    /// Return as soon as *one* listed run is terminal.
    Any,
}

impl Condition {
    fn wire(self) -> &'static str {
        match self {
            Condition::All => "all",
            Condition::Any => "any",
        }
    }
}

pub struct Args<'a> {
    pub run_ids: Vec<String>,
    /// `--any` was passed (return on the first terminal run). When false the
    /// default `--all` applies.
    pub any: bool,
    /// Wait budget. The CLI now supplies a `6h` default (see `run::mod`'s
    /// `--timeout` arg) so a stuck run can't block an orchestrator forever;
    /// `None` (only reachable by a library caller that omits it) keeps the
    /// original block-until-terminal behaviour.
    pub timeout: Option<Duration>,
    pub fail_on_error: bool,
    pub progress: bool,
    /// `--poll-interval` override. When set, the fixed cadence replaces the
    /// default exponential backoff (callers shouldn't normally need it).
    pub poll_interval: Option<Duration>,
    pub spec: &'a OutputSpec,
    pub warnings: &'a [String],
}

/// One run's settle outcome (`data.runs[]`).
#[derive(Serialize)]
struct RunOutcome {
    run_id: String,
    status: &'static str,
    /// Report-based landing marker: the terminal `node.report` carries
    /// `via: "explicit-merge"`. Retained for backward compatibility — prefer
    /// [`Self::landed`], which is git-verified and robust to a caller-side
    /// rebase (issue `landing-signal-reliable-after-rebase`).
    merged: bool,
    /// Rebase-robust landing signal: true when the worker's committed work has
    /// landed in the target, confirmed by patch-id equivalence against the
    /// *current* target tip (`git cherry`) — NOT by branch-ref ancestry, which a
    /// caller-side `git rebase` invalidates. Falls back to the durable merge
    /// marker when git verification is unavailable. See [`landed_method`] for
    /// which. This is the flag callers should trust instead of running
    /// `git merge-base --is-ancestor <branch> <target>` by hand.
    ///
    /// [`landed_method`]: RunOutcome::landed_method
    landed: bool,
    /// How [`Self::landed`] was decided: `git-verified` | `report-marker` |
    /// `unverified`.
    landed_method: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    /// The `recoverable_work` block a supervisor stamps into an `agent-died`
    /// FAILED report when the dead agent's branch has unmerged commits ahead of
    /// source (issue `agent-death-strands-recoverable-work`). Surfaced verbatim
    /// so a caller can detect stranded, salvageable work without hand-rolling
    /// `git log <source>..<branch>`. Absent on any other outcome.
    #[serde(skip_serializing_if = "Option::is_none")]
    recoverable_work: Option<Value>,
}

#[derive(Serialize)]
struct WaitData {
    waited_ms: u64,
    condition: &'static str,
    runs: Vec<RunOutcome>,
}

/// Why the poll loop stopped.
enum Stop {
    /// The wait condition (`--all`/`--any`) was satisfied.
    Met,
    /// `--timeout` elapsed first.
    TimedOut,
}

pub fn run(args: Args<'_>) -> Result<(), CliError> {
    let condition = if args.any {
        Condition::Any
    } else {
        Condition::All
    };

    let root = crate::home::root_dir()?;

    // Validate + resolve every run up front so a malformed or unknown id is a
    // fast exit-1, never something we discover mid-poll. `run_paths_from_cli_arg` rejects a
    // malformed ULID (`invalid_run_id`); a well-formed id naming no run on disk
    // surfaces as `unknown_run` here.
    let mut runs: Vec<(String, RunPaths)> = Vec::with_capacity(args.run_ids.len());
    for run_id in &args.run_ids {
        let paths = run_paths_from_cli_arg(&root, run_id)?;
        if current_status(&paths)?.is_none() {
            return Err(
                CliError::user("unknown_run", format!("no run with id {run_id}"))
                    .with_invalid_value(run_id),
            );
        }
        runs.push((run_id.clone(), paths));
    }

    let start = Instant::now();
    let stop = wait_loop(
        &runs,
        condition,
        args.timeout,
        args.poll_interval,
        args.progress,
    )?;
    let waited_ms = start.elapsed().as_millis() as u64;

    // Build the final per-run summary, folding in each terminal node report.
    let mut outcomes = Vec::with_capacity(runs.len());
    for (run_id, paths) in &runs {
        outcomes.push(read_outcome(run_id, paths)?);
    }

    // Decide the exit code from the assembled outcomes:
    //   timeout                       → 2
    //   --fail-on-error & any settled
    //     run failed/cancelled        → 3
    //   otherwise                     → 0
    // A timeout takes precedence over fail-on-error: the condition was never
    // met, so there is nothing to grade.
    let exit_code: u8 = match stop {
        Stop::TimedOut => 2,
        Stop::Met if args.fail_on_error && any_settled_error(&outcomes) => 3,
        Stop::Met => 0,
    };

    let data = WaitData {
        waited_ms,
        condition: condition.wire(),
        runs: outcomes,
    };
    emit(&data, args.spec, args.warnings)?;

    if exit_code == 0 {
        return Ok(());
    }
    // Non-zero-but-not-an-error: the data envelope is already on stdout. Flush
    // this process's buffered logs (process::exit skips the LogGuard's Drop)
    // then exit with the graded code.
    crate::cli::flush_logs();
    std::process::exit(i32::from(exit_code));
}

/// Poll until the condition is met or `--timeout` elapses. Returns the reason
/// the loop stopped; the caller times the wait and assembles the summary.
///
/// Cadence: the sleep between idle polls starts at [`BACKOFF_START`] (100ms)
/// and doubles each round up to [`BACKOFF_CAP`] (2s), unless `--poll-interval`
/// pins a fixed cadence. Every sleep is clamped to the remaining timeout
/// budget so the loop wakes right at the deadline rather than overshooting it
/// (keeps the reported `waited_ms` ≈ the requested timeout).
fn wait_loop(
    runs: &[(String, RunPaths)],
    condition: Condition,
    timeout: Option<Duration>,
    poll_interval: Option<Duration>,
    progress: bool,
) -> Result<Stop, CliError> {
    let start = Instant::now();
    let mut backoff = poll_interval.unwrap_or(BACKOFF_START);
    let mut prev: Vec<Option<Status>> = vec![None; runs.len()];

    loop {
        let mut terminal = 0usize;
        for (i, (run_id, paths)) in runs.iter().enumerate() {
            let status = current_status(paths)?.ok_or_else(|| {
                // A run dir validated at entry but its manifest vanished
                // mid-poll: report it rather than spin forever.
                CliError::system(
                    "io_error",
                    format!("run {run_id} manifest disappeared while waiting"),
                )
            })?;
            if status.is_terminal() {
                terminal += 1;
            }
            if progress && prev[i] != Some(status) {
                emit_progress(run_id, status);
            }
            prev[i] = Some(status);
        }

        let met = match condition {
            Condition::All => terminal == runs.len(),
            Condition::Any => terminal >= 1,
        };
        if met {
            return Ok(Stop::Met);
        }
        if let Some(t) = timeout {
            if start.elapsed() >= t {
                return Ok(Stop::TimedOut);
            }
        }

        // Sleep, clamped to whatever timeout budget remains.
        let sleep_dur = match timeout {
            Some(t) => backoff.min(t.saturating_sub(start.elapsed())),
            None => backoff,
        };
        if sleep_dur.is_zero() {
            // Sub-millisecond budget left: yield briefly so we don't busy-spin
            // before the next iteration trips the timeout check above.
            std::thread::sleep(Duration::from_millis(1));
        } else {
            std::thread::sleep(sleep_dur);
        }
        if poll_interval.is_none() {
            backoff = (backoff * 2).min(BACKOFF_CAP);
        }
    }
}

/// Read a run's current `manifest.status` under the shared lock. `None` when
/// the run dir holds no manifest (unknown/uninitialized run). Holding
/// `LOCK_SH` keeps the single-file read consistent with a concurrent reducer.
fn current_status(paths: &RunPaths) -> Result<Option<Status>, CliError> {
    RunLock::with_shared_lock(&paths.lock(), || {
        Ok(read_manifest_opt(paths)?.map(|m| m.status))
    })
    .map_err(from_core)
}

/// Assemble one run's terminal outcome: its `manifest.status` plus the
/// outcome fields folded from the default node's terminal `node.report`
/// (`summary`, the `via: "explicit-merge"` merge marker, and — for a
/// failed/cancelled settle — a best-effort `error` reason). The manifest and
/// node projections are read in a single shared-lock window so the status and
/// the report it implies cannot disagree (state-integrity invariant 3).
fn read_outcome(run_id: &str, paths: &RunPaths) -> Result<RunOutcome, CliError> {
    let node_id = NodeId::parse_str(DEFAULT_NODE_ID).expect("DEFAULT_NODE_ID is a valid node id");
    // Read every field the outcome needs — status, the terminal report, and the
    // git-verification inputs (source repo/branch, worker branch/base_sha) — in
    // ONE shared-lock window so the status and the projections that explain it
    // cannot disagree (state-integrity invariant 3). The git shell-out that
    // computes `landed` runs AFTER the lock is released (below): holding the
    // flock across a subprocess would serialize every reader behind git.
    let git_inputs = RunLock::with_shared_lock(&paths.lock(), || {
        let manifest = read_manifest_opt(paths)?;
        let node = read_node_opt(paths, &node_id)?;
        Ok(GitInputs {
            status: manifest.as_ref().map(|m| m.status),
            source_repo: manifest.as_ref().and_then(|m| m.source_repo.clone()),
            source_branch: manifest.as_ref().and_then(|m| m.source_branch.clone()),
            worktree_path: node.as_ref().and_then(|n| n.worktree_path.clone()),
            branch: node.as_ref().and_then(|n| n.branch.clone()),
            base_sha: node.as_ref().and_then(|n| n.base_sha.clone()),
            report: node.and_then(|n| n.last_report),
        })
    })
    .map_err(from_core)?;

    let status = git_inputs.status.ok_or_else(|| {
        CliError::system(
            "io_error",
            format!("run {run_id} manifest disappeared while waiting"),
        )
    })?;
    let report = git_inputs.report.clone();

    // Git-verified `landed` (issue `landing-signal-reliable-after-rebase`): the
    // rebase-robust signal the caller should trust instead of hand-rolling
    // `git merge-base --is-ancestor`. Computed outside the shared lock.
    let signal = crate::run::landed::landing_signal(
        &crate::run::landed::LandingInputs {
            source_repo: git_inputs.source_repo.as_deref(),
            source_branch: git_inputs.source_branch.as_deref(),
            worktree_path: git_inputs.worktree_path.as_deref(),
            branch: git_inputs.branch.as_deref(),
            base_sha: git_inputs.base_sha.as_deref(),
            report: report.as_ref(),
        },
        &crate::supervise::cleanup::git_bin(),
    );

    let merged = report
        .as_ref()
        .and_then(|r| r.get("via"))
        .and_then(Value::as_str)
        == Some("explicit-merge");
    let summary = report
        .as_ref()
        .and_then(|r| r.get("summary"))
        .and_then(Value::as_str)
        .map(str::to_string);
    // `error` is best-effort for a non-`done` settle: the §7.3 report has no
    // dedicated error field, so surface the cancel `reason` when present.
    let error = if matches!(status, Status::Failed | Status::Cancelled) {
        report
            .as_ref()
            .and_then(|r| r.get("reason"))
            .and_then(Value::as_str)
            .map(str::to_string)
    } else {
        None
    };
    // Surface the supervisor's stranded-work signal verbatim (present only on an
    // `agent-died` failed report whose branch has unmerged commits ahead of
    // source). A caller reads `recoverable_work.recoverable` to decide whether to
    // salvage; the block is otherwise absent. Gated on a `failed` status: the
    // supervisor only ever stamps it on the failed-synthesis path, so a block on
    // a `done`/`cancelled` report is stale or spoofed and must not be surfaced (a
    // regular agent report can carry unknown fields — the validator permits them).
    let recoverable_work = if matches!(status, Status::Failed) {
        report
            .as_ref()
            .and_then(|r| r.get("recoverable_work"))
            .filter(|v| v.is_object())
            .cloned()
    } else {
        None
    };

    Ok(RunOutcome {
        run_id: run_id.to_string(),
        status: status_kebab(status),
        merged,
        landed: signal.landed,
        landed_method: signal.method.wire(),
        summary,
        error,
        recoverable_work,
    })
}

/// The run/node fields `read_outcome` reads under the shared lock before
/// computing `landed` outside it. Bundling them keeps the single consistent
/// snapshot (invariant 3) explicit and lets the git shell-out run lock-free.
struct GitInputs {
    status: Option<Status>,
    source_repo: Option<String>,
    source_branch: Option<String>,
    worktree_path: Option<String>,
    branch: Option<String>,
    base_sha: Option<String>,
    report: Option<Value>,
}

/// True iff any *settled* (terminal) run did not finish `done` — i.e. it is
/// `failed` or `cancelled`. Non-terminal runs (possible under `--any`) are
/// not graded: they never settled.
fn any_settled_error(outcomes: &[RunOutcome]) -> bool {
    outcomes
        .iter()
        .any(|o| matches!(o.status, "failed" | "cancelled"))
}

/// Emit one compact JSONL transition line to **stderr** for `--progress`, so a
/// live UI can follow state changes while the machine summary still lands on
/// stdout at the end. Best-effort: a serialization failure is swallowed rather
/// than aborting the wait.
fn emit_progress(run_id: &str, status: Status) {
    if let Ok(line) = serde_json::to_string(&serde_json::json!({
        "run_id": run_id,
        "status": status_kebab(status),
    })) {
        eprintln!("{line}");
    }
}

/// Render a one-line human summary of a `recoverable_work` block for text
/// output (`run wait`, `run show`). `None` when the block is absent or malformed
/// (its presence is optional and best-effort). Shared so both surfaces phrase
/// the stranded-work signal identically.
pub(crate) fn recoverable_summary(block: Option<&Value>) -> Option<String> {
    let obj = block?.as_object()?;
    let unmerged = obj.get("unmerged_commits").and_then(Value::as_u64)?;
    let recoverable = obj
        .get("recoverable")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let branch = obj.get("branch").and_then(Value::as_str).unwrap_or("?");
    // Agree the noun AND the verb in number so "1 unmerged commit merges" reads
    // correctly alongside "3 unmerged commits merge".
    let (noun, merge_verb, not_merge_verb) = if unmerged == 1 {
        ("commit", "merges", "does NOT merge")
    } else {
        ("commits", "merge", "do NOT merge")
    };
    if recoverable {
        Some(format!(
            "recoverable={unmerged} unmerged {noun} {merge_verb} cleanly on {branch}"
        ))
    } else {
        Some(format!(
            "recoverable=false ({unmerged} unmerged {noun} on {branch} {not_merge_verb} cleanly)"
        ))
    }
}

fn emit(data: &WaitData, spec: &OutputSpec, warnings: &[String]) -> Result<(), CliError> {
    match spec.format {
        OutputFormat::Json | OutputFormat::Jsonl => {
            output::emit_envelope(data, spec, warnings)?;
        }
        OutputFormat::Text => {
            println!("condition:  {}", data.condition);
            println!("waited_ms:  {}", data.waited_ms);
            for r in &data.runs {
                print!(
                    "{}  status={} landed={} ({}) merged={}",
                    r.run_id, r.status, r.landed, r.landed_method, r.merged
                );
                if let Some(s) = &r.summary {
                    print!("  summary={}", output::escape_one_line(s));
                }
                if let Some(e) = &r.error {
                    print!("  error={}", output::escape_one_line(e));
                }
                if let Some(line) = recoverable_summary(r.recoverable_work.as_ref()) {
                    print!("  {line}");
                }
                println!();
            }
            output::emit_text_warnings(warnings);
        }
    }
    Ok(())
}

/// clap value parser for `--timeout` / `--poll-interval`. Accepts humanly
/// written durations (`30s`, `5m`, `1h`, `2h 30m`); a malformed value is
/// rejected up front by clap as `invalid_arguments` (AGENTS-AI-FIRST-CLI §4),
/// never silently coerced.
pub fn parse_duration(s: &str) -> Result<Duration, String> {
    humantime::parse_duration(s).map_err(|e| format!("invalid duration '{s}': {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_duration_accepts_human_units() {
        assert_eq!(parse_duration("30s").unwrap(), Duration::from_secs(30));
        assert_eq!(parse_duration("5m").unwrap(), Duration::from_secs(300));
        assert_eq!(parse_duration("1h").unwrap(), Duration::from_secs(3600));
    }

    #[test]
    fn parse_duration_rejects_garbage() {
        assert!(parse_duration("soon").is_err());
        assert!(parse_duration("").is_err());
        assert!(parse_duration("-5s").is_err());
    }

    #[test]
    fn any_settled_error_only_counts_terminal_failures() {
        let mk = |status: &'static str| RunOutcome {
            run_id: "r".into(),
            status,
            merged: false,
            landed: false,
            landed_method: "unverified",
            summary: None,
            error: None,
            recoverable_work: None,
        };
        assert!(!any_settled_error(&[mk("done"), mk("done")]));
        assert!(any_settled_error(&[mk("done"), mk("failed")]));
        assert!(any_settled_error(&[mk("cancelled")]));
        // A still-running run under --any is not a settled error.
        assert!(!any_settled_error(&[mk("done"), mk("running")]));
    }

    #[test]
    fn recoverable_summary_phrasing() {
        // Absent / non-object → no line.
        assert_eq!(recoverable_summary(None), None);
        assert_eq!(recoverable_summary(Some(&serde_json::json!("x"))), None);

        // Clean, singular.
        let clean = serde_json::json!({
            "recoverable": true,
            "unmerged_commits": 1,
            "merges_cleanly": true,
            "branch": "wt/foo",
        });
        assert_eq!(
            recoverable_summary(Some(&clean)).as_deref(),
            Some("recoverable=1 unmerged commit merges cleanly on wt/foo"),
        );

        // Clean, plural.
        let many = serde_json::json!({
            "recoverable": true, "unmerged_commits": 3, "merges_cleanly": true, "branch": "wt/bar",
        });
        assert_eq!(
            recoverable_summary(Some(&many)).as_deref(),
            Some("recoverable=3 unmerged commits merge cleanly on wt/bar"),
        );

        // Unmerged but conflicting → flagged, not recoverable.
        let dirty = serde_json::json!({
            "recoverable": false, "unmerged_commits": 2, "merges_cleanly": false, "branch": "wt/baz",
        });
        assert_eq!(
            recoverable_summary(Some(&dirty)).as_deref(),
            Some("recoverable=false (2 unmerged commits on wt/baz do NOT merge cleanly)"),
        );

        // Singular conflicting → the negative verb agrees in number too.
        let dirty1 = serde_json::json!({
            "recoverable": false, "unmerged_commits": 1, "merges_cleanly": false, "branch": "wt/q",
        });
        assert_eq!(
            recoverable_summary(Some(&dirty1)).as_deref(),
            Some("recoverable=false (1 unmerged commit on wt/q does NOT merge cleanly)"),
        );
    }
}
