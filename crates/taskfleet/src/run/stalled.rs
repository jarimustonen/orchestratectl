//! Read-time stall detection for a run that cannot progress on its own.
//!
//! Both shapes here — [`is_stillborn`] (a supervisor that died before creating
//! any worker node) and [`is_orphaned`] (a supervisor that died mid-run) — are
//! **computed** read-time hints, not persisted statuses: they touch no
//! event-append / reducer / schema path (state-integrity invariants 1–3 are not
//! in play). Each is derived purely from the manifest plus a single-file
//! supervisor-pid probe, read under the same shared lock the caller already
//! holds for the manifest. Terminal-status semantics are untouched — a stalled
//! run is still `pending`; the flag only says "pending, but visibly not
//! progressing".

use chrono::{DateTime, Duration, Utc};

use taskfleet_core::Status;

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
///   roll the run up is dead (or was never recorded). Combined with the exact
///   `updated_at == created_at` never-progressed signature below, this is
///   unambiguous and needs no grace window (unlike [`is_orphaned`], whose
///   moving manifest clock forces a grace window to tell a *transient* dead-read
///   — a supervisor mid-reattach/restart — from a genuinely stranded run).
/// - `node_count == 0` — not a single worker node was ever created. This makes
///   the check kind-agnostic: a run whose worker node was never even created is
///   stillborn by the same logic, while a run
///   that got as far as `n-0001` is excluded (it started).
/// - `updated_at == created_at` — no manifest-bumping event has been applied
///   since creation, so there has been zero forward progress.
///
/// # Why the timestamp guard is sound (not the fragile check it looks like)
///
/// A reasonable worry is that `supervisor.started` (emitted during supervisor
/// boot, before `run create` returns) would bump `manifest.updated_at` and make
/// this a common false negative. It does not: `supervisor.started` has **no
/// reducer arm** — it folds through the catch-all to a no-op that emits zero
/// projection ops, so it never touches `manifest.updated_at` (verified against
/// `taskfleet-core::reducer`). The first event that bumps the manifest clock on a
/// fresh run is `node.created`, which *also* increments `node_count`. So on a
/// zero-node run `node_count == 0` and `updated_at == created_at` move in
/// lockstep — the guard is redundant-but-robust confirmation, and matches the
/// incident manifest exactly. The `alive` check dominates the healthy path
/// regardless: during the (up to ~90s) create window between `run.created` and
/// `node.created`, the supervisor is alive, so a healthy run is never flagged.
///
/// Residual limitation: a human manually opening a discussion/spinoff on a
/// never-started run *would* bump `updated_at` and defeat the guard — the run
/// then degrades to the old timeout behavior (no new harm). Runs orphaned
/// *after* creating `n-0001` (a supervisor that died mid-run, `node_count > 0`)
/// are handled by the sibling [`is_orphaned`], which needs a grace window
/// because a `node_count > 0` pending/running run is also the shape of a
/// healthy working run.
///
/// Like [`is_orphaned`], this is a **computed** read-time hint over the manifest
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

/// Grace window a `node_count > 0` run may sit idle with a dead supervisor
/// before [`is_orphaned`] trips. Mirrors the supervisor's own in-process
/// `NO_WORKER_GRACE` (15 min, `supervise/mod.rs`): the *alive* supervisor waits
/// that long before terminalizing a stuck run, so a read-time orphan verdict
/// uses the same budget. Long enough that a supervisor briefly between a
/// reattach/restart handoff (its pid file momentarily reads dead while the
/// manifest clock is still fresh) is never misjudged; short enough to catch a
/// genuinely stranded run well inside a caller's default 6h `run wait` timeout.
pub const ORPHAN_GRACE: Duration = Duration::minutes(15);

/// Detect an *orphaned* run: its supervisor created ≥1 worker node and then died
/// mid-run, leaving the run `pending`/`running` with no actor able to roll it up
/// to a terminal status (issue `run-wait-still`). This is the sibling case the
/// stillborn fix (`run-wait-stillborn-run-not-detected`) deliberately scoped
/// out — [`is_stillborn`] handles `node_count == 0` (the supervisor died
/// *before* starting any work); this handles `node_count > 0` (it died *after*).
///
/// Returns `true` only when every part of the stranded signature holds:
///
/// - `status in {Pending, Running}` — a non-terminal run. Terminal runs
///   (`Done`/`Failed`/`Cancelled`) already settled; `Blocked` is a deliberate
///   human-action handoff (a blocked `node.report`), not a stranded supervisor,
///   so it is excluded.
/// - the supervisor is **not alive** — the actor that would roll the run up is
///   gone (or was never recorded). This is the crux: a `node_count > 0`
///   pending/running run with a *live* supervisor is the NORMAL shape of a
///   healthy, heads-down worker, so the liveness probe is what separates
///   "stranded" from "still working" (issue's "why it's harder than stillborn").
/// - `node_count > 0` — at least one node was created. The `== 0` case is
///   [`is_stillborn`] (unambiguous, no grace needed); this one is not.
/// - idle for longer than [`ORPHAN_GRACE`] — `manifest.updated_at` is the last
///   time ANY manifest-bumping event was applied. A dead supervisor stops
///   producing them, so a stale manifest clock alongside a dead supervisor is
///   the stranded signature. The grace window is the crucial guard against a
///   transient dead-read: a supervisor caught mid-reattach/restart (pid file
///   momentarily absent) whose clock is still fresh is NOT flagged. Unlike
///   [`is_stillborn`],
///   which can key off the exact `updated_at == created_at` never-progressed
///   signature and needs no grace, a mid-run orphan has a moving clock and so
///   REQUIRES the idle window to tell "just now" from "long dead".
///
/// A **computed** read-time hint like its siblings — no event-append / reducer /
/// schema path is touched (state-integrity invariants 1–3 are not in play). It
/// reads only fields already held under the caller's shared lock (the manifest)
/// plus the single-file supervisor-pid probe. `now` is injected so the decision
/// is deterministic in tests.
///
/// Clock skew fails closed: a `updated_at` in the future yields a negative
/// `signed_duration_since`, which is never `> ORPHAN_GRACE`, so a skewed clock
/// suppresses the verdict rather than raising a false orphan alarm — the run
/// degrades to the old timeout behavior, no new harm. (The residual weakness is
/// a genuinely-dead supervisor whose transient dead-read coincides with a
/// heads-down worker that has legitimately emitted no manifest event for the
/// grace window; hardening that needs a supervisor heartbeat/lease, tracked as a
/// follow-up. The hint stays advisory — it points at the non-destructive
/// `run reattach`, never a destructive action.)
#[must_use]
pub fn is_orphaned(
    run_status: Status,
    supervisor_alive: bool,
    node_count: u32,
    updated_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> bool {
    matches!(run_status, Status::Pending | Status::Running)
        && !supervisor_alive
        && node_count > 0
        && now.signed_duration_since(updated_at) > ORPHAN_GRACE
}

/// Which read-time "cannot progress on its own" shape a run matches, if any.
/// Both variants are supervisor-dead orphans, distinguished only by how far the
/// run got before the supervisor died. Callers settle the wait identically for
/// either but phrase a slightly different remediation hint per variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StallKind {
    /// Supervisor died *before* creating any worker node (`node_count == 0`):
    /// the run never started. See [`is_stillborn`].
    Stillborn,
    /// Supervisor died *mid-run*, after creating ≥1 node (`node_count > 0`):
    /// the run started but its work is now stranded. See [`is_orphaned`].
    Orphaned,
}

/// Combined read-time stall verdict over one consistent manifest + supervisor
/// snapshot: the run is [`Stillborn`] (never started), [`Orphaned`] (started,
/// then stranded), or neither. Both callers (`run wait`, `run show`) evaluate
/// this under the same shared lock they already hold for the manifest, so the
/// verdict, the run's `status`, and the remediation it prints all come from one
/// view that cannot straddle a reducer write.
///
/// Stillborn is checked first: the two are mutually exclusive on `node_count`
/// (`== 0` vs `> 0`), so the order only formalizes that a zero-node run can
/// never be orphaned.
///
/// [`Stillborn`]: StallKind::Stillborn
/// [`Orphaned`]: StallKind::Orphaned
#[must_use]
pub fn stall_kind(
    run_status: Status,
    supervisor_alive: bool,
    node_count: u32,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Option<StallKind> {
    if is_stillborn(
        run_status,
        supervisor_alive,
        node_count,
        created_at,
        updated_at,
    ) {
        Some(StallKind::Stillborn)
    } else if is_orphaned(run_status, supervisor_alive, node_count, updated_at, now) {
        Some(StallKind::Orphaned)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        "2026-08-06T12:00:00Z".parse().unwrap()
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

    // ── is_orphaned: supervisor died mid-run (node_count > 0) ──────────────

    /// The core orphan signature: a `pending` run with ≥1 node, a dead
    /// supervisor, and a manifest clock idle past the grace window — the exact
    /// "supervisor died mid-run, work stranded" shape (issue `run-wait-still`).
    #[test]
    fn pending_dead_supervisor_past_grace_is_orphaned() {
        let idle = now() - ORPHAN_GRACE - Duration::seconds(1);
        assert!(is_orphaned(Status::Pending, false, 1, idle, now()));
    }

    /// A `running` run (a node reached `running` before the supervisor died) is
    /// orphaned by the same logic — the issue scopes in both pending and running.
    #[test]
    fn running_dead_supervisor_past_grace_is_orphaned() {
        let idle = now() - ORPHAN_GRACE - Duration::minutes(30);
        assert!(is_orphaned(Status::Running, false, 3, idle, now()));
    }

    /// The grace-window guard: a dead supervisor with a still-fresh manifest
    /// clock is NOT orphaned. This is the transient-state protection — a
    /// supervisor caught mid-reattach/restart must not be misread as stranded,
    /// and it is why the existing `run wait` timeout test (a freshly-noded
    /// pending run) keeps blocking rather than settling early.
    #[test]
    fn recently_active_dead_supervisor_is_not_orphaned() {
        let recent = now() - Duration::minutes(1);
        assert!(!is_orphaned(Status::Pending, false, 1, recent, now()));
    }

    /// Exactly at the grace boundary is not yet orphaned (strict `>`), matching
    /// the orchestrate-stall boundary convention.
    #[test]
    fn exactly_at_orphan_grace_boundary_is_not_orphaned() {
        let boundary = now() - ORPHAN_GRACE;
        assert!(!is_orphaned(Status::Pending, false, 1, boundary, now()));
    }

    /// A live supervisor is a normal heads-down worker, never an orphan — this
    /// is the distinction the liveness probe buys over the plain "pending run
    /// with nodes" shape, however long it has been idle.
    #[test]
    fn alive_supervisor_is_never_orphaned() {
        let idle = now() - ORPHAN_GRACE - Duration::hours(2);
        assert!(!is_orphaned(Status::Pending, true, 1, idle, now()));
    }

    /// Zero nodes is the stillborn case, not the orphan case — `is_orphaned`
    /// must not fire on it (they are mutually exclusive on `node_count`).
    #[test]
    fn zero_nodes_is_not_orphaned() {
        let idle = now() - ORPHAN_GRACE - Duration::hours(1);
        assert!(!is_orphaned(Status::Pending, false, 0, idle, now()));
    }

    /// A terminal (or `blocked`) run is never orphaned: terminal runs settled,
    /// and a `blocked` run is a deliberate human-action handoff, not a stranded
    /// supervisor.
    #[test]
    fn terminal_or_blocked_run_is_not_orphaned() {
        let idle = now() - ORPHAN_GRACE - Duration::hours(1);
        for s in [
            Status::Blocked,
            Status::Done,
            Status::Failed,
            Status::Cancelled,
        ] {
            assert!(
                !is_orphaned(s, false, 1, idle, now()),
                "status {s:?} must not be orphaned"
            );
        }
    }

    // ── stall_kind: the combined verdict the callers act on ────────────────

    /// A zero-node never-progressed run classifies as `Stillborn`.
    #[test]
    fn stall_kind_classifies_stillborn() {
        assert_eq!(
            stall_kind(Status::Pending, false, 0, created(), created(), now()),
            Some(StallKind::Stillborn)
        );
    }

    /// A ≥1-node run idle past the grace with a dead supervisor classifies as
    /// `Orphaned`.
    #[test]
    fn stall_kind_classifies_orphaned() {
        let idle = now() - ORPHAN_GRACE - Duration::seconds(1);
        assert_eq!(
            stall_kind(Status::Pending, false, 1, created(), idle, now()),
            Some(StallKind::Orphaned)
        );
    }

    /// A healthy run (live supervisor, or fresh clock) classifies as neither.
    #[test]
    fn stall_kind_healthy_is_none() {
        // Live supervisor with nodes: still working.
        assert_eq!(
            stall_kind(Status::Running, true, 2, created(), now(), now()),
            None
        );
        // Dead supervisor but within the grace window: transient, not yet judged.
        let recent = now() - Duration::minutes(1);
        assert_eq!(
            stall_kind(Status::Pending, false, 1, created(), recent, now()),
            None
        );
    }
}
