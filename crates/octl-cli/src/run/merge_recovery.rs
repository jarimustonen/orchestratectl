//! Deterministic, OID-based recovery for a crashed `run merge` transaction (A2).
//!
//! `run merge` spans two durability domains — git refs and the event log — and
//! is not atomic across them (design.md §2.1b, issue `merge-transaction-recovery`).
//! Before mutating git it records a [`octl_core::MergeTxn`] via a
//! `merge.started` event ([`Node::pending_merge`](octl_core::Node)); on a clean
//! run the terminal `explicit-merge` `node.report` completes the transaction and
//! clears the record. A crash in between leaves a *pending* transaction, which
//! this module resolves — **once, by exact OID, against the ONE recorded
//! transaction**, never a general branch-content heuristic:
//!
//! 1. **Driver-liveness gate.** If the `run merge` process that recorded the
//!    transaction is still alive (PID + start-time identity), the merge is
//!    in-flight — leave it. Recovery only fires for a *crashed* driver, so it
//!    never races a live merge that is about to append its own report.
//! 2. **Compare (the CAS "compare" half).** Read the recorded `source_branch`'s
//!    current OID. If it is still exactly `expected_source_oid`, the git mutation
//!    never landed → **reject** (`merge.aborted`), preserving the worker's branch
//!    and work. This is the merge-start-recorded / no-git-mutation crash window.
//! 3. **Confirm (source moved).** If the source ref moved off
//!    `expected_source_oid`, confirm it moved because of *this* worker's work
//!    using the rebase-robust, git-verified landing check ([`landing_signal`]):
//!    - git-verified landed → **complete**: append the `explicit-merge`
//!      `node.report` the crash prevented (the git-mutated / event-not-yet-
//!      appended crash window). Uses the SAME idempotency key `run merge` uses, so
//!      a racing retry dedupes.
//!    - not landed / unverifiable → **fail closed**: source moved for some other
//!      reason (an unrelated merge, or git could not confirm), so **reject** and
//!      preserve the work rather than fabricate a completion.
//!
//! The git shell-outs run OUTSIDE the run lock (invariant 3); the resolving event
//! is appended under the exclusive lock after re-verifying the transaction is
//! still the one we classified (a concurrent retry may have resolved it).

use serde_json::{json, Value};

use octl_core::report::validate_report_payload;
use octl_core::{
    append_and_apply_idempotent, read_manifest_opt, read_node_opt, AppendOutcome, MergeTxn, NodeId,
    RunLock, RunPaths, VIA_EXPLICIT_MERGE,
};

use crate::run::landed::{landing_signal, LandingInputs};
use crate::supervise::pid_file::pid_live_with_identity;

/// What recovery did with one pending merge transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Recovery {
    /// No transaction was pending on the node.
    NothingPending,
    /// The driving `run merge` process is still alive — the merge is in-flight,
    /// so recovery left it untouched.
    DriverAlive,
    /// Git could not be consulted (missing repo/ref, git error): the transaction
    /// was left pending for a later attempt rather than resolved on a guess.
    CannotVerify,
    /// The transaction was resolved by completing it — the `explicit-merge`
    /// report the crash prevented was appended (or already present).
    Completed,
    /// The transaction was resolved by rejecting it — `merge.aborted` was
    /// appended; the worker's branch and work are preserved.
    Rejected { reason: String },
    /// The transaction we classified was superseded (resolved concurrently, or a
    /// newer attempt replaced it) before we could append — nothing to do.
    Superseded,
}

/// The git verdict for a pending transaction, computed outside the run lock.
enum Verdict {
    Complete,
    Reject { reason: String },
    CannotVerify,
}

/// Resolve every pending merge transaction on this run's nodes. Called from the
/// supervisor tick (the canonical recovery actor) and safe to call repeatedly —
/// each resolution is idempotent and a node with no pending transaction is a
/// cheap no-op.
pub(crate) fn recover_run(paths: &RunPaths, git: &str) {
    // Enumerate node ids under the shared lock (invariant 3), then resolve each
    // outside the scan so the git shell-out never runs under the flock.
    let node_ids = RunLock::with_shared_lock(&paths.lock(), || {
        let mut ids = Vec::new();
        let entries = match std::fs::read_dir(paths.nodes_dir()) {
            Ok(v) => v,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(ids),
            Err(e) => return Err(octl_core::Error::io(paths.nodes_dir(), e)),
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let Ok(nid) = NodeId::parse_str(stem) else {
                continue;
            };
            // Only nodes with a pending transaction are candidates.
            if read_node_opt(paths, &nid)?.is_some_and(|n| n.pending_merge.is_some()) {
                ids.push(nid);
            }
        }
        Ok(ids)
    });
    let node_ids = match node_ids {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                target: "orchestratectl::supervise",
                error = %e,
                "merge-recovery: could not enumerate nodes; will retry next tick"
            );
            return;
        }
    };
    for nid in node_ids {
        match recover_node(paths, &nid, git) {
            Recovery::Completed => tracing::info!(
                target: "orchestratectl::supervise",
                node_id = %nid,
                "merge-recovery: completed a crashed merge transaction (git-verified landed)"
            ),
            Recovery::Rejected { reason } => tracing::warn!(
                target: "orchestratectl::supervise",
                node_id = %nid, reason = %reason,
                "merge-recovery: rejected a crashed merge transaction; work preserved"
            ),
            _ => {}
        }
    }
}

/// Resolve the pending merge transaction (if any) on one node, by exact OID.
///
/// Reads the transaction + repo under the shared lock, gates on driver liveness,
/// computes the git verdict outside the lock, then appends the resolving event
/// under the exclusive lock after re-verifying the transaction is unchanged.
pub(crate) fn recover_node(paths: &RunPaths, node_id: &NodeId, git: &str) -> Recovery {
    // 1. Read the transaction + the repo to probe, under the shared lock.
    let probed = RunLock::with_shared_lock(&paths.lock(), || {
        let node = read_node_opt(paths, node_id)?;
        let manifest = read_manifest_opt(paths)?;
        let source_repo = manifest.as_ref().and_then(|m| m.source_repo.clone());
        let txn = node.as_ref().and_then(|n| n.pending_merge.clone());
        let worktree = node.as_ref().and_then(|n| n.worktree_path.clone());
        let terminal = node.as_ref().is_some_and(|n| n.status.is_terminal());
        Ok((txn, source_repo, worktree, terminal))
    });
    let (txn, source_repo, worktree_path, terminal) = match probed {
        Ok(v) => v,
        Err(_) => return Recovery::CannotVerify,
    };
    let Some(txn) = txn else {
        return Recovery::NothingPending;
    };
    // A terminal node's transaction is already resolved by the terminal report's
    // reducer (which clears `pending_merge`); nothing to recover.
    if terminal {
        return Recovery::NothingPending;
    }

    // 2. Driver-liveness gate: a live `run merge` still owns the transaction.
    if let Some(pid) = txn.driver_pid {
        if pid > 0 && pid_live_with_identity(pid as u32, txn.driver_pid_start_secs) {
            return Recovery::DriverAlive;
        }
    }

    // 3. Classify by OID, outside the lock (git must not run under the flock).
    let verdict = classify(&txn, source_repo.as_deref(), worktree_path.as_deref(), git);
    let verdict = match verdict {
        Verdict::CannotVerify => return Recovery::CannotVerify,
        v => v,
    };

    // 4. Append the resolving event under the exclusive lock, re-verifying the
    //    transaction is still the exact one we classified (a concurrent retry may
    //    have superseded or resolved it in the git-probe window).
    let run_id = paths.run_id.as_str().to_string();
    let outcome = RunLock::with_lock(paths, |lock| {
        let Some(node) = read_node_opt(paths, node_id)? else {
            return Ok(Recovery::Superseded);
        };
        let still_pending = node
            .pending_merge
            .as_ref()
            .is_some_and(|t| t.op_id == txn.op_id);
        if !still_pending {
            return Ok(Recovery::Superseded);
        }
        match &verdict {
            Verdict::Complete => {
                // Append the terminal report the crashed driver would have — same
                // idempotency key as `run merge`, so a racing retry dedupes.
                let report = completion_report(&txn);
                let key = format!("explicit-merge:{run_id}:{node_id}");
                append_and_apply_idempotent(
                    paths,
                    lock,
                    "node.report",
                    Some(node_id),
                    &key,
                    |_seq| Ok(report.clone()),
                )?;
                Ok(Recovery::Completed)
            }
            Verdict::Reject { reason } => {
                let key = format!("merge-aborted:{run_id}:{node_id}:{}", txn.op_id);
                let data = json!({ "op_id": txn.op_id, "reason": reason });
                match append_and_apply_idempotent(
                    paths,
                    lock,
                    octl_core::KIND_MERGE_ABORTED,
                    Some(node_id),
                    &key,
                    |_seq| Ok(data.clone()),
                )? {
                    AppendOutcome::Conflict { .. } => Ok(Recovery::Superseded),
                    _ => Ok(Recovery::Rejected {
                        reason: reason.clone(),
                    }),
                }
            }
            Verdict::CannotVerify => Ok(Recovery::CannotVerify),
        }
    });
    outcome.unwrap_or(Recovery::CannotVerify)
}

/// The deterministic, OID-based verdict for a pending transaction.
///
/// - source ref still at `expected_source_oid` → `Reject` (git mutation never
///   landed — the no-git-mutation crash window),
/// - source ref moved AND the worker's content is git-verified integrated →
///   `Complete` (the git-mutated / event-not-appended crash window),
/// - source ref moved but the content is not git-verified integrated → `Reject`
///   (fail closed: an unrelated move, work preserved),
/// - source ref/repo unreadable → `CannotVerify` (leave pending; don't guess).
fn classify(
    txn: &MergeTxn,
    source_repo: Option<&str>,
    worktree_path: Option<&str>,
    git: &str,
) -> Verdict {
    // The repo to run git in: the durable `source_repo` (survives teardown) or,
    // as a fallback while it still exists, the worker's worktree (a linked
    // worktree shares the common git dir, so the source ref resolves there too).
    let Some(repo) = source_repo.or(worktree_path) else {
        return Verdict::CannotVerify;
    };
    let Some(source_now) = read_oid(git, repo, &txn.source_branch) else {
        // The source ref could not be read (git error, ref gone). Don't guess.
        return Verdict::CannotVerify;
    };
    if source_now == txn.expected_source_oid {
        // Compare failed the CAS: the source ref is still exactly where we left
        // it, so the git mutation never applied. Reject, preserving the work.
        return Verdict::Reject {
            reason: "source ref unchanged — merge never landed".to_string(),
        };
    }
    // Source moved. Confirm it moved because of THIS worker's content, using the
    // same rebase-robust, git-verified landing check `run wait` / `run show` use.
    let inputs = LandingInputs {
        source_repo: Some(repo),
        source_branch: Some(&txn.source_branch),
        worktree_path,
        branch: Some(&txn.worker_branch),
        base_sha: txn.base_sha.as_deref(),
        report: None,
    };
    let signal = landing_signal(&inputs, git);
    // Only a positive GIT-VERIFIED landing completes: a `report-marker` verdict
    // is not applicable here (there is no terminal report yet — that is exactly
    // what the crash prevented), and an `unverified` verdict must fail closed.
    if signal.landed && signal.method == crate::run::landed::LandedMethod::GitVerified {
        Verdict::Complete
    } else {
        Verdict::Reject {
            reason: "source ref moved but the worker's work is not integrated".to_string(),
        }
    }
}

/// The terminal `explicit-merge` report recovery appends to complete a
/// transaction. Mirrors the minimal report `run merge` synthesizes (a clean
/// merge is a success), plus a note that it was recovered.
fn completion_report(txn: &MergeTxn) -> Value {
    let report = json!({
        "success": true,
        "summary": format!(
            "recovered crashed merge of {} into {} (op {})",
            txn.worker_branch, txn.source_branch, txn.op_id
        ),
        "via": VIA_EXPLICIT_MERGE,
    });
    // The reducer validates the terminal outcome; validate the §7.3 shape here
    // too so a future edit to the payload cannot append an invalid report. This
    // payload is constant-shaped, so validation never fails at runtime.
    debug_assert!(validate_report_payload(&report).is_ok());
    report
}

/// `git -C <repo> rev-parse <rev>` → the full object id, or `None` on any git
/// error / non-SHA output. Rejects an option-injection-shaped ref.
pub(crate) fn read_oid(git: &str, repo: &str, rev: &str) -> Option<String> {
    if repo.is_empty() || rev.is_empty() || rev.starts_with('-') {
        return None;
    }
    let out = std::process::Command::new(git)
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "--verify", "--end-of-options", rev])
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let ok = matches!(sha.len(), 40 | 64) && sha.chars().all(|c| c.is_ascii_hexdigit());
    ok.then_some(sha)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::process::Command;

    use octl_core::{append_and_apply_event, read_node_opt, Status};
    use serde_json::json;

    use crate::supervise::cleanup::git_bin;

    fn git(repo: &Path, args: &[&str]) -> String {
        let out = Command::new(git_bin())
            .arg("-C")
            .arg(repo)
            .args(args)
            .output()
            .expect("git runs");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// A git repo with `main` at a base commit and a `wt/worker` branch carrying
    /// one commit of real work. Returns `(dir, base_sha, worker_tip)`.
    fn repo_with_worker() -> (tempfile::TempDir, String, String) {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = dir.path();
        git(repo, &["init", "-q", "-b", "main"]);
        git(repo, &["config", "user.email", "t@t"]);
        git(repo, &["config", "user.name", "t"]);
        std::fs::write(repo.join("f"), "base\n").unwrap();
        git(repo, &["add", "f"]);
        git(repo, &["commit", "-qm", "base"]);
        let base = git(repo, &["rev-parse", "HEAD"]);
        git(repo, &["checkout", "-q", "-b", "wt/worker"]);
        std::fs::write(repo.join("f"), "base\nwork\n").unwrap();
        git(repo, &["commit", "-qam", "worker work"]);
        let worker_tip = git(repo, &["rev-parse", "HEAD"]);
        git(repo, &["checkout", "-q", "main"]);
        (dir, base, worker_tip)
    }

    fn fresh_run(tmp: &tempfile::TempDir, repo: &Path) -> RunPaths {
        let run_id = "01jxsnap000000000000000000";
        let dir = tmp.path().join(run_id);
        std::fs::create_dir_all(&dir).unwrap();
        let paths = RunPaths::new(dir, run_id).unwrap();
        append_and_apply_event(
            &paths,
            "run.created",
            None,
            None,
            json!({
                "kind": "spinoff",
                "lifecycle": "autonomous",
                "title": "t",
                "source_branch": "main",
                "source_repo": repo.to_str().unwrap(),
            }),
        )
        .unwrap();
        append_and_apply_event(
            &paths,
            "node.created",
            Some(&NodeId::parse_str("n-0001").unwrap()),
            None,
            json!({
                "kind": "spinoff",
                "worktree_path": repo.to_str().unwrap(),
                "branch": "wt/worker",
            }),
        )
        .unwrap();
        paths
    }

    fn record_txn(paths: &RunPaths, expected_source_oid: &str, worker_oid: &str, base: &str) {
        let txn = json!({
            "op_id": "01jxop00000000000000000000",
            "source_branch": "main",
            "worker_branch": "wt/worker",
            "expected_source_oid": expected_source_oid,
            "worker_oid": worker_oid,
            "base_sha": base,
            // A definitely-dead driver pid so recovery is not gated on liveness.
            "driver_pid": 2_000_000_000,
            "driver_pid_start_secs": null,
            "started_at": "2026-08-15T00:00:00Z",
        });
        append_and_apply_event(
            paths,
            octl_core::KIND_MERGE_STARTED,
            Some(&NodeId::parse_str("n-0001").unwrap()),
            None,
            txn,
        )
        .unwrap();
    }

    /// Crash window 1: git was mutated (worker's work fast-forwarded into `main`)
    /// but the terminal `explicit-merge` report was never appended. Recovery must
    /// COMPLETE the transaction — terminalize the node `Done` with the merge marker.
    #[test]
    fn recovers_git_mutated_event_not_appended_by_completing() {
        let (repo_dir, base, worker_tip) = repo_with_worker();
        let repo = repo_dir.path();
        // The merge landed: main fast-forwards to the worker's tip. `expected` is
        // the pre-merge source OID (base), so source has moved off it.
        git(repo, &["merge", "-q", "--ff-only", "wt/worker"]);
        assert_ne!(git(repo, &["rev-parse", "main"]), base);

        let home = tempfile::TempDir::new().unwrap();
        let paths = fresh_run(&home, repo);
        record_txn(&paths, &base, &worker_tip, &base);
        let nid = NodeId::parse_str("n-0001").unwrap();
        // Precondition: the transaction is pending and the node is live.
        let n = read_node_opt(&paths, &nid).unwrap().unwrap();
        assert!(n.pending_merge.is_some());
        assert_eq!(n.status, Status::Pending);

        let outcome = recover_node(&paths, &nid, &git_bin());
        assert_eq!(outcome, Recovery::Completed);

        let n = read_node_opt(&paths, &nid).unwrap().unwrap();
        assert_eq!(n.status, Status::Done, "completed merge terminalizes Done");
        assert!(
            n.pending_merge.is_none(),
            "transaction cleared on completion"
        );
        let via = n
            .last_report
            .as_ref()
            .and_then(|r| r.get("via"))
            .and_then(|v| v.as_str());
        assert_eq!(
            via,
            Some(VIA_EXPLICIT_MERGE),
            "explicit-merge report adopted"
        );
    }

    /// Crash window 2: `merge.started` was recorded but git was never mutated (the
    /// source ref is still exactly `expected_source_oid`). Recovery must REJECT the
    /// transaction — fail closed, clear it, and leave the node live with its work
    /// preserved (never fabricate a completion).
    #[test]
    fn rejects_merge_started_with_no_git_mutation() {
        let (repo_dir, base, worker_tip) = repo_with_worker();
        let repo = repo_dir.path();
        // No merge happened: main is still at base == expected_source_oid.
        assert_eq!(git(repo, &["rev-parse", "main"]), base);

        let home = tempfile::TempDir::new().unwrap();
        let paths = fresh_run(&home, repo);
        record_txn(&paths, &base, &worker_tip, &base);
        let nid = NodeId::parse_str("n-0001").unwrap();

        let outcome = recover_node(&paths, &nid, &git_bin());
        assert!(
            matches!(outcome, Recovery::Rejected { .. }),
            "no git mutation → reject, got {outcome:?}"
        );

        let n = read_node_opt(&paths, &nid).unwrap().unwrap();
        assert_eq!(n.status, Status::Pending, "rejected merge leaves node live");
        assert!(
            n.pending_merge.is_none(),
            "transaction cleared on rejection"
        );
        // The worker branch is untouched — its work is preserved.
        assert_eq!(git(repo, &["rev-parse", "wt/worker"]), worker_tip);
    }

    /// A live driver still owns the transaction — recovery must not race it.
    #[test]
    fn leaves_transaction_when_driver_alive() {
        let (repo_dir, base, worker_tip) = repo_with_worker();
        let repo = repo_dir.path();
        let home = tempfile::TempDir::new().unwrap();
        let paths = fresh_run(&home, repo);
        // Record with OUR pid as the driver — very much alive.
        let pid = std::process::id();
        let txn = json!({
            "op_id": "01jxop00000000000000000000",
            "source_branch": "main",
            "worker_branch": "wt/worker",
            "expected_source_oid": base,
            "worker_oid": worker_tip,
            "base_sha": base,
            "driver_pid": pid as i32,
            "driver_pid_start_secs": crate::supervise::watchdog::pid_start_time(pid),
            "started_at": "2026-08-15T00:00:00Z",
        });
        append_and_apply_event(
            &paths,
            octl_core::KIND_MERGE_STARTED,
            Some(&NodeId::parse_str("n-0001").unwrap()),
            None,
            txn,
        )
        .unwrap();
        let nid = NodeId::parse_str("n-0001").unwrap();
        assert_eq!(
            recover_node(&paths, &nid, &git_bin()),
            Recovery::DriverAlive
        );
        assert!(read_node_opt(&paths, &nid)
            .unwrap()
            .unwrap()
            .pending_merge
            .is_some());
    }

    /// Source moved, but NOT to the worker's content (an unrelated commit landed on
    /// main). Recovery must fail closed — REJECT, never fabricate a completion.
    #[test]
    fn rejects_when_source_moved_without_worker_content() {
        let (repo_dir, base, worker_tip) = repo_with_worker();
        let repo = repo_dir.path();
        // An unrelated commit advances main; the worker's work is NOT integrated.
        std::fs::write(repo.join("g"), "unrelated\n").unwrap();
        git(repo, &["add", "g"]);
        git(repo, &["commit", "-qm", "unrelated"]);
        assert_ne!(git(repo, &["rev-parse", "main"]), base);

        let home = tempfile::TempDir::new().unwrap();
        let paths = fresh_run(&home, repo);
        record_txn(&paths, &base, &worker_tip, &base);
        let nid = NodeId::parse_str("n-0001").unwrap();

        let outcome = recover_node(&paths, &nid, &git_bin());
        assert!(
            matches!(outcome, Recovery::Rejected { .. }),
            "unrelated move → fail closed, got {outcome:?}"
        );
        let n = read_node_opt(&paths, &nid).unwrap().unwrap();
        assert_eq!(n.status, Status::Pending);
        assert!(n.pending_merge.is_none());
    }
}
