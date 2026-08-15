//! Read-time detection of an *attention-required* run (design.md §2.5 / A5,
//! issue `attention-required-run-surface`).
//!
//! An attention-required run is the design's deliberate "finished but skipped
//! `run merge`" case: the worker process exited **cleanly** (the launcher shim
//! recorded a `worker.exited` with `code == 0` and no signal) yet the node is
//! still **non-terminal** — no `explicit-merge` transition ever landed, so the
//! typed outcome table (§2.6) leaves it as
//! [`LiveVerdict::AttentionRequired`](crate::supervise::outcome::LiveVerdict::AttentionRequired) and
//! the supervisor does nothing further. Without this surface such a run sits
//! `pending`/`running` forever: `run wait` would block the whole timeout, and the
//! PO-review cadence that is supposed to discover the jam never sees it.
//!
//! Crucially this is **not a terminal state** and this module **never mutates**
//! anything — like its sibling [`crate::run::stalled`], it is a *computed*
//! read-time hint over a durable told fact (`Node::worker_exit`) plus the node's
//! current status. Terminalizing the run here would defeat the whole point: the
//! manual-finish path (§2.2 — drive `run merge` from the worktree, or
//! `run cancel`) needs the run left exactly where it is, work preserved.
//!
//! ## Why this is a durable fact, not a race
//!
//! The shim records `worker.exited` only **after** `wait()` on the agent process
//! returns — i.e. after the agent has exited. An exited agent cannot go on to run
//! `run merge`, so a clean-exit-and-no-merge observation is stable: it never
//! flips back to a merge on a later poll. That is why the detector needs no grace
//! window (unlike the orphaned-supervisor shape, whose "is it just restarting?"
//! ambiguity forces one).
//!
//! ## Precedence over the stall shapes
//!
//! Attention-required is checked **before** the [`crate::run::stalled`] verdicts.
//! A worker that exited cleanly and whose supervisor *also* later died would
//! otherwise match `orphaned` (dead supervisor, ≥1 node, idle) — but the correct
//! remediation is the manual finish (`run merge` from the worktree), NOT
//! `run reattach` (reviving the supervisor just re-observes the same clean exit
//! and does nothing). The told clean-exit fact is the more specific truth, so it
//! wins.

use chrono::{DateTime, Utc};

use octl_core::{Status, WorkerExit};

/// Is this node in the attention-required state — a clean worker exit with the
/// node still non-terminal (so `run merge` was skipped)?
///
/// Returns `true` only when both hold:
///
/// - the node's status is **non-terminal** (`Pending`/`Running`/`Blocked`) — a
///   `Done`/`Failed`/`Cancelled` node already settled and is not awaiting a human;
/// - the launcher shim recorded a **clean** `worker.exited`
///   ([`WorkerExit::is_clean`] — `code == 0`, no signal). A failing exit is the
///   supervisor's `failed` path, not attention; an absent exit means the worker
///   is still running (or was never launched through the shim) — neither is
///   attention-required.
///
/// A merge, had it happened, would have driven the node terminal, so a
/// non-terminal node with a clean exit is exactly "finished but did not
/// `run merge`". Pure over its inputs; touches no event/reducer/schema path.
#[must_use]
pub fn is_attention_required(node_status: Status, worker_exit: Option<WorkerExit>) -> bool {
    !node_status.is_terminal() && worker_exit.is_some_and(WorkerExit::is_clean)
}

/// The human/JSON resume hint for an attention-required run: the two ways a PO
/// can settle it — drive the skipped merge from the worktree, or lay it to rest.
/// Kept as one shared helper so `run wait` / `run show` / `run list` phrase it
/// identically.
#[must_use]
pub fn resume_hint(run_id: &str, worktree_path: Option<&str>) -> String {
    match worktree_path {
        Some(path) => format!(
            "worker finished but skipped `run merge`; finish it with \
             `cd {path} && orchestratectl run merge {run_id}`, or `run cancel {run_id}` to \
             lay it to rest"
        ),
        None => format!(
            "worker finished but skipped `run merge`; finish it with \
             `orchestratectl run merge {run_id}` from its worktree, or `run cancel {run_id}` to \
             lay it to rest"
        ),
    }
}

/// The resume-context fields `run show` / `run list` surface for an
/// attention-required run so a PO can find and finish the stuck worktree without
/// re-deriving them from the node/manifest projections (design.md §2.5).
///
/// Built only when [`is_attention_required`] holds; a `None` on the summary means
/// the run is not attention-required.
#[derive(serde::Serialize)]
pub struct AttentionView {
    /// Why the run needs attention. A stable machine string a JSON consumer can
    /// branch on without parsing [`Self::resume_hint`].
    pub reason: &'static str,
    /// Seconds the node has been sitting non-terminal since it started
    /// (`now - started_at`), or since the run was created when the node never
    /// recorded a `started_at`. Clamped at 0 so clock skew never prints negative.
    pub pending_age_secs: i64,
    /// The last-observed worker PID (`node.agent_pid`) — the process the shim
    /// wrapped, now exited. Surfaced as evidence of which worker finished; `null`
    /// when the node never recorded a pid.
    pub worker_pid: Option<i32>,
    /// The node's git worktree, where the manual finish (`run merge`) is driven.
    /// `null` when unrecorded.
    pub worktree_path: Option<String>,
    /// The run's source/target branch the skipped merge would land on. `null` for
    /// a legacy run that never recorded one.
    pub source_branch: Option<String>,
    /// One-line resume hint (see [`resume_hint`]) — the two ways to settle it.
    pub resume_hint: String,
}

/// The stable `reason` string on [`AttentionView`].
pub const ATTENTION_REASON: &str = "worker exited cleanly without running `run merge`";

impl AttentionView {
    /// Assemble the resume-context view for an attention-required run. `now` is
    /// injected so callers share one per-scan clock and tests are deterministic.
    #[must_use]
    pub fn build(
        run_id: &str,
        now: DateTime<Utc>,
        started_at: Option<DateTime<Utc>>,
        created_at: DateTime<Utc>,
        worker_pid: Option<i32>,
        worktree_path: Option<String>,
        source_branch: Option<String>,
    ) -> Self {
        let anchor = started_at.unwrap_or(created_at);
        let pending_age_secs = now.signed_duration_since(anchor).num_seconds().max(0);
        let resume_hint = resume_hint(run_id, worktree_path.as_deref());
        Self {
            reason: ATTENTION_REASON,
            pending_age_secs,
            worker_pid,
            worktree_path,
            source_branch,
            resume_hint,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exit(code: Option<i32>, signal: Option<i32>) -> WorkerExit {
        WorkerExit {
            code,
            signal,
            at: Utc::now(),
        }
    }

    /// The exact attention signature: a non-terminal node with a clean (0) exit.
    #[test]
    fn clean_exit_non_terminal_is_attention() {
        for status in [Status::Pending, Status::Running, Status::Blocked] {
            assert!(
                is_attention_required(status, Some(exit(Some(0), None))),
                "status {status:?} with a clean exit must be attention-required"
            );
        }
    }

    /// A terminal node already settled — never attention, even with a clean exit
    /// recorded (a merged node exits 0 too, but it is Done, not awaiting a human).
    #[test]
    fn terminal_node_is_not_attention() {
        for status in [Status::Done, Status::Failed, Status::Cancelled] {
            assert!(
                !is_attention_required(status, Some(exit(Some(0), None))),
                "terminal status {status:?} must not be attention-required"
            );
        }
    }

    /// A failing exit is the supervisor's `failed` path, not attention.
    #[test]
    fn failing_exit_is_not_attention() {
        assert!(!is_attention_required(
            Status::Running,
            Some(exit(Some(2), None))
        ));
        assert!(!is_attention_required(
            Status::Running,
            Some(exit(None, Some(9)))
        ));
    }

    /// No recorded exit means the worker is still running (or was never launched
    /// through the shim) — not attention-required.
    #[test]
    fn no_exit_is_not_attention() {
        assert!(!is_attention_required(Status::Running, None));
        assert!(!is_attention_required(Status::Pending, None));
    }

    /// `pending_age_secs` is `now - started_at`, clamped at 0 under clock skew,
    /// and falls back to `created_at` when the node never started.
    #[test]
    fn pending_age_anchors_and_clamps() {
        let now: DateTime<Utc> = "2026-08-15T12:00:00Z".parse().unwrap();
        let started: DateTime<Utc> = "2026-08-15T11:30:00Z".parse().unwrap();
        let created: DateTime<Utc> = "2026-08-15T11:00:00Z".parse().unwrap();

        // Anchored to started_at when present.
        let v = AttentionView::build("r", now, Some(started), created, None, None, None);
        assert_eq!(v.pending_age_secs, 30 * 60);

        // Falls back to created_at when no started_at.
        let v = AttentionView::build("r", now, None, created, None, None, None);
        assert_eq!(v.pending_age_secs, 60 * 60);

        // Clock skew (future anchor) clamps to 0, never negative.
        let future: DateTime<Utc> = "2026-08-15T13:00:00Z".parse().unwrap();
        let v = AttentionView::build("r", now, Some(future), created, None, None, None);
        assert_eq!(v.pending_age_secs, 0);
    }

    /// The resume hint names the worktree when known, and both remediations.
    #[test]
    fn resume_hint_mentions_worktree_and_both_actions() {
        let with_wt = resume_hint("01run", Some("/tmp/wt/foo"));
        assert!(with_wt.contains("/tmp/wt/foo"));
        assert!(with_wt.contains("run merge 01run"));
        assert!(with_wt.contains("run cancel 01run"));

        let without = resume_hint("01run", None);
        assert!(without.contains("run merge 01run"));
        assert!(without.contains("run cancel 01run"));
    }
}
