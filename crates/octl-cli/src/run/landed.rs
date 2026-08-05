//! The rebase-robust `landed` signal surfaced by `run wait` / `run show`.
//!
//! ## Why this exists (issue `landing-signal-reliable-after-rebase`)
//!
//! The `worktree-spinoff` / `stint` skills used to tell the caller to confirm a
//! landing *from git* with
//!
//! ```text
//! git merge-base --is-ancestor <worker-branch> <target>
//! ```
//!
//! That check is a **false-negative trap in exactly the environment `/stint`
//! targets** — a heavy-parallel repo where other sessions push to `origin/main`
//! continuously, forcing the conductor to `git rebase origin/main` its local
//! `main` repeatedly during a round. After such a rebase:
//!
//! - the worker's merge commit is **replayed under a new hash** on the rebased
//!   local `main`;
//! - the worker **branch ref** still points at its **pre-rebase** hash;
//! - `git merge-base --is-ancestor <branch> main` then returns **false** even
//!   though the worker's *content* is fully merged.
//!
//! The conductor concludes "the worker died / didn't land" and nearly takes a
//! destructive recovery action. Observed firing twice in one real session.
//!
//! ## The fix: patch-id equivalence, not branch-ref ancestry
//!
//! [`landing_signal`] computes `landed` by **content**, not by ref ancestry:
//! `git -C <repo> cherry <source_branch> <branch> <base_sha>` reports each of the
//! branch's own commits as `-` (a patch-equivalent commit exists in the target)
//! or `+` (none does). Patch-id equivalence is stable across a rebase — a
//! replayed commit keeps its patch-id even under a new hash — so a landing stays
//! confirmed after the caller rebases their local target. (Verified: even
//! immediately after a rebase-then-ff merge, `--is-ancestor` reads false while
//! `cherry` reads `-`.)
//!
//! When git verification cannot run — the branch ref was already torn down by the
//! supervisor, no `source_repo`/`branch`/`base_sha` was recorded, or git errors —
//! the signal falls back to the durable **report marker**: a terminal
//! `node.report` whose `via` is `explicit-merge` (a `run merge`) or
//! `merge-reconciled` (a supervisor git-reconcile). That marker is the recorded
//! fact that the merge completed; it was correct in the session where the ancestry
//! check lied. Git verification therefore only ever *upgrades* confidence — it
//! never turns a recorded merge into a false negative.
//!
//! The caller reads one boolean (`landed`) plus a `landed_method`
//! (`git-verified` | `report-marker` | `unverified`) that says how the verdict
//! was reached, and never has to run `merge-base --is-ancestor` by hand.

use std::process::{Command, Stdio};

use serde_json::Value;

use crate::supervise::cleanup::VIA_MERGE_RECONCILED;
use octl_core::VIA_EXPLICIT_MERGE;

/// How a [`LandingSignal`] verdict was reached — surfaced as `landed_method`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LandedMethod {
    /// `git cherry` confirmed the branch's commits are patch-present in (or
    /// absent from) the current target tip. Robust to a caller-side rebase.
    GitVerified,
    /// Git verification was unavailable, so the verdict came from the durable
    /// terminal-report `via` marker (`explicit-merge` / `merge-reconciled`).
    ReportMarker,
    /// Neither git verification nor a merge marker was available — `landed` is
    /// `false` because nothing confirms a landing, not because one was disproven.
    Unverified,
}

impl LandedMethod {
    /// Wire string for the `landed_method` field.
    pub(crate) fn wire(self) -> &'static str {
        match self {
            LandedMethod::GitVerified => "git-verified",
            LandedMethod::ReportMarker => "report-marker",
            LandedMethod::Unverified => "unverified",
        }
    }
}

/// The computed landing verdict for one run's reporting node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LandingSignal {
    /// True when the worker's committed work has landed in the target — either
    /// git-confirmed (patch-id present) or attested by the durable merge marker.
    pub landed: bool,
    /// How [`Self::landed`] was decided.
    pub method: LandedMethod,
}

/// The already-read run/node fields [`landing_signal`] needs. The caller reads
/// these under the run's shared lock (state-integrity invariant 3) and then
/// calls `landing_signal` *outside* the lock — the git shell-out must not run
/// while the flock is held.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct LandingInputs<'a> {
    /// `manifest.source_repo` — the repo the target branch lives in. Survives
    /// worktree teardown, so it is preferred over the worktree path.
    pub source_repo: Option<&'a str>,
    /// `manifest.source_branch` — the merge target (`main` for a worktree, an
    /// integration branch for an orchestrated child).
    pub source_branch: Option<&'a str>,
    /// `node.worktree_path` — a fallback repo to run git in while the worktree
    /// still exists and no `source_repo` was recorded.
    pub worktree_path: Option<&'a str>,
    /// `node.branch` — the worker branch ref.
    pub branch: Option<&'a str>,
    /// `node.base_sha` — the fork point, used as `git cherry`'s limit so only the
    /// branch's own commits are examined.
    pub base_sha: Option<&'a str>,
    /// The reporting node's terminal `node.report`, for the marker fallback.
    pub report: Option<&'a Value>,
}

/// Compute the rebase-robust `landed` signal from already-read fields.
///
/// Precedence (see the module docs for the rationale):
/// 1. `git cherry` says every branch commit is patch-present → `git-verified` true.
/// 2. else a durable merge `via` marker is present → `report-marker` true.
/// 3. else `git cherry` positively says a commit is absent → `git-verified` false.
/// 4. else nothing confirms a landing → `unverified` false.
pub(crate) fn landing_signal(inputs: &LandingInputs<'_>, git: &str) -> LandingSignal {
    let report_merged = report_has_merge_marker(inputs.report);
    match git_verify_landed(inputs, git) {
        Some(true) => LandingSignal {
            landed: true,
            method: LandedMethod::GitVerified,
        },
        _ if report_merged => LandingSignal {
            landed: true,
            method: LandedMethod::ReportMarker,
        },
        Some(false) => LandingSignal {
            landed: false,
            method: LandedMethod::GitVerified,
        },
        None => LandingSignal {
            landed: false,
            method: LandedMethod::Unverified,
        },
    }
}

/// True when a terminal report's `via` marks a confirmed successful merge — a
/// `run merge` (`explicit-merge`) or a supervisor git-reconcile
/// (`merge-reconciled`). Both mean the branch landed in source. A blocked
/// handoff (`success: false`, no such `via`) is not a merge and reads false.
fn report_has_merge_marker(report: Option<&Value>) -> bool {
    matches!(
        report.and_then(|r| r.get("via")).and_then(Value::as_str),
        Some(VIA_EXPLICIT_MERGE | VIA_MERGE_RECONCILED)
    )
}

/// Patch-id landing check via `git cherry`. Returns:
/// - `Some(true)`  — the branch has ≥1 commit and *every* one has a
///   patch-equivalent in the target (all `-` lines): landed.
/// - `Some(false)` — at least one branch commit has no equivalent in the target
///   (a `+` line): genuinely not (fully) landed.
/// - `None`        — cannot tell: a required input is missing, the branch ref is
///   gone (deleted at teardown), git errored, or the branch carries no commits to
///   judge. The caller then falls back to the report marker.
///
/// Conservative throughout: any spawn failure, non-zero exit, or unexpected line
/// reads as `None` (never a fabricated `Some(true)`), so a transient git hiccup
/// degrades to the marker fallback rather than a false landing.
fn git_verify_landed(inputs: &LandingInputs<'_>, git: &str) -> Option<bool> {
    let repo = non_empty(inputs.source_repo).or_else(|| non_empty(inputs.worktree_path))?;
    let source = non_empty(inputs.source_branch)?;
    let branch = non_empty(inputs.branch)?;

    let mut cmd = Command::new(git);
    cmd.arg("-C").arg(repo).args(["cherry", source, branch]);
    // Limit `git cherry` to the branch's own commits when we know the fork point;
    // without it, cherry defaults the limit to `source`, which is still correct
    // (it examines the same `source..branch` range) but less precise.
    if let Some(base) = non_empty(inputs.base_sha) {
        cmd.arg(base);
    }
    let out = cmd.stderr(Stdio::null()).output().ok()?;
    if !out.status.success() {
        return None;
    }

    let text = String::from_utf8_lossy(&out.stdout);
    let mut saw_commit = false;
    let mut all_present = true;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        saw_commit = true;
        if let Some(rest) = line.strip_prefix('+') {
            // `+ <sha>`: no patch-equivalent upstream → not landed.
            let _ = rest;
            all_present = false;
        } else if line.starts_with('-') {
            // `- <sha>`: a patch-equivalent commit exists upstream → landed.
        } else {
            // Unexpected shape — refuse to guess.
            return None;
        }
    }
    if !saw_commit {
        // No commits between base and branch to judge (branch never advanced, or
        // was rewound): git cannot confirm a landing. Defer to the marker.
        return None;
    }
    Some(all_present)
}

/// `Some(trimmed-non-empty)` for a present, non-blank string; `None` otherwise.
fn non_empty(s: Option<&str>) -> Option<&str> {
    s.map(str::trim).filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::Path;
    use std::process::Command;

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

    /// Build a repo where a worker branch merged into `main`, then the caller
    /// rebases local `main` (replaying the worker's merge under a new hash while
    /// the worker branch ref stays at its pre-rebase tip) — the exact
    /// false-negative trap `merge-base --is-ancestor` falls into. Returns
    /// `(repo, base_sha, worker_branch)`.
    fn repo_with_rebase_replayed_merge() -> (tempfile::TempDir, String, String) {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = dir.path();
        git(repo, &["init", "-q", "-b", "main"]);
        git(repo, &["config", "user.email", "t@t"]);
        git(repo, &["config", "user.name", "t"]);
        std::fs::write(repo.join("f"), "base\n").unwrap();
        git(repo, &["add", "f"]);
        git(repo, &["commit", "-qm", "base"]);
        let base = git(repo, &["rev-parse", "HEAD"]);

        // Worker branch off base commits real work.
        git(repo, &["checkout", "-q", "-b", "wt/worker"]);
        std::fs::write(repo.join("f"), "base\nwork\n").unwrap();
        git(repo, &["commit", "-qam", "worker change"]);

        // Another session advances main under us.
        git(repo, &["checkout", "-q", "main"]);
        std::fs::write(repo.join("g"), "other\n").unwrap();
        git(repo, &["add", "g"]);
        git(repo, &["commit", "-qm", "other session"]);

        // Merge worker via rebase-then-ff (like merge.sh --rebase). The worker
        // branch ref is left at its pre-rebase tip on purpose (mirrors reality:
        // the ref is not moved to the replayed commit).
        let worker_tip = git(repo, &["rev-parse", "wt/worker"]);
        git(repo, &["checkout", "-q", "-b", "replay", &worker_tip]);
        git(repo, &["rebase", "-q", "main"]);
        git(repo, &["checkout", "-q", "main"]);
        git(repo, &["merge", "-q", "--ff-only", "replay"]);
        git(repo, &["branch", "-q", "-D", "replay"]);

        // The caller rebases local main onto a further-moved origin — replaying
        // the merge under yet another new hash.
        git(repo, &["checkout", "-q", "-b", "tmp", &base]);
        std::fs::write(repo.join("h"), "upstream\n").unwrap();
        git(repo, &["add", "h"]);
        git(repo, &["commit", "-qm", "origin moved"]);
        git(repo, &["checkout", "-q", "main"]);
        git(repo, &["rebase", "-q", "tmp"]);

        (dir, base, "wt/worker".to_string())
    }

    #[test]
    fn git_verified_landed_survives_caller_rebase() {
        let (dir, base, branch) = repo_with_rebase_replayed_merge();
        let repo = dir.path().to_str().unwrap();

        // Precondition: the OLD ancestry check the skills warned about lies here.
        let is_ancestor = Command::new(git_bin())
            .arg("-C")
            .arg(repo)
            .args(["merge-base", "--is-ancestor", &branch, "main"])
            .status()
            .unwrap()
            .success();
        assert!(
            !is_ancestor,
            "the rebase-replay case must make --is-ancestor lie (return false)"
        );

        // The content-based signal reports the truth: landed, git-verified.
        let inputs = LandingInputs {
            source_repo: Some(repo),
            source_branch: Some("main"),
            branch: Some(&branch),
            base_sha: Some(&base),
            ..Default::default()
        };
        let sig = landing_signal(&inputs, &git_bin());
        assert_eq!(
            sig,
            LandingSignal {
                landed: true,
                method: LandedMethod::GitVerified
            },
            "patch-id equivalence must confirm the landing after a caller rebase"
        );
    }

    #[test]
    fn git_verified_not_landed_for_unmerged_branch() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = dir.path();
        git(repo, &["init", "-q", "-b", "main"]);
        git(repo, &["config", "user.email", "t@t"]);
        git(repo, &["config", "user.name", "t"]);
        std::fs::write(repo.join("f"), "base\n").unwrap();
        git(repo, &["add", "f"]);
        git(repo, &["commit", "-qm", "base"]);
        let base = git(repo, &["rev-parse", "HEAD"]);
        // Worker commits but never merges.
        git(repo, &["checkout", "-q", "-b", "wt/worker"]);
        std::fs::write(repo.join("f"), "base\nwork\n").unwrap();
        git(repo, &["commit", "-qam", "unmerged work"]);
        git(repo, &["checkout", "-q", "main"]);

        let repo_s = repo.to_str().unwrap();
        let inputs = LandingInputs {
            source_repo: Some(repo_s),
            source_branch: Some("main"),
            branch: Some("wt/worker"),
            base_sha: Some(&base),
            ..Default::default()
        };
        let sig = landing_signal(&inputs, &git_bin());
        assert_eq!(
            sig,
            LandingSignal {
                landed: false,
                method: LandedMethod::GitVerified
            },
            "a branch with a commit absent from the target is not landed"
        );
    }

    #[test]
    fn falls_back_to_report_marker_when_branch_gone() {
        // No repo/branch resolvable → git verification yields None → the durable
        // merge marker decides. This is the real post-teardown case: the branch
        // ref was force-deleted, but `via: explicit-merge` remains.
        let report = json!({ "success": true, "via": "explicit-merge" });
        let inputs = LandingInputs {
            report: Some(&report),
            ..Default::default()
        };
        let sig = landing_signal(&inputs, "git");
        assert_eq!(
            sig,
            LandingSignal {
                landed: true,
                method: LandedMethod::ReportMarker
            }
        );

        // merge-reconciled counts too.
        let reconciled = json!({ "success": true, "via": "merge-reconciled" });
        let inputs = LandingInputs {
            report: Some(&reconciled),
            ..Default::default()
        };
        assert_eq!(
            landing_signal(&inputs, "git").method,
            LandedMethod::ReportMarker
        );
    }

    #[test]
    fn unverified_when_nothing_confirms() {
        // A blocked handoff: success false, no merge marker, no git inputs.
        let report = json!({ "success": false, "summary": "blocked on X" });
        let inputs = LandingInputs {
            report: Some(&report),
            ..Default::default()
        };
        let sig = landing_signal(&inputs, "git");
        assert_eq!(
            sig,
            LandingSignal {
                landed: false,
                method: LandedMethod::Unverified
            }
        );
    }

    #[test]
    fn git_positive_beats_absent_marker_and_marker_beats_git_none() {
        let (dir, base, branch) = repo_with_rebase_replayed_merge();
        let repo = dir.path().to_str().unwrap();

        // Git confirms even with no report at all.
        let inputs = LandingInputs {
            source_repo: Some(repo),
            source_branch: Some("main"),
            branch: Some(&branch),
            base_sha: Some(&base),
            worktree_path: None,
            report: None,
        };
        assert_eq!(
            landing_signal(&inputs, &git_bin()).method,
            LandedMethod::GitVerified
        );

        // With a bogus branch (git → None) but a merge marker, marker wins true.
        let report = json!({ "via": "explicit-merge" });
        let inputs = LandingInputs {
            source_repo: Some(repo),
            source_branch: Some("main"),
            branch: Some("does/not/exist"),
            base_sha: Some(&base),
            worktree_path: None,
            report: Some(&report),
        };
        assert_eq!(
            landing_signal(&inputs, &git_bin()),
            LandingSignal {
                landed: true,
                method: LandedMethod::ReportMarker
            }
        );
    }
}
