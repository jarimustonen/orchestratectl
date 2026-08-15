//! Single-lock run cancellation.
//!
//! `run cancel` does three things under **one** held [`RunLock`]: refuse a run
//! that is already in a non-cancelled terminal state, synthesize a terminal
//! `node.report` for every still-live node, and append `run.status: cancelled`
//! once. Holding one lock for the whole operation serializes it against other
//! *cooperating* writers (those that honor the lock) so the node reads and the
//! node-report appends can't interleave — which is what made the pre-refactor
//! CLI loop both racy and prone to over-reporting `cancelled_nodes` (it pushed
//! a node id even when the per-node append landed after another process had
//! already settled the node, so the reducer dropped it). Under one lock the
//! node we read is the node we cancel, so the reported count is honest.
//!
//! This is **not crash-atomic**: each `append_and_apply_unlocked` is its own
//! durable append, so a crash or I/O error partway through can leave some nodes
//! cancelled and `run.status` not yet appended. Recovery is convergent — a
//! re-`cancel` of an already-`Cancelled` run scans the still-live stragglers
//! and finishes the job — not transactional rollback.
//!
//! Two consistency properties beyond the single lock:
//!
//! - **Enumeration *and per-node liveness* are from the event log, not the
//!   projection directory.** The node set and each node's current status are
//!   both replayed from `events.jsonl` (the source of truth) in one streaming
//!   pass rather than scanned from `nodes/*.json`. A `node.created` can be
//!   appended+fsynced while its projection write is crash-interrupted
//!   (`events.rs` documents the log leading the projections); a `nodes/` scan
//!   would silently drop that node, mark the run `cancelled`, and let a future
//!   `rebuild_projections` resurrect it as live under a `Cancelled` run. Walking
//!   the log closes that window. Crucially, replaying `node.status` / `node.report`
//!   to derive each node's status (rather than trusting `read_node_opt`) closes a
//!   second window: a *non-cancel* terminal event (e.g. a `node.report`
//!   `success: true`) fsynced but not yet folded leaves a stale-live projection,
//!   and a projection-derived liveness check would over-write that node with a
//!   fresh cancel that diverges on rebuild. The log-derived status settles it as
//!   already-terminal instead — the log wins. (The manifest's `node_count` is
//!   *also* a projection written in the same interrupted fold, so it is no more
//!   authoritative than `nodes/` — and it carries no node ids — which is why we
//!   replay the log rather than trust the counter.)
//!
//! - **Each synthesized event carries a deterministic idempotency key**
//!   (`run-cancel:<run_id>:node:<node_id>` and `run-cancel:<run_id>:run-status`).
//!   If a crash lands an append+fsync but interrupts the projection fold, the
//!   node/run still reads non-terminal, so a re-`cancel` would append a *second*
//!   logical-cancel event (duplicating it for auditors, metrics, and rebuild).
//!   The prior cancel events (scoped by `(kind, key)` for this run) are captured
//!   in the same replay pass, so instead of re-appending, the loop **re-folds
//!   the already-logged event** via [`apply_event`](crate::reducer) — converging a projection
//!   the crash left non-terminal *without* a duplicate log line (a re-fold is a
//!   clean no-op when the projection already agrees). The whole transaction is
//!   then both non-duplicating and projection-convergent.
//!
//! The cancel ledger is built by a *streaming* pass: [`for_each_event_probe`](crate::events)
//! walks `events.jsonl` line by line, parsing only the small envelope + status
//! fields each line needs and materializing a full [`Event`] payload solely for
//! the handful of lines in this run's `run-cancel:<run_id>:` key namespace (the
//! events the re-fold path replays). The whole log is never held in memory, so
//! lock-hold time and peak memory stay bounded even for a run with hundreds of
//! nodes and multi-KB `node.report` payloads.
//!
//! What is still *not* derived from the log here: **run-level** liveness (the
//! terminal-refusal check and `run_was_already_cancelled`) is read from the
//! manifest projection, with the prior-cancel re-fold converging a crash-stranded
//! `run.status`. Deriving the run status from the log too would conflate a
//! crash-stranded `run.status: cancelled` (manifest stale, must re-fold and
//! report a *fresh* cancel) with an already-folded one, since the log is
//! identical in both cases — so the manifest read stays authoritative for the
//! run-level decision, exactly as the per-node convergence path consults
//! `read_node_opt` only to tell those two cases apart.

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::{Error, Result};
use crate::events::{append_and_apply_unlocked, excerpt, for_each_event_probe};
use crate::lock::{LockedRun, RunLock};
use crate::paths::RunPaths;
use crate::projections::{read_manifest, read_node_opt};
use crate::reducer::apply_event;
use crate::schema::{Event, NodeId, RunId, Status};

/// Outcome of a [`cancel_run`] transaction. Lets a thin CLI wrapper report
/// honestly what actually changed: which live nodes it converged, which were
/// already settled (skipped, not double-reported), and whether the run itself
/// was already cancelled (a convergence-only no-op rather than a fresh cancel).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CancelOutcome {
    /// True when the run's manifest was already `Cancelled` on entry, so no
    /// `run.status: cancelled` event was appended. The call still scans and
    /// converges any straggler nodes (an interrupted earlier cancel), so this
    /// is a SUCCESS, not an error: "no-op: run was already cancelled,
    /// converged N additional nodes".
    pub run_was_already_cancelled: bool,
    /// Nodes this cancel transaction ensured are terminally cancelled: live
    /// nodes for which it synthesized and durably appended a terminal cancel
    /// `node.report` (and folded it), plus any node whose cancel `node.report` a
    /// prior interrupted cancel had already durably appended (matched by
    /// `(kind, idempotency_key)`) and which this call converged by *re-folding*
    /// that event rather than re-appending. Either way the node carries a
    /// terminal cancel in the source-of-truth log and its projection is folded
    /// (or, for a still-missing projection, will fold on rebuild — see the
    /// module docs); none is double-reported against a node that was already
    /// terminal on entry.
    pub nodes_cancelled: Vec<NodeId>,
    /// Nodes whose *status* was already terminal on entry and so were skipped —
    /// never double-reported as freshly cancelled.
    pub nodes_already_terminal: Vec<NodeId>,
}

/// Cancel a run in a single locked transaction. Acquires the run's
/// [`RunLock`] once for the whole operation, then delegates to
/// [`cancel_run_unlocked`].
///
/// # Errors
///
/// - [`Error::RunAlreadyTerminal`] if the run is `Done`/`Failed` — refused
///   without mutating state.
/// - I/O / corrupt-log errors from reading the manifest, listing nodes, or
///   appending events.
pub fn cancel_run(paths: &RunPaths, note: Option<&str>) -> Result<CancelOutcome> {
    RunLock::with_lock(paths, |lock| cancel_run_unlocked(lock, paths, note))
}

/// The locked body of [`cancel_run`]. The `lock: &LockedRun` witness proves the
/// caller already holds the run's exclusive [`RunLock`]; this is the sanctioned
/// lock-held composition path so the manifest read, the per-node
/// read-then-append loop, and the final `run.status` append all share one
/// critical section (it calls [`append_and_apply_unlocked`], never
/// [`crate::append_and_apply_event`], which would deadlock by re-locking).
pub fn cancel_run_unlocked(
    lock: &LockedRun<'_>,
    paths: &RunPaths,
    note: Option<&str>,
) -> Result<CancelOutcome> {
    let started = std::time::Instant::now();
    let manifest = read_manifest(paths)?;

    // Refuse a non-cancelled terminal run BEFORE touching any node: cancelling
    // a Done/Failed run would synthesize node reports and append a
    // `run.status: cancelled` the reducer's terminal-state guard then drops,
    // so the CLI would claim a transition that never happened. An already-
    // `Cancelled` run is not refused — it falls through to converge stragglers.
    if manifest.status.is_terminal() && manifest.status != Status::Cancelled {
        return Err(Error::RunAlreadyTerminal {
            status: manifest.status,
        });
    }
    let run_was_already_cancelled = manifest.status == Status::Cancelled;

    // Normalize the cancel reason ONCE up front (see [`normalize_cancel_reason`]):
    // a blank `--note` would flow in as `reason: ""`, which the reducer rejects
    // and would brick the run's cancellability. It falls back to the default.
    let reason = normalize_cancel_reason(note);

    // One streaming replay pass over the source-of-truth log: the authoritative
    // node set *and each node's current status* (both immune to the projection
    // crash window), plus the prior cancel events already recorded (so a prior
    // interrupted cancel isn't duplicated — it is re-folded instead).
    let CancelLedger {
        node_status,
        prior_cancel,
    } = read_cancel_ledger(paths)?;

    let mut nodes_cancelled = Vec::new();
    let mut nodes_already_terminal = Vec::new();

    for (nid, log_status) in node_status {
        let key = node_cancel_key(&paths.run_id, &nid);
        // Convergence path first: this run's cancel already logged a
        // `node.report` for this node (a prior, possibly crash-interrupted,
        // cancel). The log is identical whether that report's projection fold
        // landed or not, so `read_node_opt` is what tells the two apart — an
        // already-folded terminal projection is a clean no-op reported as
        // already-terminal, while a crash-stranded still-live projection is
        // converged by re-folding the already-logged event (no duplicate append)
        // and reported as cancelled. This is the only remaining projection read,
        // and it serves convergence, not the liveness decision below.
        if let Some(prior) = prior_cancel.get(&("node.report".to_owned(), key.clone())) {
            if let Some(n) = read_node_opt(paths, &nid)? {
                if n.status.is_terminal() {
                    nodes_already_terminal.push(nid);
                    continue;
                }
            }
            apply_event(paths, prior)?;
            nodes_cancelled.push(nid);
            continue;
        }
        // No prior cancel for this node: the event log is authoritative for
        // liveness. A node the log replays as terminal — a non-cancel terminal
        // (`node.report success` / a `node.status` to a terminal value), or a
        // cancel logged outside this run's key namespace — is already settled
        // and skipped, even if a stale projection still reads live (the window
        // cancel-liveness-from-log closes: the log wins). Only a node the log
        // shows non-terminal (including a `node.created` whose projection write
        // was interrupted — the crash window a `nodes/*.json` scan would drop)
        // gets a synthesized terminal cancel report so the log records it as
        // cancelled and a future rebuild can't resurrect it as live.
        if log_status.is_terminal() {
            nodes_already_terminal.push(nid);
            continue;
        }
        let data = json!({
            "success": false,
            "cancelled": true,
            "reason": reason,
            "summary": "Run cancelled before agent reported.",
            "discussion_items": [],
            "spinoff_proposals": [],
            "wrap_up_recommendations": []
        });
        append_and_apply_unlocked(lock, paths, "node.report", Some(&nid), Some(&key), data)?;
        nodes_cancelled.push(nid);
    }

    if !run_was_already_cancelled {
        let key = run_status_cancel_key(&paths.run_id);
        if let Some(prior) = prior_cancel.get(&("run.status".to_owned(), key.clone())) {
            // A prior interrupted cancel already logged the terminal `run.status`
            // (fsynced before its manifest fold). Re-fold it to converge the
            // manifest instead of appending a duplicate `run.status: cancelled`.
            apply_event(paths, prior)?;
        } else {
            let mut status_data = serde_json::Map::new();
            status_data.insert("status".into(), "cancelled".into());
            // Record the operator note only when one was actually supplied (the
            // trimmed, non-blank value); a blank `--note` leaves the field unset
            // rather than writing an empty string.
            if let Some(n) = note.map(str::trim).filter(|s| !s.is_empty()) {
                status_data.insert("note".into(), n.into());
            }
            append_and_apply_unlocked(
                lock,
                paths,
                "run.status",
                None,
                Some(&key),
                serde_json::Value::Object(status_data),
            )?;
        }
    }

    tracing::debug!(
        target: "octl_core::cancel",
        run_id = %paths.run_id,
        held_ms = started.elapsed().as_millis() as u64,
        nodes_cancelled = nodes_cancelled.len(),
        nodes_already_terminal = nodes_already_terminal.len(),
        "cancel transaction complete",
    );

    Ok(CancelOutcome {
        run_was_already_cancelled,
        nodes_cancelled,
        nodes_already_terminal,
    })
}

/// Outcome of a [`cancel_node`] transaction — a single-node, branch-preserving
/// cancel for one live fan-out child.
///
/// Unlike [`CancelOutcome`], this **never** carries a run-level decision: per-node
/// cancel deliberately leaves the run non-terminal while any sibling is still
/// live. Terminalizing the run once every node settles is the supervisor's
/// rollup job (`supervise::cleanup::rollup_status`), so a stuck child can be
/// unblocked without killing the batch (design §2.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeCancelOutcome {
    /// The node this call targeted (fully resolved).
    pub node_id: NodeId,
    /// True when this call ensured the node carries a terminal cancel in the
    /// source-of-truth log — either by synthesizing and durably appending a fresh
    /// cancel `node.report`, or by re-folding a prior interrupted cancel's
    /// already-logged event (crash convergence, no duplicate append). False when
    /// the node was already terminal on entry (see `already_terminal`).
    pub cancelled: bool,
    /// True when the node was *already* terminal on entry (merged, failed, or a
    /// prior cancel already folded) — a clean idempotent no-op, never a fresh
    /// cancel. Mutually exclusive with `cancelled`.
    pub already_terminal: bool,
}

/// Cancel exactly ONE live node of a run, preserving its branch + worktree.
/// Acquires the run's [`RunLock`] once and delegates to
/// [`cancel_node_unlocked`].
///
/// This is the fan-out selectivity primitive (design §2.5, issue
/// `per-node-run`): where [`cancel_run`] settles every live node and rolls the
/// run up to `Cancelled` in one shot, this settles a single named node and
/// **appends no `run.status`** — the run stays live while its siblings run, and
/// the supervisor's rollup terminalizes the batch once the last node settles.
/// The synthesized terminal cancel `node.report` classifies as
/// [`Cancelled`](crate::Status) → `Teardown::SourceRelative`, so invariant 5
/// preserves the node's committed work rather than force-deleting it.
///
/// # Errors
///
/// - [`Error::NodeNotFound`] if `node_id` names no node in the run's log.
/// - I/O / corrupt-log errors from reading the manifest, replaying the log, or
///   appending the report.
pub fn cancel_node(
    paths: &RunPaths,
    node_id: &NodeId,
    note: Option<&str>,
) -> Result<NodeCancelOutcome> {
    RunLock::with_lock(paths, |lock| {
        cancel_node_unlocked(lock, paths, node_id, note)
    })
}

/// The locked body of [`cancel_node`]. The `lock: &LockedRun` witness proves the
/// caller already holds the run's exclusive [`RunLock`], so the log replay, the
/// convergence read, and the single report append share one critical section
/// (it calls [`append_and_apply_unlocked`], never [`crate::append_and_apply_event`],
/// which would deadlock by re-locking).
pub fn cancel_node_unlocked(
    lock: &LockedRun<'_>,
    paths: &RunPaths,
    node_id: &NodeId,
    note: Option<&str>,
) -> Result<NodeCancelOutcome> {
    // One streaming replay pass over the source-of-truth log gives the
    // authoritative node set *and* each node's log-derived status (both immune to
    // the projection crash window), plus this run's already-logged cancel events
    // so a prior interrupted cancel is re-folded, never duplicated.
    let CancelLedger {
        node_status,
        prior_cancel,
    } = read_cancel_ledger(paths)?;

    // The log is authoritative for the node set: a node whose `node.created` was
    // fsynced but whose projection write was crash-interrupted is still
    // resolvable here (a `nodes/*.json` scan would miss it). A genuinely absent
    // id is a caller error.
    let log_status = node_status
        .iter()
        .find(|(nid, _)| nid == node_id)
        .map(|(_, s)| *s)
        .ok_or_else(|| Error::NodeNotFound {
            node_id: node_id.as_str().to_owned(),
        })?;

    let key = node_cancel_key(&paths.run_id, node_id);

    // Convergence path first: this run's cancel already logged a `node.report`
    // for this node (a prior, possibly crash-interrupted, per-node or whole-run
    // cancel). The log is identical whether that report's projection fold landed
    // or not, so `read_node_opt` tells the two apart — an already-folded terminal
    // projection is a clean no-op (already-terminal), while a crash-stranded
    // still-live projection is converged by re-folding the already-logged event
    // (no duplicate append).
    if let Some(prior) = prior_cancel.get(&("node.report".to_owned(), key.clone())) {
        if let Some(n) = read_node_opt(paths, node_id)? {
            if n.status.is_terminal() {
                return Ok(NodeCancelOutcome {
                    node_id: node_id.clone(),
                    cancelled: false,
                    already_terminal: true,
                });
            }
        }
        apply_event(paths, prior)?;
        return Ok(NodeCancelOutcome {
            node_id: node_id.clone(),
            cancelled: true,
            already_terminal: false,
        });
    }

    // No prior cancel for this node: the log is authoritative for liveness. A
    // node the log replays as terminal — a natural success/failure, or a cancel
    // logged outside this run's key namespace — is already settled and reported
    // as such, even if a stale projection still reads live (the log wins).
    if log_status.is_terminal() {
        return Ok(NodeCancelOutcome {
            node_id: node_id.clone(),
            cancelled: false,
            already_terminal: true,
        });
    }

    let reason = normalize_cancel_reason(note);
    let data = json!({
        "success": false,
        "cancelled": true,
        "reason": reason,
        "summary": "Node cancelled before agent reported.",
        "discussion_items": [],
        "spinoff_proposals": [],
        "wrap_up_recommendations": []
    });
    append_and_apply_unlocked(lock, paths, "node.report", Some(node_id), Some(&key), data)?;
    Ok(NodeCancelOutcome {
        node_id: node_id.clone(),
        cancelled: true,
        already_terminal: false,
    })
}

/// Normalize a `--note` into the terminal cancel report's `reason`. An empty or
/// whitespace-only note would otherwise flow in as `reason: ""`, which the
/// reducer rejects (`CancelledRequiresReason`) — aborting the transaction and,
/// since a retry reuses the same bad note, leaving the node/run permanently
/// un-cancellable. A blank note falls back to the default.
fn normalize_cancel_reason(note: Option<&str>) -> &str {
    note.map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("cancelled by user")
}

/// Cancel-relevant facts replayed from `events.jsonl` in one streaming pass
/// under the held lock.
struct CancelLedger {
    /// Every node a `node.created` event introduced, paired with the status the
    /// log replays for it, deduped and sorted by numeric suffix. The
    /// authoritative live-node set *and* per-node liveness: replayed from the
    /// source of truth, so it includes a node whose projection write was
    /// crash-interrupted (the node a `nodes/*.json` scan would miss) and reports
    /// a node terminal whenever the log says so even if the projection still
    /// reads live (the window [`crate::events`] documents the log leading the
    /// projections through).
    node_status: Vec<(NodeId, Status)>,
    /// Cancel events this run already logged, keyed by `(kind, idempotency_key)`
    /// and limited to this run's `run-cancel:<run_id>:` key namespace (first
    /// occurrence wins, mirroring [`crate::events::find_prior_with_key`]). The
    /// cancel loop looks an entry up by its deterministic key to (a) avoid
    /// re-appending a duplicate and (b) re-fold the event so a crash-stranded
    /// projection converges. Keying by `(kind, key)` — not the bare string —
    /// keeps a coincidental or forged key on an unrelated `kind` from masking a
    /// real cancel append. Only these few lines have their full [`Event`] payload
    /// materialized; every other line is skimmed envelope-only.
    prior_cancel: HashMap<(String, String), Event>,
}

/// Envelope + the few small `data` fields the cancel ledger needs from each
/// line, skimmed by [`for_each_event_probe`] without materializing the
/// (potentially multi-KB) full `data` payload. serde ignores every other field,
/// so a rich `node.report` is scanned but never allocated.
#[derive(Deserialize)]
struct CancelProbe {
    kind: String,
    #[serde(default)]
    node_id: Option<NodeId>,
    #[serde(default)]
    idempotency_key: Option<String>,
    #[serde(default)]
    data: CancelProbeData,
}

/// The status-bearing `data` fields of `node.status` / `node.report`. All
/// optional: any other event kind simply leaves them `None`.
#[derive(Deserialize, Default)]
struct CancelProbeData {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    success: Option<bool>,
    #[serde(default)]
    cancelled: Option<bool>,
}

/// Replay `events.jsonl` once, streaming, to build the [`CancelLedger`].
///
/// Reads through [`RunPaths::checked_events`] so a symlinked event log is
/// refused, matching the mutation path — the cancel decision must not be made
/// from content redirected outside the run tree. Uses [`for_each_event_probe`],
/// which shares the crate's torn-tail policy: a crash-truncated final line is
/// dropped as an uncommitted partial write, while any *interior* unparseable
/// line is surfaced as [`Error::CorruptEventLog`] — so a corrupt log fails the
/// cancel loudly rather than silently dropping a node. A missing log yields an
/// empty ledger (run never appended an event — nothing to cancel).
///
/// Per-node status is accumulated by a tiny per-node state machine that mirrors
/// the reducer's terminal-state guard: `node.created` seeds [`Status::Pending`]
/// (idempotent on replay), and `node.status` / `node.report` transition a node
/// only while it is still non-terminal. A `node.status` / `node.report` for a
/// node id never introduced by a `node.created` is ignored, exactly as the
/// reducer no-ops a status/report against a non-existent node. Malformed status
/// fields degrade gracefully (the node is left non-terminal, hence cancelled)
/// rather than aborting the whole cancel — the append path already validates
/// every committed event, so this only matters for a hand-corrupted log.
///
/// Node ids are sorted by numeric suffix (not lexically), so output and the
/// cancel order stay intuitive past the digit-width boundary where `n-10000`
/// would otherwise sort before `n-9999` (see [`NodeId`]).
fn read_cancel_ledger(paths: &RunPaths) -> Result<CancelLedger> {
    let events_path = paths.checked_events()?;
    let prefix = format!("run-cancel:{}:", paths.run_id.as_str());
    // Creation order is preserved here and re-sorted by numeric suffix below.
    let mut order: Vec<NodeId> = Vec::new();
    let mut status: HashMap<NodeId, Status> = HashMap::new();
    let mut prior_cancel: HashMap<(String, String), Event> = HashMap::new();

    for_each_event_probe::<CancelProbe, _>(&events_path, |probe, raw| {
        match probe.kind.as_str() {
            "node.created" => {
                if let Some(nid) = &probe.node_id {
                    // Idempotent on replay: a second `node.created` for the same
                    // id is a no-op, mirroring the reducer's existence guard.
                    if !status.contains_key(nid) {
                        order.push(nid.clone());
                        status.insert(nid.clone(), Status::Pending);
                    }
                }
            }
            "node.status" => {
                if let Some(nid) = &probe.node_id {
                    if let Some(cur) = status.get_mut(nid) {
                        if !cur.is_terminal() {
                            if let Some(ns) = probe.data.status.as_deref().and_then(parse_status) {
                                *cur = ns;
                            }
                        }
                    }
                }
            }
            "node.report" => {
                if let Some(nid) = &probe.node_id {
                    if let Some(cur) = status.get_mut(nid) {
                        // Terminal guard *before* deriving the outcome, mirroring
                        // the reducer: a report against an already-terminal node
                        // is a dead event (its payload may even be a bare `{}`).
                        if !cur.is_terminal() {
                            if let Some(ns) =
                                report_terminal_status(probe.data.success, probe.data.cancelled)
                            {
                                *cur = ns;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        // Capture only this run's cancel events, keyed by (kind, key), and only
        // for those materialize the full payload the re-fold path needs.
        if let Some(key) = probe
            .idempotency_key
            .as_deref()
            .filter(|k| k.starts_with(&prefix))
        {
            let entry = (probe.kind.clone(), key.to_owned());
            if let std::collections::hash_map::Entry::Vacant(slot) = prior_cancel.entry(entry) {
                let ev: Event =
                    serde_json::from_slice(raw).map_err(|e| Error::CorruptEventLog {
                        path: events_path.clone(),
                        reason: format!(
                            "cancel ledger: line matched a run-cancel key but is not a \
                         replayable event: {} [{e}]",
                            excerpt(raw)
                        ),
                    })?;
                slot.insert(ev);
            }
        }
        Ok(())
    })?;

    // A validated `NodeId` is `n-` + ASCII digits (≤10, so it fits in u64); the
    // unwrap_or keeps the sort total even for a hypothetical unparseable body.
    order.sort_by_key(|id| {
        id.as_str()
            .strip_prefix("n-")
            .and_then(|d| d.parse::<u64>().ok())
            .unwrap_or(0)
    });
    let node_status = order
        .into_iter()
        .map(|id| {
            let s = status[&id];
            (id, s)
        })
        .collect();
    Ok(CancelLedger {
        node_status,
        prior_cancel,
    })
}

/// Parse a `node.status` / `run.status` status string into a [`Status`],
/// returning `None` for an unrecognized value (treated as "no transition" so a
/// corrupt status never aborts the cancel). Goes through serde so the kebab-case
/// mapping can never drift from the [`Status`] enum.
fn parse_status(s: &str) -> Option<Status> {
    serde_json::from_value(Value::String(s.to_owned())).ok()
}

/// Derive the terminal status a `node.report` asserts from its `success` /
/// `cancelled` flags, mirroring the reducer's success-XOR-cancelled rule but
/// *lenient*: a bare/contradictory report yields `None` (no transition — the
/// node stays live and is cancelled) rather than the reducer's
/// [`Error::CorruptEventLog`]. The append path rejects such a report before it
/// is ever committed against a live node, so a `None` here only arises from a
/// hand-corrupted log, where leaving the node cancellable is the safe default.
fn report_terminal_status(success: Option<bool>, cancelled: Option<bool>) -> Option<Status> {
    if cancelled.unwrap_or(false) {
        // `cancelled: true` with `success: true` is contradictory → no transition.
        if success == Some(true) {
            return None;
        }
        Some(Status::Cancelled)
    } else {
        match success {
            Some(true) => Some(Status::Done),
            Some(false) => Some(Status::Failed),
            None => None,
        }
    }
}

/// Deterministic idempotency key for the synthesized cancel `node.report` of
/// one node. Stable in `(run_id, node_id)` so a re-`cancel` after a crash that
/// fsynced the report but never folded its projection finds the prior event and
/// does not append a duplicate logical-cancel.
fn node_cancel_key(run_id: &RunId, node_id: &NodeId) -> String {
    format!("run-cancel:{}:node:{}", run_id.as_str(), node_id.as_str())
}

/// Deterministic idempotency key for the run's terminal `run.status: cancelled`
/// event. Stable in `run_id` for the same crash-retry reason as
/// [`node_cancel_key`].
fn run_status_cancel_key(run_id: &RunId) -> String {
    format!("run-cancel:{}:run-status", run_id.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{append_and_apply_event, append_event_with_seq, read_all_events};
    use crate::lock::ACQUIRE_COUNT;
    use tempfile::TempDir;

    /// Count `node.report` events recorded in the log for one node id.
    fn report_count(paths: &RunPaths, nid: &str) -> usize {
        read_all_events(&paths.events())
            .unwrap()
            .iter()
            .filter(|e| {
                e.kind == "node.report" && e.node_id.as_ref().map(NodeId::as_str) == Some(nid)
            })
            .count()
    }

    fn fresh_run(tmp: &TempDir) -> RunPaths {
        let run_id = "01jxsnap000000000000000000";
        let dir = tmp.path().join(run_id);
        std::fs::create_dir_all(&dir).unwrap();
        RunPaths::new(dir, run_id).unwrap()
    }

    /// Parse a `NodeId` for a test append call (the typed envelope id).
    fn nid(s: &str) -> NodeId {
        NodeId::parse_str(s).unwrap()
    }

    /// Drive a run to `count` live nodes (n-0001..) under a created manifest.
    fn bootstrap(paths: &RunPaths, count: usize) {
        append_and_apply_event(
            paths,
            "run.created",
            None,
            None,
            json!({ "kind": "spinoff", "lifecycle": "autonomous", "title": "t" }),
        )
        .unwrap();
        for i in 1..=count {
            let node_id = nid(&format!("n-{i:04}"));
            append_and_apply_event(
                paths,
                "node.created",
                Some(&node_id),
                None,
                json!({ "kind": "spinoff" }),
            )
            .unwrap();
        }
    }

    fn node_status(paths: &RunPaths, nid: &str) -> Status {
        let id = NodeId::parse_str(nid).unwrap();
        crate::read_node(paths, &id).unwrap().status
    }

    #[test]
    fn cancel_running_run_converges_live_nodes_and_settles_run() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 2);

        let out = cancel_run(&paths, Some("stop")).unwrap();
        assert!(!out.run_was_already_cancelled);
        assert_eq!(
            out.nodes_cancelled
                .iter()
                .map(NodeId::as_str)
                .collect::<Vec<_>>(),
            vec!["n-0001", "n-0002"]
        );
        assert!(out.nodes_already_terminal.is_empty());
        assert_eq!(node_status(&paths, "n-0001"), Status::Cancelled);
        assert_eq!(
            crate::read_manifest(&paths).unwrap().status,
            Status::Cancelled
        );
    }

    #[test]
    fn cancel_done_run_is_refused_without_mutation() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 1);
        // Settle the single node, then the run, to Done.
        append_and_apply_event(
            &paths,
            "node.report",
            Some(&nid("n-0001")),
            None,
            json!({ "success": true }),
        )
        .unwrap();
        append_and_apply_event(
            &paths,
            "run.status",
            None,
            None,
            json!({ "status": "done" }),
        )
        .unwrap();
        let before = read_all_events(&paths.events()).unwrap().len();

        let err = cancel_run(&paths, None).unwrap_err();
        assert!(
            matches!(
                err,
                Error::RunAlreadyTerminal {
                    status: Status::Done
                }
            ),
            "got {err:?}"
        );
        assert_eq!(
            read_all_events(&paths.events()).unwrap().len(),
            before,
            "a refused cancel must not append any event"
        );
        assert_eq!(crate::read_manifest(&paths).unwrap().status, Status::Done);
    }

    #[test]
    fn recancel_cancelled_run_converges_straggler_node() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 2);
        // Simulate an interrupted cancel: run is Cancelled, but n-0002 is still
        // live (its node.report never landed).
        append_and_apply_event(
            &paths,
            "node.report",
            Some(&nid("n-0001")),
            None,
            json!({ "success": false, "cancelled": true, "reason": "x" }),
        )
        .unwrap();
        append_and_apply_event(
            &paths,
            "run.status",
            None,
            None,
            json!({ "status": "cancelled" }),
        )
        .unwrap();
        assert_eq!(node_status(&paths, "n-0002"), Status::Pending);

        let out = cancel_run(&paths, None).unwrap();
        assert!(out.run_was_already_cancelled);
        assert_eq!(
            out.nodes_cancelled
                .iter()
                .map(NodeId::as_str)
                .collect::<Vec<_>>(),
            vec!["n-0002"],
            "only the straggler converges"
        );
        assert_eq!(
            out.nodes_already_terminal
                .iter()
                .map(NodeId::as_str)
                .collect::<Vec<_>>(),
            vec!["n-0001"]
        );
        assert_eq!(node_status(&paths, "n-0002"), Status::Cancelled);
    }

    #[test]
    fn recancel_fully_converged_run_is_a_clean_noop() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 1);
        cancel_run(&paths, None).unwrap(); // first cancel converges everything
        let before = read_all_events(&paths.events()).unwrap().len();

        let out = cancel_run(&paths, None).unwrap();
        assert!(out.run_was_already_cancelled);
        assert!(out.nodes_cancelled.is_empty(), "nothing left to converge");
        assert_eq!(
            out.nodes_already_terminal
                .iter()
                .map(NodeId::as_str)
                .collect::<Vec<_>>(),
            vec!["n-0001"]
        );
        assert_eq!(
            read_all_events(&paths.events()).unwrap().len(),
            before,
            "a fully-converged re-cancel appends nothing"
        );
    }

    #[test]
    fn already_terminal_node_is_not_over_reported() {
        // The honesty guard: a node already settled (terminal) on entry is
        // reported under `nodes_already_terminal`, never `nodes_cancelled`,
        // even though it sits in nodes/ alongside a live node.
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 2);
        // n-0001 finishes on its own (Done) before the cancel.
        append_and_apply_event(
            &paths,
            "node.report",
            Some(&nid("n-0001")),
            None,
            json!({ "success": true }),
        )
        .unwrap();

        let out = cancel_run(&paths, None).unwrap();
        assert_eq!(
            out.nodes_cancelled
                .iter()
                .map(NodeId::as_str)
                .collect::<Vec<_>>(),
            vec!["n-0002"]
        );
        assert_eq!(
            out.nodes_already_terminal
                .iter()
                .map(NodeId::as_str)
                .collect::<Vec<_>>(),
            vec!["n-0001"]
        );
        assert_eq!(
            node_status(&paths, "n-0001"),
            Status::Done,
            "Done node untouched"
        );
        assert_eq!(node_status(&paths, "n-0002"), Status::Cancelled);
    }

    #[test]
    fn cancel_run_with_no_nodes_dir_settles_run_only() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        append_and_apply_event(
            &paths,
            "run.created",
            None,
            None,
            json!({ "kind": "spinoff", "lifecycle": "autonomous", "title": "t" }),
        )
        .unwrap();

        let out = cancel_run(&paths, None).unwrap();
        assert!(!out.run_was_already_cancelled);
        assert!(out.nodes_cancelled.is_empty());
        assert!(out.nodes_already_terminal.is_empty());
        assert_eq!(
            crate::read_manifest(&paths).unwrap().status,
            Status::Cancelled
        );
    }

    #[test]
    fn blank_note_falls_back_to_default_reason_and_does_not_brick_cancel() {
        // A `--note ""` (or whitespace-only) must NOT flow an empty `reason`
        // into the synthesized report — that would be rejected by the reducer
        // mid-loop and leave the run permanently un-cancellable. It normalizes
        // to the default reason and the cancel completes cleanly.
        for blank in ["", "   ", "\n\t"] {
            let tmp = TempDir::new().unwrap();
            let paths = fresh_run(&tmp);
            bootstrap(&paths, 1);

            let out = cancel_run(&paths, Some(blank)).unwrap();
            assert_eq!(
                out.nodes_cancelled
                    .iter()
                    .map(NodeId::as_str)
                    .collect::<Vec<_>>(),
                vec!["n-0001"],
                "blank note {blank:?} still converges the live node"
            );
            assert_eq!(node_status(&paths, "n-0001"), Status::Cancelled);
            let report = crate::read_node(&paths, &NodeId::parse_str("n-0001").unwrap())
                .unwrap()
                .last_report
                .expect("cancel report recorded");
            assert_eq!(report["reason"], "cancelled by user");
        }
    }

    #[test]
    fn nodes_are_converged_in_numeric_not_lexical_order() {
        // Past the digit-width boundary, lexical order would place n-10000
        // before n-9999. The numeric sort keeps the reported order intuitive.
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        append_and_apply_event(
            &paths,
            "run.created",
            None,
            None,
            json!({ "kind": "spinoff", "lifecycle": "autonomous", "title": "t" }),
        )
        .unwrap();
        for node in ["n-9999", "n-10000", "n-0001"] {
            append_and_apply_event(
                &paths,
                "node.created",
                Some(&nid(node)),
                None,
                json!({ "kind": "spinoff" }),
            )
            .unwrap();
        }

        let out = cancel_run(&paths, None).unwrap();
        assert_eq!(
            out.nodes_cancelled
                .iter()
                .map(NodeId::as_str)
                .collect::<Vec<_>>(),
            vec!["n-0001", "n-9999", "n-10000"],
        );
    }

    #[test]
    fn cancel_synthesizes_report_for_node_with_missing_projection() {
        // The crash window this fix closes: a `node.created` was appended+fsynced
        // to the log, but its projection write (`nodes/n-NNNN.json`) was
        // interrupted. A `nodes/*.json` scan would not see n-0002 and would
        // cancel the run while leaving a created-but-never-cancelled node a
        // future rebuild could resurrect as live. Enumerating from the event log
        // sees it and synthesizes the cancel report.
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 2);
        // Delete n-0002's projection file, leaving its `node.created` event in
        // the log — exactly the interrupted-fold state.
        let n2 = NodeId::parse_str("n-0002").unwrap();
        std::fs::remove_file(paths.node(&n2)).unwrap();
        assert!(
            read_node_opt(&paths, &n2).unwrap().is_none(),
            "projection gone"
        );

        let out = cancel_run(&paths, Some("stop")).unwrap();
        // Both nodes are cancelled — the projection-present n-0001 AND the
        // projection-missing n-0002.
        assert_eq!(
            out.nodes_cancelled
                .iter()
                .map(NodeId::as_str)
                .collect::<Vec<_>>(),
            vec!["n-0001", "n-0002"],
            "the node with a missing projection is still cancelled"
        );
        assert!(out.nodes_already_terminal.is_empty());
        // The source-of-truth log now carries a terminal cancel report for the
        // node whose projection was missing — so a rebuild reconstructs it as
        // Cancelled, not live.
        assert_eq!(report_count(&paths, "n-0002"), 1);
        assert_eq!(
            crate::read_manifest(&paths).unwrap().status,
            Status::Cancelled
        );
    }

    #[test]
    fn cancel_takes_the_run_lock_exactly_once() {
        // The single-lock honesty guarantee: the whole transaction (N node
        // reports + the run.status append) runs under ONE flock acquisition, not
        // one per appended event. Spy on `RunLock::acquire` to prove it.
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 5);

        // Bootstrap itself takes the lock once per append; only the cancel call
        // is under measurement.
        ACQUIRE_COUNT.with(|c| c.set(0));
        let out = cancel_run(&paths, Some("stop")).unwrap();
        assert_eq!(out.nodes_cancelled.len(), 5);
        assert_eq!(
            ACQUIRE_COUNT.with(std::cell::Cell::get),
            1,
            "cancel must take the run lock exactly once, not once per node (N+1)"
        );
    }

    #[test]
    fn cancel_does_not_duplicate_a_node_report_already_in_the_log() {
        // Crash-retry idempotency: a prior cancel appended+fsynced a node's
        // cancel `node.report` (carrying the deterministic key) but crashed
        // before folding its projection, so the node still reads live. A
        // re-cancel must NOT append a second logical-cancel event for it.
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 1); // run.created (seq 1) + node.created (seq 2)

        // Durably append the cancel report WITH the deterministic key, but
        // without folding it — the node stays Pending (live), modeling the
        // fsynced-but-not-applied window.
        let node = nid("n-0001");
        let key = node_cancel_key(&paths.run_id, &node);
        RunLock::with_lock(&paths, |lock| {
            append_event_with_seq(
                lock,
                &paths,
                3,
                "node.report",
                Some(&node),
                Some(&key),
                json!({ "success": false, "cancelled": true, "reason": "x" }),
            )
        })
        .unwrap();
        assert_eq!(node_status(&paths, "n-0001"), Status::Pending);
        assert_eq!(report_count(&paths, "n-0001"), 1);

        let out = cancel_run(&paths, None).unwrap();
        // The node converges (it is reported cancelled) but no duplicate report
        // is appended — the log still holds exactly one `node.report` for it.
        assert_eq!(
            out.nodes_cancelled
                .iter()
                .map(NodeId::as_str)
                .collect::<Vec<_>>(),
            vec!["n-0001"],
        );
        assert_eq!(
            report_count(&paths, "n-0001"),
            1,
            "the already-logged cancel report must not be duplicated"
        );
        // Convergence: the crash-stranded projection is folded from the
        // already-logged event, so the node reads Cancelled (not the stale
        // Pending) even though no new event was appended for it.
        assert_eq!(
            node_status(&paths, "n-0001"),
            Status::Cancelled,
            "the already-logged cancel must be re-folded, not just skipped"
        );
        assert_eq!(
            crate::read_manifest(&paths).unwrap().status,
            Status::Cancelled
        );
    }

    #[test]
    fn cancel_does_not_duplicate_run_status_already_in_the_log() {
        // The run-status analogue: a prior cancel fsynced `run.status: cancelled`
        // (with its deterministic key) but crashed before folding the manifest,
        // so the manifest still reads non-terminal. A re-cancel must not append a
        // second `run.status: cancelled`.
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 0); // run.created only (seq 1)

        let key = run_status_cancel_key(&paths.run_id);
        RunLock::with_lock(&paths, |lock| {
            append_event_with_seq(
                lock,
                &paths,
                2,
                "run.status",
                None,
                Some(&key),
                json!({ "status": "cancelled" }),
            )
        })
        .unwrap();
        // Manifest never folded the cancel, so it is not terminal here.
        assert_ne!(
            crate::read_manifest(&paths).unwrap().status,
            Status::Cancelled
        );
        let before = read_all_events(&paths.events()).unwrap().len();

        let out = cancel_run(&paths, None).unwrap();
        assert!(!out.run_was_already_cancelled);
        assert_eq!(
            read_all_events(&paths.events()).unwrap().len(),
            before,
            "no duplicate run.status appended when one is already logged"
        );
        // Convergence: the manifest is folded from the already-logged
        // `run.status: cancelled` instead of being left stale.
        assert_eq!(
            crate::read_manifest(&paths).unwrap().status,
            Status::Cancelled,
            "the already-logged run.status must be re-folded, not just skipped"
        );
    }

    #[test]
    fn cancel_skips_node_terminal_in_log_despite_stale_live_projection() {
        // cancel-liveness-from-log: a non-cancel terminal event (here a
        // `node.status` to a terminal value) was fsynced to the log but its
        // projection fold was crash-interrupted, so `nodes/n-0001.json` still
        // reads the stale live (Pending) status. The cancel must derive liveness
        // from the LOG and treat the node as already-terminal — never
        // synthesizing a cancel that would over-write the log's terminal and
        // diverge on a future rebuild (which replays node.status: done FIRST and
        // drops the later cancel).
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 2); // run.created(1) + node.created n-0001(2), n-0002(3)

        // Raw-append (no fold) a terminal `node.status` for n-0001: the log
        // records it Done, but the projection stays the stale crash-window
        // Pending.
        let n1 = nid("n-0001");
        RunLock::with_lock(&paths, |lock| {
            append_event_with_seq(
                lock,
                &paths,
                4,
                "node.status",
                Some(&n1),
                None,
                json!({ "status": "done" }),
            )
        })
        .unwrap();
        assert_eq!(
            node_status(&paths, "n-0001"),
            Status::Pending,
            "projection is the stale, crash-stranded live status"
        );

        let out = cancel_run(&paths, Some("stop")).unwrap();
        // n-0001 is settled by the log, NOT freshly cancelled; only the
        // genuinely live n-0002 is cancelled.
        assert_eq!(
            out.nodes_already_terminal
                .iter()
                .map(NodeId::as_str)
                .collect::<Vec<_>>(),
            vec!["n-0001"],
            "the log-terminal node is reported already-terminal, not cancelled"
        );
        assert_eq!(
            out.nodes_cancelled
                .iter()
                .map(NodeId::as_str)
                .collect::<Vec<_>>(),
            vec!["n-0002"],
        );
        // No cancel report was synthesized for n-0001: the log still holds zero
        // `node.report` lines for it, so a rebuild reconstructs it from the
        // `node.status: done` (Done), not a divergent Cancelled.
        assert_eq!(
            report_count(&paths, "n-0001"),
            0,
            "no cancel over-write was appended for the log-terminal node"
        );
    }

    #[test]
    fn cancel_skips_node_with_unfolded_success_report_in_log() {
        // The issue's headline case: a `node.report { success: true }` fsynced
        // but not folded leaves a stale-live projection. Liveness from the log
        // settles the node as Done (already-terminal); the old projection-derived
        // check would have wrongly cancelled it over its already-logged success.
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 1); // run.created(1) + node.created n-0001(2)
        let n1 = nid("n-0001");
        RunLock::with_lock(&paths, |lock| {
            append_event_with_seq(
                lock,
                &paths,
                3,
                "node.report",
                Some(&n1),
                None,
                json!({ "success": true }),
            )
        })
        .unwrap();
        assert_eq!(
            node_status(&paths, "n-0001"),
            Status::Pending,
            "stale live projection (success report fsynced but not folded)"
        );

        let out = cancel_run(&paths, None).unwrap();
        assert_eq!(
            out.nodes_already_terminal
                .iter()
                .map(NodeId::as_str)
                .collect::<Vec<_>>(),
            vec!["n-0001"],
        );
        assert!(
            out.nodes_cancelled.is_empty(),
            "a node the log shows Done must not be cancelled"
        );
        assert_eq!(
            report_count(&paths, "n-0001"),
            1,
            "only the original success report remains; no cancel was appended"
        );
    }

    #[test]
    fn cancel_ledger_streams_large_report_payloads() {
        // The streaming ledger skims each line's envelope + a few small status
        // fields, never materializing the (here multi-KB) `node.report` `data`
        // payload. A node settled by such a report is still correctly seen as
        // terminal from the log, and a live sibling is still cancelled — proving
        // liveness is derived without holding whole reports in memory.
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 2);
        let big = "x".repeat(64 * 1024);
        append_and_apply_event(
            &paths,
            "node.report",
            Some(&nid("n-0001")),
            None,
            json!({ "success": true, "summary": big }),
        )
        .unwrap();
        assert_eq!(node_status(&paths, "n-0001"), Status::Done);

        let out = cancel_run(&paths, Some("stop")).unwrap();
        assert_eq!(
            out.nodes_already_terminal
                .iter()
                .map(NodeId::as_str)
                .collect::<Vec<_>>(),
            vec!["n-0001"],
        );
        assert_eq!(
            out.nodes_cancelled
                .iter()
                .map(NodeId::as_str)
                .collect::<Vec<_>>(),
            vec!["n-0002"],
        );
    }

    // --- per-node cancel (`cancel_node`) -----------------------------------

    #[test]
    fn cancel_node_settles_one_node_and_leaves_the_run_and_siblings_live() {
        // The fan-out headline: cancelling one live child settles ONLY that node,
        // preserves it as Cancelled, and leaves the run + every sibling untouched
        // and non-terminal — the supervisor's rollup (not this call) terminalizes
        // the batch later.
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 3);

        let out = cancel_node(&paths, &nid("n-0002"), Some("stuck")).unwrap();
        assert_eq!(out.node_id.as_str(), "n-0002");
        assert!(out.cancelled);
        assert!(!out.already_terminal);

        assert_eq!(node_status(&paths, "n-0002"), Status::Cancelled);
        assert_eq!(node_status(&paths, "n-0001"), Status::Pending);
        assert_eq!(node_status(&paths, "n-0003"), Status::Pending);
        assert!(
            !crate::read_manifest(&paths).unwrap().status.is_terminal(),
            "no run.status is appended by a per-node cancel while siblings are live"
        );
        // The synthesized report carries the branch-preserving cancel shape.
        let report = crate::read_node(&paths, &nid("n-0002"))
            .unwrap()
            .last_report
            .expect("cancel report recorded");
        assert_eq!(report["cancelled"], true);
        assert_eq!(report["success"], false);
        assert_eq!(report["reason"], "stuck");
    }

    #[test]
    fn cancel_node_unknown_id_is_node_not_found() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 1);
        let err = cancel_node(&paths, &nid("n-0009"), None).unwrap_err();
        assert!(
            matches!(err, Error::NodeNotFound { ref node_id } if node_id == "n-0009"),
            "got {err:?}"
        );
    }

    #[test]
    fn cancel_node_on_already_terminal_node_is_idempotent_noop() {
        // A node that finished on its own (Done) is reported already-terminal,
        // never freshly cancelled, and no cancel report is appended over its
        // success.
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 2);
        append_and_apply_event(
            &paths,
            "node.report",
            Some(&nid("n-0001")),
            None,
            json!({ "success": true }),
        )
        .unwrap();

        let out = cancel_node(&paths, &nid("n-0001"), None).unwrap();
        assert!(!out.cancelled);
        assert!(out.already_terminal);
        assert_eq!(node_status(&paths, "n-0001"), Status::Done, "untouched");
        assert_eq!(report_count(&paths, "n-0001"), 1, "no cancel over-write");
    }

    #[test]
    fn cancel_node_twice_does_not_duplicate_the_report() {
        // Idempotent duplicate per-node cancel: the second call converges/no-ops
        // and never appends a second cancel `node.report`.
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 2);

        let first = cancel_node(&paths, &nid("n-0001"), Some("x")).unwrap();
        assert!(first.cancelled);
        assert_eq!(report_count(&paths, "n-0001"), 1);

        let second = cancel_node(&paths, &nid("n-0001"), Some("x")).unwrap();
        assert!(!second.cancelled);
        assert!(second.already_terminal);
        assert_eq!(
            report_count(&paths, "n-0001"),
            1,
            "a duplicate per-node cancel must not append a second report"
        );
        assert_eq!(node_status(&paths, "n-0001"), Status::Cancelled);
    }

    #[test]
    fn cancel_node_converges_a_crash_stranded_prior_cancel_without_duplicating() {
        // Crash-retry: a prior cancel fsynced the node's cancel `node.report`
        // (with the deterministic key) but crashed before folding the projection,
        // so the node still reads live. A re-cancel re-folds the logged event
        // (node → Cancelled) without appending a second report.
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 1); // run.created(1) + node.created(2)
        let node = nid("n-0001");
        let key = node_cancel_key(&paths.run_id, &node);
        RunLock::with_lock(&paths, |lock| {
            append_event_with_seq(
                lock,
                &paths,
                3,
                "node.report",
                Some(&node),
                Some(&key),
                json!({ "success": false, "cancelled": true, "reason": "x" }),
            )
        })
        .unwrap();
        assert_eq!(node_status(&paths, "n-0001"), Status::Pending);

        let out = cancel_node(&paths, &node, None).unwrap();
        assert!(out.cancelled, "the stranded cancel is converged");
        assert!(!out.already_terminal);
        assert_eq!(report_count(&paths, "n-0001"), 1, "no duplicate append");
        assert_eq!(node_status(&paths, "n-0001"), Status::Cancelled);
    }

    #[test]
    fn cancel_node_resolves_a_node_with_a_missing_projection() {
        // The log — not the `nodes/*.json` scan — is authoritative for the node
        // set: a node whose projection write was crash-interrupted is still
        // cancellable (and its cancel report lands so a rebuild reconstructs it
        // Cancelled, not live).
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 2);
        let n2 = nid("n-0002");
        std::fs::remove_file(paths.node(&n2)).unwrap();
        assert!(read_node_opt(&paths, &n2).unwrap().is_none());

        let out = cancel_node(&paths, &n2, Some("stop")).unwrap();
        assert!(out.cancelled);
        // The source-of-truth log now carries the terminal cancel report, so a
        // rebuild reconstructs the node Cancelled (its projection stays absent —
        // the reducer folds a report without resurrecting a deleted projection,
        // exactly as the whole-run cancel does).
        assert_eq!(report_count(&paths, "n-0002"), 1);
    }

    #[test]
    fn cancel_node_blank_note_falls_back_to_default_reason() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 1);
        let out = cancel_node(&paths, &nid("n-0001"), Some("   ")).unwrap();
        assert!(out.cancelled);
        let report = crate::read_node(&paths, &nid("n-0001"))
            .unwrap()
            .last_report
            .expect("cancel report recorded");
        assert_eq!(report["reason"], "cancelled by user");
    }

    #[test]
    fn cancel_node_takes_the_run_lock_exactly_once() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 3);
        ACQUIRE_COUNT.with(|c| c.set(0));
        let out = cancel_node(&paths, &nid("n-0002"), Some("x")).unwrap();
        assert!(out.cancelled);
        assert_eq!(
            ACQUIRE_COUNT.with(std::cell::Cell::get),
            1,
            "per-node cancel must take the run lock exactly once"
        );
    }

    #[test]
    fn cancel_last_live_node_leaves_run_live_for_the_rollup() {
        // Cancelling the final live node still appends NO run.status — the run
        // stays non-terminal, and terminalizing it is the supervisor rollup's job
        // (which sees every node Cancelled and rolls up). This asserts the
        // division of labor the design mandates.
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 2);
        cancel_node(&paths, &nid("n-0001"), Some("x")).unwrap();
        let out = cancel_node(&paths, &nid("n-0002"), Some("x")).unwrap();
        assert!(out.cancelled);
        assert_eq!(node_status(&paths, "n-0001"), Status::Cancelled);
        assert_eq!(node_status(&paths, "n-0002"), Status::Cancelled);
        assert_ne!(
            crate::read_manifest(&paths).unwrap().status,
            Status::Cancelled,
            "per-node cancel never terminalizes the run itself"
        );
    }
}
