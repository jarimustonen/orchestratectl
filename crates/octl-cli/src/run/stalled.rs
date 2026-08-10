//! Read-time stall detection for an undriven `--kind orchestrate` driver run.
//!
//! A `--kind orchestrate` supervisor only *adopts* children — it never drives
//! the fan-out itself; the orchestrator agent runs in the user's main
//! conversation and is what spawns children (issue
//! `peculiarly-muddled-caption`). If that agent never runs its drive loop (or
//! dies immediately after `run create`), the driver node `n-0001` sits
//! `pending` with zero children and no fresh events forever, supervisor alive —
//! indistinguishable at a glance from a healthy long-running campaign (a real
//! reproduction sat this way for 15 hours).
//!
//! `stalled` is a **computed** hint, not a persisted status: it touches no
//! event-append / reducer / schema path (state-integrity invariants 1–3 are not
//! in play). It is derived purely from the driver node's existing timestamp +
//! status + children counter, read under the same shared lock the caller
//! already holds for the manifest. Terminal-status semantics are untouched — a
//! stalled run is still `pending`; the flag only says "pending, but visibly not
//! progressing".
//!
//! Scope: this catches the *specific* zombie in the issue — a driver that was
//! **never driven** (still `pending`, zero children). It deliberately does NOT
//! try to detect every stalled shape (a driver that spawned one child then
//! died, or transitioned to `running` and then stopped emitting events): those
//! need a real orchestrator liveness/heartbeat signal, tracked as the follow-up
//! `peculiarly-cheerful-mine`. The hint is a heuristic, not a liveness proof —
//! hence the human output says "verify" before prescribing a cancel.

use chrono::{DateTime, Duration, Utc};

use octl_core::{Kind, Node, Status};

/// Grace window an undriven orchestrate driver node may sit `pending` with zero
/// children before `stalled` trips. Chosen to comfortably exceed the time a
/// genuinely-driven orchestrator takes to spawn its first child (planning +
/// `run create` of the first ready feature) while still catching a zombie
/// within a useful window. The 15h real reproduction dwarfs any reasonable
/// value here.
pub const STALL_GRACE: Duration = Duration::minutes(12);

/// The driver node's `n-0001` — the single fan-out driver of an `orchestrate`
/// run. Mirrors `run show`'s `DEFAULT_NODE_ID`.
pub const DRIVER_NODE_ID: &str = "n-0001";

/// Compute the `stalled` hint for a run from its manifest status, kind, and
/// driver node.
///
/// Returns `true` only for a `pending` `--kind orchestrate` run whose driver
/// node is itself still `pending`, has spawned **zero** children, and has not
/// been touched for longer than [`STALL_GRACE`] — the exact signature of a
/// driver that was created but never driven. Any of these disqualifies it:
///
/// - a non-`pending` run (a `done`/`failed`/`cancelled`/`running` manifest is
///   not a zombie — a terminal run whose `n-0001` projection stayed `pending`
///   must never be flagged, or `run list` would print `done (stalled)` and the
///   remediation would tell the user to cancel an already-terminal run);
/// - a non-`orchestrate` kind (only the orchestrate driver has the
///   "supervisor adopts but does not drive" shape);
/// - a driver node that reached a non-`pending` status (it is running,
///   terminal, or otherwise progressing);
/// - a driver node with ≥1 child (the orchestrator agent *is* driving);
/// - a driver node touched within the grace window. The node projection's
///   `updated_at` is bumped by exactly the events that mark driver progress —
///   `node.status` / `node.report` / `node.retry` / `child.spawned` (verified
///   against the reducer). Discussion / spinoff / supervisor events bump the
///   *manifest* timestamp, not the node's, so `node.updated_at` is a precise
///   "the driver made progress" proxy — deliberately narrower than
///   `manifest.updated_at`, which unrelated supervisor churn would keep falsely
///   fresh. The narrow cost: a driver that only opens a discussion (e.g. asks
///   the user a question) without spawning a child is still counted idle, which
///   is why the hint is advisory, not authoritative;
/// - a missing driver node (a half-initialized run — not assessable, so not
///   flagged rather than falsely alarmed).
///
/// `now` is injected so the decision is deterministic in tests.
#[must_use]
pub fn is_stalled(
    run_status: Status,
    kind: Kind,
    driver: Option<&Node>,
    now: DateTime<Utc>,
) -> bool {
    if run_status != Status::Pending {
        return false;
    }
    if kind != Kind::Orchestrate {
        return false;
    }
    let Some(node) = driver else {
        return false;
    };
    if node.status != Status::Pending {
        return false;
    }
    if !node.children.is_empty() {
        return false;
    }
    now.signed_duration_since(node.updated_at) > STALL_GRACE
}

/// Detect a *stillborn* run: created successfully, but its supervisor died
/// before ever spawning the first worker node — so the run can never make
/// progress and will otherwise sit `pending` until a caller's timeout expires
/// (issue `run-wait-stillborn-run-not-detected`; a real incident blocked
/// `run wait` for ~6h).
///
/// Returns `true` only for the exact "never started" signature:
///
/// - `status == Pending` — the run never advanced past creation. A terminal or
///   `running` manifest is not stillborn (it started).
/// - the supervisor is **not alive** — the actor that would create `n-0001` and
///   roll the run up is dead (or was never recorded). This is the crucial
///   difference from [`is_stalled`]: there the supervisor is *alive* but idle,
///   so a grace window is needed to tell "slow" from "dead"; here the
///   supervisor is confirmed dead, which is unambiguous and needs no grace.
/// - `node_count == 0` — not a single worker node was ever created. This also
///   makes the check kind-agnostic: a `--kind orchestrate` run whose driver
///   node was never even created is stillborn by the same logic, while a run
///   that got as far as `n-0001` is excluded (it started).
/// - `updated_at == created_at` — no event has been applied since creation
///   (`node.created` / `supervisor.started` / any projection write bumps
///   `manifest.updated_at`), so there has been zero forward progress.
///
/// Like [`is_stalled`], this is a **computed** read-time hint over the manifest
/// (plus a single-file supervisor-pid probe) — it touches no
/// event-append / reducer / schema path.
#[must_use]
pub fn is_stillborn(
    run_status: Status,
    supervisor_alive: bool,
    node_count: u32,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
) -> bool {
    run_status == Status::Pending
        && !supervisor_alive
        && node_count == 0
        && updated_at == created_at
}

#[cfg(test)]
mod tests {
    use super::*;
    use octl_core::{NodeId, RunId};

    fn node(status: Status, children: usize, updated_at: DateTime<Utc>) -> Node {
        Node {
            schema_version: 1,
            node_id: NodeId::parse_str("n-0001").unwrap(),
            run_id: RunId::parse_str("01arz3ndektsv4rrffq69g5fav").unwrap(),
            parent_node_id: None,
            kind: Kind::Orchestrate,
            status,
            task: None,
            worktree_path: None,
            branch: None,
            base_sha: None,
            tmux_window: None,
            tmux_identity: None,
            agent_pid: None,
            agent_pid_start_time: None,
            supervisor_pid: None,
            children: (0..children)
                .map(|i| octl_core::ChildRef {
                    run_id: RunId::parse_str("01arz3ndektsv4rrffq69g5fav").unwrap(),
                    node_id: NodeId::parse_str(&format!("n-{:04}", i + 2)).unwrap(),
                })
                .collect(),
            started_at: None,
            updated_at,
            last_report: None,
            last_processed_report_seq_by_child: serde_json::Map::new(),
            retry_attempts: 0,
        }
    }

    fn now() -> DateTime<Utc> {
        "2026-08-06T12:00:00Z".parse().unwrap()
    }

    /// (a) An undriven orchestrate driver past the grace window is stalled.
    #[test]
    fn undriven_driver_past_grace_is_stalled() {
        let created = now() - STALL_GRACE - Duration::seconds(1);
        let n = node(Status::Pending, 0, created);
        assert!(is_stalled(
            Status::Pending,
            Kind::Orchestrate,
            Some(&n),
            now()
        ));
    }

    /// (b1) A driver that has spawned a child is being driven — not stalled,
    /// even long past the grace window.
    #[test]
    fn driver_with_child_is_not_stalled() {
        let created = now() - STALL_GRACE - Duration::hours(1);
        let n = node(Status::Pending, 1, created);
        assert!(!is_stalled(
            Status::Pending,
            Kind::Orchestrate,
            Some(&n),
            now()
        ));
    }

    /// (b2) A driver whose node was touched recently (fresh events) is not
    /// stalled, even with zero children yet.
    #[test]
    fn driver_with_recent_activity_is_not_stalled() {
        let recent = now() - Duration::minutes(1);
        let n = node(Status::Pending, 0, recent);
        assert!(!is_stalled(
            Status::Pending,
            Kind::Orchestrate,
            Some(&n),
            now()
        ));
    }

    /// (c) Within the grace window, an undriven driver is not yet stalled.
    #[test]
    fn within_grace_window_is_not_stalled() {
        let created = now() - STALL_GRACE + Duration::seconds(1);
        let n = node(Status::Pending, 0, created);
        assert!(!is_stalled(
            Status::Pending,
            Kind::Orchestrate,
            Some(&n),
            now()
        ));
    }

    /// Exactly at the grace boundary is not yet stalled (strict `>`).
    #[test]
    fn exactly_at_grace_boundary_is_not_stalled() {
        let created = now() - STALL_GRACE;
        let n = node(Status::Pending, 0, created);
        assert!(!is_stalled(
            Status::Pending,
            Kind::Orchestrate,
            Some(&n),
            now()
        ));
    }

    /// A terminal (or otherwise non-`pending`) *manifest* is never flagged, even
    /// when its driver projection stayed `pending` with 0 children and is stale
    /// — a cancelled/done run is not a zombie, and flagging it would tell the
    /// user to cancel an already-terminal run (the review's top finding).
    #[test]
    fn non_pending_run_status_is_never_stalled() {
        let created = now() - STALL_GRACE - Duration::hours(1);
        let n = node(Status::Pending, 0, created);
        for run_status in [
            Status::Running,
            Status::Blocked,
            Status::Done,
            Status::Failed,
            Status::Cancelled,
        ] {
            assert!(
                !is_stalled(run_status, Kind::Orchestrate, Some(&n), now()),
                "run status {run_status:?} must not stall"
            );
        }
    }

    /// A non-`orchestrate` kind is never flagged, however idle — other kinds do
    /// not have the "supervisor adopts but does not drive" shape.
    #[test]
    fn non_orchestrate_kind_is_never_stalled() {
        let created = now() - STALL_GRACE - Duration::hours(1);
        let n = node(Status::Pending, 0, created);
        for k in [Kind::Spinoff, Kind::FanOut, Kind::Orchestrated, Kind::Code] {
            assert!(
                !is_stalled(Status::Pending, k, Some(&n), now()),
                "kind {k:?} must not stall"
            );
        }
    }

    /// A driver node that reached a non-`pending` status (running / blocked /
    /// terminal) is progressing, so it is never flagged even if idle.
    #[test]
    fn non_pending_driver_is_not_stalled() {
        let created = now() - STALL_GRACE - Duration::hours(1);
        for s in [
            Status::Running,
            Status::Blocked,
            Status::Done,
            Status::Failed,
            Status::Cancelled,
        ] {
            let n = node(s, 0, created);
            assert!(
                !is_stalled(Status::Pending, Kind::Orchestrate, Some(&n), now()),
                "driver status {s:?} must not stall"
            );
        }
    }

    /// A run with no driver node yet cannot be judged — not stalled.
    #[test]
    fn missing_driver_node_is_not_stalled() {
        assert!(!is_stalled(Status::Pending, Kind::Orchestrate, None, now()));
    }

    fn created() -> DateTime<Utc> {
        "2026-08-06T11:00:00Z".parse().unwrap()
    }

    /// The exact stillborn signature: pending, dead supervisor, zero nodes, no
    /// forward progress since creation.
    #[test]
    fn stillborn_signature_is_detected() {
        assert!(is_stillborn(
            Status::Pending,
            false,
            0,
            created(),
            created()
        ));
    }

    /// An alive supervisor is a run that is (or may still be) starting — never
    /// stillborn, however fresh.
    #[test]
    fn alive_supervisor_is_not_stillborn() {
        assert!(!is_stillborn(
            Status::Pending,
            true,
            0,
            created(),
            created()
        ));
    }

    /// A run that created its first node started — not stillborn, even with a
    /// dead supervisor (that is an orphaned-but-started run, a different shape).
    #[test]
    fn nonzero_node_count_is_not_stillborn() {
        assert!(!is_stillborn(
            Status::Pending,
            false,
            1,
            created(),
            created()
        ));
    }

    /// Any forward progress (`updated_at` past `created_at`) means the
    /// supervisor did something before dying — not the never-started shape.
    #[test]
    fn forward_progress_is_not_stillborn() {
        let updated = created() + Duration::seconds(1);
        assert!(!is_stillborn(Status::Pending, false, 0, created(), updated));
    }

    /// A non-`pending` run started (and possibly finished) — never stillborn,
    /// whatever the counters say.
    #[test]
    fn non_pending_run_is_not_stillborn() {
        for s in [
            Status::Running,
            Status::Blocked,
            Status::Done,
            Status::Failed,
            Status::Cancelled,
        ] {
            assert!(
                !is_stillborn(s, false, 0, created(), created()),
                "status {s:?} must not be stillborn"
            );
        }
    }
}
