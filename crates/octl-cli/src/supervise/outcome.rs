//! The typed supervisor outcome table (design.md §2.6 / A6, issue
//! `typed-supervisor-outcomes`).
//!
//! `run merge` is the only **success** truth, but not the only **terminal**
//! truth. Before this module, the supervisor's terminal verdict was spread
//! across a cascade of `if` branches in [`super::watchdog_tick`] and a second,
//! parallel field-sniffing gate in [`super::cleanup`] — each re-deriving the
//! outcome from a cross-product of proxies (pid × pane × branch × report ×
//! activity clocks). That cross-product is exactly the inference the thin model
//! deletes.
//!
//! This module replaces it with two small, pure, exhaustively-tested tables:
//!
//! - [`TerminalOutcome`] classifies a node's **terminal `node.report`** into one
//!   typed outcome, and [`TerminalOutcome::teardown`] maps it to the single
//!   [`Teardown`] policy it authorizes. This is the invariant-critical table:
//!   it is what prevents a future change from silently re-introducing heuristic
//!   teardown of unmerged work (invariant 5). `cleanup` reads it instead of
//!   re-sniffing `last_report` JSON.
//! - [`LiveVerdict`] classifies a **non-terminal** node from its *told facts*
//!   (the A1 `worker.exited` status) plus a residual [`DeathObservation`] — the
//!   pid crash backstop, the ONLY place pid liveness still governs an outcome.
//!   The watchdog reads it to decide what, if anything, to synthesize this tick.
//!
//! Both tables are pure functions of durable inputs, so the reducer's
//! append/replay/fsync correctness and the lock invariants are untouched — the
//! caller still threads the [`octl_core::RunLock`] witness through every append.

use octl_core::{Node, WorkerExit, VIA_EXPLICIT_MERGE};
use serde_json::Value;

/// The teardown a [`TerminalOutcome`] authorizes — the "Teardown?" column of the
/// design §2.6 table, made explicit and total.
///
/// This enum is the whole point of the typed table: teardown is a function of a
/// *typed outcome*, never of a signal combination. Adding a new terminal outcome
/// forces a deliberate teardown choice here (the [`TerminalOutcome::teardown`]
/// match is exhaustive), so a future outcome can never default into destroying a
/// worker's branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Teardown {
    /// A confirmed explicit `run merge`: the work is in the run's source branch,
    /// so the worktree is disposable and the branch may be **force-deleted**
    /// (`git branch -D`). The only outcome that earns the force teardown.
    Full,
    /// Preserve the worker's branch **and** worktree — only the tmux window winds
    /// down. Every non-merge negative terminal: a blocked human handoff, a told
    /// worker-exit failure, the confirmed-death crash backstop. The committed
    /// work is left exactly where the human/salvage skill can pick it up
    /// (invariant 5, issue `blocked-report-deletes-branch`).
    PreserveWork,
    /// Source-relative teardown: wind the window down, and remove the worktree +
    /// delete the branch **only if** the branch carries no commits unreachable
    /// from the run's source branch (`git branch -d` refuses an unmerged branch
    /// as the last-resort backstop). Used for `run cancel` — which explicitly
    /// preserves work-bearing branches, never a teardown authorization
    /// (design.md §2.6) — and for the (unusual) plain success that skipped
    /// `run merge`.
    SourceRelative,
}

/// A node's typed **terminal** outcome, classified purely from its terminal
/// `node.report` (design.md §2.6). `None` from [`TerminalOutcome::classify`]
/// means the node has no terminal report yet — it is still live, and the
/// [`LiveVerdict`] table governs instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalOutcome {
    /// `run merge` succeeded: `success: true` with `via: "explicit-merge"`. The
    /// one success truth. → [`Teardown::Full`].
    Merged,
    /// A blocked handoff: the agent hit a wall and handed committed-but-unmerged
    /// work to a human via a plain `node report` (`success: false`, not
    /// cancelled, not an explicit merge). → [`Teardown::PreserveWork`].
    Blocked,
    /// A failure the supervisor recorded: a told `worker.exited` non-zero/signal
    /// (A1) or the confirmed-death crash backstop (`success: false`, a known
    /// supervisor/worker failure reason). → [`Teardown::PreserveWork`].
    Failed,
    /// `run cancel` (run or node): `cancelled: true`. Cancel explicitly PRESERVES
    /// work — it is never a teardown authorization (design.md §2.6). →
    /// [`Teardown::SourceRelative`].
    Cancelled,
    /// A `success: true` terminal report that did NOT come through `run merge`
    /// (no `via: "explicit-merge"`). Unusual under the thin model — success is
    /// only ever told via `run merge` — but a hand-authored/legacy success
    /// report is classified here rather than mistaken for a merge, so it never
    /// earns the force teardown. → [`Teardown::SourceRelative`].
    PlainSuccess,
}

impl TerminalOutcome {
    /// Classify a node's terminal `node.report` into a typed outcome, or `None`
    /// if the node has no terminal report yet.
    ///
    /// The order is the table's precedence: an explicit merge wins over every
    /// other shape; then cancel; then a negative report splits into blocked vs
    /// failed by reason; then a residual plain success. A report is only ever one
    /// of these — the reducer enforces the success-XOR-cancelled invariant on
    /// append — so the branches are mutually exclusive in practice.
    #[must_use]
    pub fn classify(n: &Node) -> Option<Self> {
        let report = n.last_report.as_ref()?;
        let success = report.get("success").and_then(Value::as_bool);
        let cancelled = report.get("cancelled").and_then(Value::as_bool) == Some(true);
        let is_explicit_merge = report_via(report) == Some(VIA_EXPLICIT_MERGE);

        // Explicit merge: success:true AND via:explicit-merge. Checked first — a
        // merge marker with success:false (malformed/spoofed) is deliberately NOT
        // a merge and falls through to the negative arms below, so it never earns
        // the force teardown on its marker alone.
        if success == Some(true) && is_explicit_merge {
            return Some(TerminalOutcome::Merged);
        }
        // Cancel: cancelled:true. A `run cancel` report carries no via and
        // success:false; the reducer rejects the contradiction success:true +
        // cancelled:true, so this is unambiguous.
        if cancelled {
            return Some(TerminalOutcome::Cancelled);
        }
        match success {
            // A negative report that is neither cancel nor merge: a blocked
            // handoff or a supervisor/worker-recorded failure. They share a
            // teardown policy (PreserveWork); the split is observability only.
            Some(false) => Some(if is_supervisor_failure(report) {
                TerminalOutcome::Failed
            } else {
                TerminalOutcome::Blocked
            }),
            // A success report that did not come through `run merge`.
            Some(true) => Some(TerminalOutcome::PlainSuccess),
            // No boolean `success` on a present report: the reducer would have
            // rejected such an event as corrupt before it ever set `last_report`,
            // so this is unreachable in practice. Treat it as the safest bucket —
            // preserve the work rather than guess a teardown.
            None => Some(TerminalOutcome::Blocked),
        }
    }

    /// The teardown policy this outcome authorizes. Exhaustive so a new outcome
    /// forces a deliberate teardown decision here.
    #[must_use]
    pub fn teardown(self) -> Teardown {
        match self {
            TerminalOutcome::Merged => Teardown::Full,
            TerminalOutcome::Blocked | TerminalOutcome::Failed => Teardown::PreserveWork,
            TerminalOutcome::Cancelled | TerminalOutcome::PlainSuccess => Teardown::SourceRelative,
        }
    }

    /// True when this outcome is a confirmed, successful explicit `run merge` —
    /// the only outcome whose branch may be force-deleted. The single caller in
    /// `cleanup` uses this to pick `git branch -D` over the safe `-d`.
    #[must_use]
    pub fn is_explicit_merge(self) -> bool {
        matches!(self, TerminalOutcome::Merged)
    }
}

/// The residual pid-liveness observation, the ONLY place pid liveness still
/// governs an outcome (design.md §2.1a — "pid liveness as a pure crash backstop
/// only, never a primary signal").
///
/// The watchdog computes this from the liveness probe plus the node's durable
/// first-death observation ([`Node::first_death_at`]) and a fixed post-death
/// grace: the grace exists only to let an in-flight `worker.exited` / merge
/// append land before the backstop fires (design.md §2.1a), and it is anchored
/// to a persisted, monotonic first-death timestamp so it survives a supervisor
/// restart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeathObservation {
    /// The worker process is (still) alive per the liveness probe.
    Alive,
    /// Confirmed dead, but the post-death grace has not yet elapsed since the
    /// first-death observation — defer, so a racing exit/merge append can win.
    DeadWithinGrace,
    /// Confirmed dead and the post-death grace has elapsed with no exit event and
    /// no merge — the shim was lost (hard kill / host death); fire the backstop.
    DeadGraceElapsed,
}

/// What the watchdog should do about a **non-terminal** node this tick, from its
/// told facts plus the residual death observation (design.md §2.1 / §2.6).
///
/// The caller must have already established that the node is non-terminal and has
/// no `explicit-merge` report — a merged node is terminal and classified by
/// [`TerminalOutcome`], not here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveVerdict {
    /// The launcher shim recorded a FAILING `worker.exited` (non-zero / signal).
    /// Synthesize a `failed` `node.report` from the told fact — never a pid guess
    /// (branch preserved via [`Teardown::PreserveWork`]).
    WorkerFailed(WorkerExit),
    /// The shim recorded a CLEAN exit (`0`) but there is no merge — the worker
    /// finished but skipped `run merge`. Stay **non-terminal** (attention-
    /// required); hand to the manual finish skill. NEVER auto-failed
    /// (design.md §2.1) — this is the case the deleted idle-unmerged net used to
    /// wrongly terminalize.
    AttentionRequired,
    /// No told exit, the process is confirmed dead, and the post-death grace has
    /// elapsed — the residual crash backstop. Synthesize `failed`, preserve the
    /// branch/worktree.
    CrashBackstopFailed,
    /// No told exit, confirmed dead, but still inside the post-death grace —
    /// defer this tick so an in-flight exit/merge append can win.
    DeferGrace,
    /// Nothing to do this tick: alive with no told exit, or awaiting its own
    /// terminal signal.
    Alive,
}

/// Classify a non-terminal node into a [`LiveVerdict`] (design.md §2.1 / §2.6).
///
/// Told facts beat guesses: a recorded `worker.exited` — clean or failing — is
/// authoritative and short-circuits the pid backstop entirely. Only when the
/// shim recorded nothing does the residual [`DeathObservation`] govern.
#[must_use]
pub fn classify_live_node(worker_exit: Option<WorkerExit>, death: DeathObservation) -> LiveVerdict {
    if let Some(exit) = worker_exit {
        return if exit.is_failure() {
            LiveVerdict::WorkerFailed(exit)
        } else {
            LiveVerdict::AttentionRequired
        };
    }
    match death {
        DeathObservation::Alive => LiveVerdict::Alive,
        DeathObservation::DeadWithinGrace => LiveVerdict::DeferGrace,
        DeathObservation::DeadGraceElapsed => LiveVerdict::CrashBackstopFailed,
    }
}

/// The set of `reason` strings the supervisor/launcher stamps on a `failed`
/// `node.report` it synthesizes — as opposed to a blocked handoff an agent
/// authored. Used only to split [`TerminalOutcome::Failed`] from
/// [`TerminalOutcome::Blocked`] for observability; both share a teardown policy,
/// so a mis-split can never change teardown behavior.
fn is_supervisor_failure(report: &Value) -> bool {
    let Some(reason) = report.get("reason").and_then(Value::as_str) else {
        // A negative report with no reason is more likely an agent handoff than a
        // supervisor failure (the supervisor always stamps a reason); classify as
        // blocked. Teardown is identical either way.
        return false;
    };
    reason == super::WORKER_EXITED_NONZERO_REASON
        || reason == super::WORKER_KILLED_BY_SIGNAL_REASON
        || reason == super::NO_WORKER_REASON
        // The confirmed-death crash backstop and the liveness probe stamp reasons
        // that all describe a dead/gone agent (`agent-died`, `agent-*`). Matched
        // as a prefix so every liveness-reason variant counts without enumerating
        // them here.
        || reason.starts_with(AGENT_DIED_REASON_PREFIX)
}

/// Reason prefix the confirmed-death crash backstop / liveness probe stamps.
const AGENT_DIED_REASON_PREFIX: &str = "agent-";

/// The `via` field of a terminal report, if any.
fn report_via(report: &Value) -> Option<&str> {
    report.get("via").and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use octl_core::{Kind, NodeId, RunId, Status};
    use serde_json::json;

    /// Build a minimal `Node` carrying `report` as its terminal `last_report`.
    fn node_with_report(report: Option<Value>) -> Node {
        Node {
            schema_version: 1,
            node_id: NodeId::parse_str("n-0001").unwrap(),
            run_id: RunId::parse_str("01jxsnap000000000000000000").unwrap(),
            parent_node_id: None,
            kind: Kind::Spinoff,
            status: Status::Failed,
            task: None,
            worktree_path: None,
            branch: None,
            base_sha: None,
            tmux_window: None,
            tmux_identity: None,
            agent_pid: None,
            agent_pid_start_time: None,
            supervisor_pid: None,
            children: vec![],
            started_at: None,
            updated_at: Utc::now(),
            last_report: report,
            last_processed_report_seq_by_child: serde_json::Map::new(),
            retry_attempts: 0,
            worker_exit: None,
            pending_merge: None,
            first_death_at: None,
        }
    }

    /// The teardown table (design §2.6 "Teardown?" column), asserted row by row
    /// so a future change cannot silently re-introduce heuristic teardown of
    /// unmerged work.
    #[test]
    fn terminal_outcome_teardown_table() {
        let rows: &[(&str, Value, TerminalOutcome, Teardown)] = &[
            (
                "explicit merge succeeds -> done + full teardown",
                json!({ "success": true, "via": "explicit-merge" }),
                TerminalOutcome::Merged,
                Teardown::Full,
            ),
            (
                "blocked handoff -> preserve branch + worktree",
                json!({ "success": false, "reason": "blocked-on-human" }),
                TerminalOutcome::Blocked,
                Teardown::PreserveWork,
            ),
            (
                "told worker-exit failure -> failed, preserve",
                json!({ "success": false, "reason": "worker-exited-nonzero" }),
                TerminalOutcome::Failed,
                Teardown::PreserveWork,
            ),
            (
                "confirmed-death backstop -> failed, preserve",
                json!({ "success": false, "reason": "agent-died" }),
                TerminalOutcome::Failed,
                Teardown::PreserveWork,
            ),
            (
                "run cancel -> cancelled, source-relative preserve",
                json!({ "success": false, "cancelled": true, "reason": "cancelled by user" }),
                TerminalOutcome::Cancelled,
                Teardown::SourceRelative,
            ),
            (
                "plain success without run merge -> source-relative",
                json!({ "success": true }),
                TerminalOutcome::PlainSuccess,
                Teardown::SourceRelative,
            ),
        ];
        for (why, report, want_outcome, want_teardown) in rows {
            let n = node_with_report(Some(report.clone()));
            let got = TerminalOutcome::classify(&n).expect("terminal report classifies");
            assert_eq!(got, *want_outcome, "outcome: {why}");
            assert_eq!(got.teardown(), *want_teardown, "teardown: {why}");
        }
    }

    /// Only an explicit merge is ever force-deletable, and a merge marker with
    /// `success: false` is NOT a merge (never earns the force teardown).
    #[test]
    fn only_explicit_merge_force_deletes() {
        let merged = node_with_report(Some(json!({ "success": true, "via": "explicit-merge" })));
        assert!(TerminalOutcome::classify(&merged)
            .unwrap()
            .is_explicit_merge());

        let spoofed = node_with_report(Some(json!({ "success": false, "via": "explicit-merge" })));
        let outcome = TerminalOutcome::classify(&spoofed).unwrap();
        assert!(!outcome.is_explicit_merge(), "success:false is not a merge");
        assert_eq!(outcome.teardown(), Teardown::PreserveWork);
    }

    /// A node with no terminal report is not classified — it is still live.
    #[test]
    fn no_report_is_not_terminal() {
        assert_eq!(TerminalOutcome::classify(&node_with_report(None)), None);
    }

    /// Cancel wins over a merge marker: a `cancelled: true` report is never
    /// force-torn-down even if a spoofed `via` rides along.
    #[test]
    fn cancel_beats_spoofed_merge_marker() {
        let n = node_with_report(Some(
            json!({ "success": false, "cancelled": true, "via": "explicit-merge" }),
        ));
        let outcome = TerminalOutcome::classify(&n).unwrap();
        assert_eq!(outcome, TerminalOutcome::Cancelled);
        assert_eq!(outcome.teardown(), Teardown::SourceRelative);
    }

    /// The live-node table (design §2.6, non-terminal rows): told facts beat the
    /// pid backstop, a clean exit is attention-required (never failed), and the
    /// backstop only fires after the post-death grace.
    #[test]
    fn live_verdict_table() {
        let clean = WorkerExit {
            code: Some(0),
            signal: None,
            at: Utc::now(),
        };
        let nonzero = WorkerExit {
            code: Some(2),
            signal: None,
            at: Utc::now(),
        };
        let killed = WorkerExit {
            code: None,
            signal: Some(9),
            at: Utc::now(),
        };

        // Told failure -> WorkerFailed regardless of death observation.
        assert_eq!(
            classify_live_node(Some(nonzero), DeathObservation::Alive),
            LiveVerdict::WorkerFailed(nonzero)
        );
        assert_eq!(
            classify_live_node(Some(killed), DeathObservation::DeadGraceElapsed),
            LiveVerdict::WorkerFailed(killed)
        );
        // Clean exit -> AttentionRequired, NEVER failed, even if the pid is gone.
        assert_eq!(
            classify_live_node(Some(clean), DeathObservation::DeadGraceElapsed),
            LiveVerdict::AttentionRequired
        );
        // No told exit: the residual pid backstop governs.
        assert_eq!(
            classify_live_node(None, DeathObservation::Alive),
            LiveVerdict::Alive
        );
        assert_eq!(
            classify_live_node(None, DeathObservation::DeadWithinGrace),
            LiveVerdict::DeferGrace
        );
        assert_eq!(
            classify_live_node(None, DeathObservation::DeadGraceElapsed),
            LiveVerdict::CrashBackstopFailed
        );
    }
}
