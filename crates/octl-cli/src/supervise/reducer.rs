//! `node.report` consumption with deterministic-ID dedup (design.md §7.3).
//!
//! For each spinoff/discussion item carried by a child's `node.report`,
//! derive a stable ID from `(child_run_id, child_node_id, report_seq,
//! item_kind, item_index)`. Scan the parent's projection dir for an
//! existing file with that ID; if found, **skip emission** — the
//! supervisor crashed mid-batch on a prior run and is replaying. The
//! deterministic-ID formula plus the projection-existence check is the
//! exactly-once guarantee under crash recovery.
//!
//! Also marks the child's spawning-node `status: done | failed |
//! cancelled` based on the report payload.

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use octl_core::{
    append_and_apply_unlocked, read_node_opt, write_node, DiscussionId, NodeId, ProposalId, RunId,
    RunLock, RunPaths, Status, STATE_SCHEMA_VERSION,
};

use crate::error::CliError;
use crate::supervise::state::SupervisorState;

/// Existence check for a dedup-guard path that never swallows the error.
/// `Path::try_exists` returns `Ok(false)` for a plainly-absent file and errs
/// only when existence is genuinely unknowable (permission/IO). A dedup guard
/// must not guess at that boundary: a wrong "absent" re-emits, a wrong
/// "present" silently drops a never-emitted item, and the caller advances the
/// report cursor either way. Surfacing the error lets the supervisor abort the
/// batch before the cursor moves and retry the report later.
fn exists_or_io_err(path: &std::path::Path) -> Result<bool, CliError> {
    path.try_exists().map_err(|e| {
        CliError::system(
            "io_error",
            format!("cannot check existence of {}: {e}", path.display()),
        )
    })
}

/// `s-<10 base32 chars>` / `d-<10 base32 chars>` (50 bits of entropy).
///
/// Matches the encoding contract in design.md §1.4: base32-lowercase
/// (RFC 4648, no padding) of the leading 50 bits of `sha256(tuple)`.
/// 50 bits is comfortable headroom for per-run dedup (a few hundred
/// items at most) — the 50% birthday-collision midpoint is around
/// ~40M items, far beyond any realistic single-run scope.
///
/// The hashed tuple includes `item_kind` ("discussion" / "spinoff") as
/// belt-and-suspenders alongside the `d-` / `s-` prefix: even if a
/// future caller misroutes a tuple through the wrong prefix, the
/// underlying hash still differs across item kinds.
pub fn deterministic_id(
    prefix: char,
    child_run_id: &str,
    child_node_id: &str,
    report_seq: u64,
    item_kind: &str,
    item_index: usize,
) -> String {
    let mut h = Sha256::new();
    h.update(child_run_id.as_bytes());
    h.update(b":");
    h.update(child_node_id.as_bytes());
    h.update(b":");
    h.update(report_seq.to_string().as_bytes());
    h.update(b":");
    h.update(item_kind.as_bytes());
    h.update(b":");
    h.update(item_index.to_string().as_bytes());
    let digest = h.finalize();
    let head: &[u8; 7] = digest[..7].try_into().expect("sha256 produces 32 bytes");
    let mut out = String::with_capacity(2 + 10);
    out.push(prefix);
    out.push('-');
    out.push_str(&base32_lower_10(head));
    out
}

/// Encode the leading 50 bits of `bytes` as 10 lowercase base32 chars
/// (RFC 4648 alphabet `a-z2-7`, no padding). The fixed `&[u8; 7]`
/// signature makes the length invariant a type-level guarantee —
/// `sha256` digests always have at least 7 bytes, but a future caller
/// who slices wrong gets a compile error instead of silently truncated
/// entropy.
// The `&[u8; 7]` reference is the documented design choice above; the
// by-value efficiency clippy suggests is irrelevant for a 7-byte one-shot.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn base32_lower_10(bytes: &[u8; 7]) -> String {
    const ALPHA: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
    // Pack 7 bytes (56 bits) into a u64 MSB-first, then drop the bottom
    // 6 bits — the remaining 50 bits become ten high-to-low 5-bit groups.
    let mut acc: u64 = 0;
    for &b in bytes {
        acc = (acc << 8) | u64::from(b);
    }
    acc >>= 6;
    let mut out = [0u8; 10];
    for (i, slot) in out.iter_mut().enumerate() {
        let shift = (9 - i) * 5;
        let idx = ((acc >> shift) & 0x1f) as usize;
        *slot = ALPHA[idx];
    }
    std::str::from_utf8(&out)
        .expect("base32 alphabet is ASCII")
        .to_owned()
}

// Fault-injection hook for V7's crash-recovery test. When set to
// `Some(n)`, `process_node_report` panics after writing the `n`-th
// derived event (1-indexed) but before recording the cursor. Always
// `None` outside tests. Thread-local makes the flip race-free under
// `cargo test`.
#[cfg(test)]
thread_local! {
    pub static FAULT_INJECT_AFTER_NTH: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// Outcome of a single report-consumption call. Returned for tests and
/// for the supervisor's own bookkeeping (e.g. logging counts).
#[derive(Debug, Default, Clone)]
pub struct ReportConsumption {
    pub emitted_discussions: Vec<String>,
    pub emitted_spinoffs: Vec<String>,
    pub skipped_already_present: usize,
}

/// Process one `node.report` from `child_run_id` against `parent_paths`,
/// holding the parent run's `flock` for the entire write batch.
///
/// `parent_node_id` is the parent node whose agent spawned this child
/// (the one that should accumulate child references in
/// `last_processed_report_seq_by_child`). `child_node_id` is the
/// reporting node *inside the child run*, typically `n-0001`.
///
/// On success returns the per-batch counts and atomically advances the
/// parent's `last_processed_report_seq_by_child[child_run_id]` in
/// `state`. The caller must then [`crate::supervise::state::save`] the
/// updated state. Returns Ok(None) if `report_seq <= cursor` (already
/// processed) — a fast-path replay guard.
#[allow(clippy::too_many_arguments)]
pub fn process_node_report(
    parent_paths: &RunPaths,
    parent_node_id: &str,
    child_run_id: &str,
    child_node_id: &str,
    report_seq: u64,
    report: &Value,
    state: &mut SupervisorState,
) -> Result<Option<ReportConsumption>, CliError> {
    // Validate every id at the boundary, BEFORE acquiring the lock or emitting
    // any event. Doing it up front means a malformed id can't leave a partial
    // batch of discussion/spinoff events behind (the previous code validated
    // `parent_node_id` only at the end, after writes). `parent_nid` is reused
    // for the node-projection update below; the child ids are validated for
    // their own sake (they feed deterministic-id derivation and state keys).
    let parent_nid = NodeId::parse_str(parent_node_id)
        .map_err(|e| CliError::user("invalid_id", e.to_string()))?;
    RunId::parse_str(child_run_id).map_err(|e| CliError::user("invalid_id", e.to_string()))?;
    NodeId::parse_str(child_node_id).map_err(|e| CliError::user("invalid_id", e.to_string()))?;

    if let Some(prev) = state
        .last_processed_report_seq_by_child
        .get(child_run_id)
        .copied()
    {
        if report_seq <= prev {
            return Ok(None);
        }
    }

    let mut consumption = ReportConsumption::default();
    let mut emitted_count: usize = 0;

    // Hold the parent run's flock for the full write batch. Using
    // `RunLock::acquire` rather than `with_lock` lets the closure body
    // return `CliError` directly instead of going through `core::Error`.
    let guard = RunLock::acquire(&parent_paths.lock())
        .map_err(|e| CliError::system("io_error", e.to_string()))?;
    {
        // Discussions first, then spinoffs — stable order makes the
        // deterministic-ID formula's `item_index` axis unambiguous.
        if let Some(items) = report.get("discussion_items").and_then(Value::as_array) {
            for (i, item) in items.iter().enumerate() {
                let id = deterministic_id(
                    'd',
                    child_run_id,
                    child_node_id,
                    report_seq,
                    "discussion",
                    i,
                );
                // `id` is our own deterministic output (always a valid
                // `d-<base32>`), so a parse failure is an unreachable
                // generator bug — fail loudly rather than smuggling it through
                // as a recoverable error.
                let did = DiscussionId::parse_str(&id)
                    .expect("deterministic_id must produce a valid DiscussionId");
                // Dedup existence check — `try_exists()` errs only on
                // permission/IO faults that leave existence genuinely unknown.
                // Propagate that error rather than guessing: returning `Err`
                // here aborts the batch *before* the cursor is advanced, so the
                // report is retried on the next pass. Neither swallow-as-absent
                // (`Path::exists()` → re-emit) nor swallow-as-present
                // (`unwrap_or(true)` → silently drop a never-emitted item and
                // advance the cursor) is safe; only fail-and-retry loses
                // nothing. A spurious retry is harmless: `apply_discussion_opened`
                // short-circuits on the existing projection.
                if exists_or_io_err(&parent_paths.discussion(&did))? {
                    consumption.skipped_already_present += 1;
                    continue;
                }
                let mut data = serde_json::Map::new();
                data.insert("discussion_id".into(), Value::String(id.clone()));
                if let Some(topic) = item.get("topic") {
                    data.insert("topic".into(), topic.clone());
                } else {
                    data.insert(
                        "topic".into(),
                        Value::String("(no topic supplied)".to_string()),
                    );
                }
                if let Some(sev) = item.get("severity") {
                    data.insert("severity".into(), sev.clone());
                }
                if let Some(opts) = item.get("options") {
                    data.insert("options".into(), opts.clone());
                }
                if let Some(ctx) = item.get("context") {
                    data.insert("context".into(), ctx.clone());
                }
                append_and_apply_unlocked(
                    parent_paths,
                    "discussion.opened",
                    Some(parent_node_id),
                    None,
                    Value::Object(data),
                )
                .map_err(|e| CliError::system("io_error", e.to_string()))?;
                consumption.emitted_discussions.push(id);
                emitted_count += 1;
                fault_inject_check(emitted_count);
            }
        }
        if let Some(items) = report.get("spinoff_proposals").and_then(Value::as_array) {
            for (i, item) in items.iter().enumerate() {
                let id =
                    deterministic_id('s', child_run_id, child_node_id, report_seq, "spinoff", i);
                let pid = ProposalId::parse_str(&id)
                    .expect("deterministic_id must produce a valid ProposalId");
                // Propagate an unknowable existence error rather than guessing
                // — see the discussion-loop note above.
                if exists_or_io_err(&parent_paths.spinoff(&pid))? {
                    consumption.skipped_already_present += 1;
                    continue;
                }
                let mut data = serde_json::Map::new();
                data.insert("proposal_id".into(), Value::String(id.clone()));
                let title = item
                    .get("proposed_title")
                    .cloned()
                    .unwrap_or_else(|| Value::String("(no title)".into()));
                data.insert("proposed_title".into(), title);
                let kind = item
                    .get("proposed_kind")
                    .cloned()
                    .unwrap_or_else(|| Value::String("spinoff".into()));
                data.insert("proposed_kind".into(), kind);
                if let Some(r) = item.get("rationale") {
                    data.insert("rationale".into(), r.clone());
                }
                append_and_apply_unlocked(
                    parent_paths,
                    "spinoff.proposed",
                    Some(parent_node_id),
                    None,
                    Value::Object(data),
                )
                .map_err(|e| CliError::system("io_error", e.to_string()))?;
                consumption.emitted_spinoffs.push(id);
                emitted_count += 1;
                fault_inject_check(emitted_count);
            }
        }

        // Mark the parent-side projection of the *child's* root node by
        // syncing the parent node's `last_processed_report_seq_by_child`
        // map onto its on-disk projection. The state file is the cursor
        // of record; the node-projection mirror is a debugging aid.
        // `parent_nid` was validated at function entry.
        if let Some(mut n) = read_node_opt(parent_paths, &parent_nid)
            .map_err(|e| CliError::system("io_error", e.to_string()))?
        {
            n.last_processed_report_seq_by_child
                .insert(child_run_id.to_string(), json!(report_seq));
            n.schema_version = STATE_SCHEMA_VERSION;
            write_node(parent_paths, &n)
                .map_err(|e| CliError::system("io_error", e.to_string()))?;
        }
    }
    drop(guard);

    state
        .last_processed_report_seq_by_child
        .insert(child_run_id.to_string(), report_seq);
    Ok(Some(consumption))
}

#[inline]
// `_emitted` is unused in non-test builds (hence the underscore) but read
// inside the `cfg(test)` block, which trips `used_underscore_binding`.
#[allow(clippy::used_underscore_binding)]
fn fault_inject_check(_emitted: usize) {
    #[cfg(test)]
    {
        FAULT_INJECT_AFTER_NTH.with(|c| {
            if let Some(n) = c.get() {
                if _emitted >= n {
                    // Clear so retry won't re-trigger.
                    c.set(None);
                    panic!("fault_inject: forced crash after {_emitted} emit(s)");
                }
            }
        });
    }
}

/// Apply the child node's terminal status to the **parent**'s view of
/// that child. The supervisor calls this after `process_node_report`
/// so the parent's `nodes/<spawning>.json` records a snapshot of the
/// child's outcome. The actual child-side `nodes/n-0001.json` is
/// updated by the child supervisor (or the child's own reducer).
#[allow(dead_code)]
pub fn child_terminal_status_from_report(report: &Value) -> Status {
    let cancelled = report
        .get("cancelled")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if cancelled {
        return Status::Cancelled;
    }
    // `Some(false)` (explicit failure) and `None` (missing field) both mean
    // Failed; listed separately to document the missing-field case.
    #[allow(clippy::match_same_arms)]
    match report.get("success").and_then(Value::as_bool) {
        Some(true) => Status::Done,
        Some(false) => Status::Failed,
        None => Status::Failed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_id_is_stable() {
        let a = deterministic_id('d', "run-x", "n-0001", 7, "discussion", 2);
        let b = deterministic_id('d', "run-x", "n-0001", 7, "discussion", 2);
        assert_eq!(a, b);
        assert!(a.starts_with("d-"));
        assert_eq!(a.len(), 2 + 10);
        // Output is base32-lowercase per design.md §1.4: only the RFC
        // 4648 alphabet `a-z2-7` (no padding, no uppercase, no hex).
        for c in a[2..].chars() {
            assert!(
                c.is_ascii_lowercase() || ('2'..='7').contains(&c),
                "non-base32 char {c:?} in {a}"
            );
        }
    }

    /// Lock the encoding contract from design.md §1.4 against a known
    /// input tuple. The expected literal was computed independently
    /// from the spec primitives (sha256 → take 50 bits → base32-lower);
    /// any drift in the formula (input order, separators, hashing,
    /// encoding) trips this assertion.
    #[test]
    fn deterministic_id_formula_matches_design_md_1_4() {
        let got = deterministic_id('d', "run-x", "n-0001", 7, "discussion", 2);
        assert_eq!(got, "d-a4ldwigubn");
    }

    #[test]
    fn base32_lower_10_alphabet_is_rfc4648_lowercase() {
        // Alphabet endpoints.
        assert_eq!(base32_lower_10(&[0u8; 7]), "aaaaaaaaaa");
        assert_eq!(base32_lower_10(&[0xff; 7]), "7777777777");

        // RFC 4648 §10 known vector: "foobar" -> "MZXW6YTBOI======".
        // With a zero 7th byte the first 50 bits are the 48 bits of
        // "foobar" plus two zero pad bits — matches the first 10 chars
        // of the canonical encoding, lowercased.
        assert_eq!(base32_lower_10(b"foobar\0"), "mzxw6ytboi");

        // Asymmetric fixtures that catch MSB/LSB swaps and an off-by-one
        // in the `>>= 6` shift. The high bit of byte 0 must land in the
        // top bit of char 0; bit position 49 (counted from the MSB of
        // the 50-bit window) must land in the low bit of char 9.
        assert_eq!(base32_lower_10(&[0x80, 0, 0, 0, 0, 0, 0]), "qaaaaaaaaa");
        assert_eq!(base32_lower_10(&[0, 0, 0, 0, 0, 0, 0x40]), "aaaaaaaaab");
    }

    #[test]
    fn deterministic_ids_validate_against_core_id_newtypes() {
        // Producer/consumer guard: the supervisor's deterministic_id is fed
        // straight into DiscussionId/ProposalId::parse_str at emit time, so its
        // output MUST satisfy the tightened core validators (10-char RFC 4648
        // base32). If base32_lower_10's alphabet or width ever drifts, replay
        // would break — catch it here instead.
        for seq in [0u64, 1, 7, 42, 1000, u64::MAX] {
            for i in [0usize, 1, 5, 99] {
                let d = deterministic_id(
                    'd',
                    "01jxsnap000000000000000000",
                    "n-0001",
                    seq,
                    "discussion",
                    i,
                );
                DiscussionId::parse_str(&d)
                    .unwrap_or_else(|e| panic!("deterministic discussion id {d} rejected: {e}"));
                let s = deterministic_id(
                    's',
                    "01jxsnap000000000000000000",
                    "n-0001",
                    seq,
                    "spinoff",
                    i,
                );
                ProposalId::parse_str(&s)
                    .unwrap_or_else(|e| panic!("deterministic spinoff id {s} rejected: {e}"));
            }
        }
    }

    #[test]
    fn deterministic_id_differs_per_axis() {
        let base = deterministic_id('s', "r", "n-0001", 1, "spinoff", 0);
        for diff in [
            deterministic_id('s', "r", "n-0001", 1, "spinoff", 1),
            deterministic_id('s', "r2", "n-0001", 1, "spinoff", 0),
            deterministic_id('s', "r", "n-0002", 1, "spinoff", 0),
            deterministic_id('s', "r", "n-0001", 2, "spinoff", 0),
        ] {
            assert_ne!(base, diff);
        }
    }
}
