//! Minimal, deterministic git helpers for the live pipeline (design.md §7
//! branch lifecycle). Impure by nature — they shell out — but kept small and
//! self-contained so the [`super`] driver's orchestration logic is the part
//! under test, exercised against a real throwaway git repo (offline,
//! deterministic) rather than mocked git.
//!
//! Honours the crate-wide `GIT_BIN` override (mirrors `floor::git::git_bin`,
//! `harness::support::git_bin`, `supervise::cleanup::git_bin`) so a test can
//! point at a fixture.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::PipelineError;

/// The `git` binary, honouring the `GIT_BIN` override.
fn git_bin() -> String {
    std::env::var("GIT_BIN").unwrap_or_else(|_| "git".to_string())
}

/// A `git -C <dir>` command with a stabilized environment (`LC_ALL=C`, no
/// path re-encoding) so any message we must read is locale-independent.
/// `commit.gpgsign=false` is forced so a merge commit created under a user
/// whose global config signs commits cannot block on a gpg passphrase prompt
/// (which would wedge the non-interactive pipeline).
///
/// A deterministic committer/author identity (`user.name` / `user.email`) is
/// forced too (item H): every commit-creating helper here — `merge_no_ff`,
/// `cherry_pick` — otherwise relies on the ambient git identity, so an
/// identity-less CI/sandbox would fail to create the merge/replay commit with
/// `*** Please tell me who you are`. Pinning it also keeps the pipeline's own
/// commits attributed to the tool rather than to whoever's config happens to be
/// in scope. Passed as `-c` overrides (not env) so they win over any repo/global
/// config without mutating it.
fn git_at(dir: &Path) -> Command {
    let mut cmd = Command::new(git_bin());
    cmd.arg("-C")
        .arg(dir)
        .args([
            "-c",
            "core.quotePath=false",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "user.name=orchestratectl pipeline",
            "-c",
            "user.email=pipeline@orchestratectl.local",
        ])
        .env("LC_ALL", "C");
    cmd
}

/// Run a git subcommand in `dir`, returning trimmed stdout on success.
///
/// # Errors
///
/// Returns [`PipelineError::Git`] on a spawn failure or a non-zero exit.
pub fn git(dir: &Path, args: &[&str]) -> Result<String, PipelineError> {
    let out = git_at(dir).args(args).output().map_err(|e| {
        PipelineError::Git(format!(
            "could not run git {} in {}: {e}",
            args.join(" "),
            dir.display()
        ))
    })?;
    if !out.status.success() {
        return Err(PipelineError::Git(format!(
            "git {} failed in {}: {}",
            args.join(" "),
            dir.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The repository's top-level directory (the git worktree root of `dir`).
pub fn toplevel(dir: &Path) -> Result<PathBuf, PipelineError> {
    Ok(PathBuf::from(git(dir, &["rev-parse", "--show-toplevel"])?))
}

/// Resolve `rev` to a canonical commit oid, verifying it exists and is a commit.
pub fn resolve_commit(dir: &Path, rev: &str) -> Result<String, PipelineError> {
    git(
        dir,
        &["rev-parse", "--verify", &format!("{rev}^{{commit}}")],
    )
    .map_err(|_| {
        PipelineError::Git(format!(
            "`{rev}` does not resolve to a commit in {}",
            dir.display()
        ))
    })
}

/// Whether a local branch exists.
pub fn branch_exists(dir: &Path, branch: &str) -> bool {
    git_at(dir)
        .args([
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ])
        .status()
        .is_ok_and(|s| s.success())
}

/// Create a new branch `name` starting at `start`.
pub fn create_branch(dir: &Path, name: &str, start: &str) -> Result<(), PipelineError> {
    git(dir, &["branch", name, start])?;
    Ok(())
}

/// Delete a branch. `force` uses `-D` (only ever called for a confirmed-merged
/// branch); the default `-d` refuses to drop an unmerged branch (state-integrity
/// invariant 5: never silently drop unmerged work).
pub fn delete_branch(dir: &Path, name: &str, force: bool) -> Result<(), PipelineError> {
    let flag = if force { "-D" } else { "-d" };
    git(dir, &["branch", flag, name])?;
    Ok(())
}

/// Add a worktree at `path` checked out on the existing `branch`.
pub fn worktree_add(dir: &Path, path: &Path, branch: &str) -> Result<(), PipelineError> {
    git(
        dir,
        &["worktree", "add", &path.display().to_string(), branch],
    )?;
    Ok(())
}

/// Add a worktree at `path` on a NEW branch `new_branch` forked from `start`.
pub fn worktree_add_new_branch(
    dir: &Path,
    path: &Path,
    new_branch: &str,
    start: &str,
) -> Result<(), PipelineError> {
    git(
        dir,
        &[
            "worktree",
            "add",
            "-b",
            new_branch,
            &path.display().to_string(),
            start,
        ],
    )?;
    Ok(())
}

/// Remove a worktree (force, since it may hold uncommitted throwaway state).
/// Best-effort: a failure to remove a scratch worktree is not fatal to the
/// pipeline result, so the caller logs and continues.
pub fn worktree_remove(dir: &Path, path: &Path) -> Result<(), PipelineError> {
    git(
        dir,
        &["worktree", "remove", "--force", &path.display().to_string()],
    )?;
    Ok(())
}

/// The current `HEAD` oid of a worktree.
pub fn head(worktree: &Path) -> Result<String, PipelineError> {
    resolve_commit(worktree, "HEAD")
}

/// Whether the worktree has no uncommitted changes (tracked or untracked).
pub fn is_clean(worktree: &Path) -> Result<bool, PipelineError> {
    Ok(git(worktree, &["status", "--porcelain"])?.is_empty())
}

/// Whether `ancestor` is an ancestor of `descendant` (i.e. HEAD only moved
/// forward from the base). `git merge-base --is-ancestor` exits 0 for yes, 1 for
/// no; any other failure is surfaced. Used to reject a chunk that rewrote history
/// instead of forking forward from its assigned base.
pub fn is_ancestor(dir: &Path, ancestor: &str, descendant: &str) -> Result<bool, PipelineError> {
    let out = git_at(dir)
        .args(["merge-base", "--is-ancestor", ancestor, descendant])
        .output()
        .map_err(|e| {
            PipelineError::Git(format!(
                "could not run git merge-base in {}: {e}",
                dir.display()
            ))
        })?;
    match out.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(PipelineError::Git(format!(
            "git merge-base --is-ancestor failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))),
    }
}

/// Whether the range `base..tip` contains a merge commit (any commit with more
/// than one parent). The provenance-aware rollback (item 1) replays each kept
/// chunk with `git cherry-pick base..commit`, and a plain range cherry-pick
/// cannot replay a merge commit without `-m <parent>` — it aborts with
/// `error: commit … is a merge but no -m option was given`, which the rollback
/// then reports as a spurious conflict. So a chunk whose history is non-linear is
/// rejected at gate time (item F) rather than becoming an un-replayable kept
/// chunk later. `git rev-list --merges --count` reports how many merge commits
/// the range contains.
pub fn range_has_merge(dir: &Path, base: &str, tip: &str) -> Result<bool, PipelineError> {
    let out = git(
        dir,
        &["rev-list", "--merges", "--count", &format!("{base}..{tip}")],
    )?;
    let n = out.trim().parse::<usize>().map_err(|e| {
        PipelineError::Git(format!(
            "could not parse rev-list --merges count {out:?}: {e}"
        ))
    })?;
    Ok(n > 0)
}

/// Hard-reset `worktree` to `rev` and remove untracked files/dirs, restoring it
/// to exactly the tree at `rev`. Used to discard any side effects a
/// planner/judge (spec/verify) stage left in the worktree, so ONLY the
/// floor-gated chunk content can ever reach the source branch — the LLM stages
/// run headless with skipped permissions and could otherwise mutate the tree
/// between the gate and the merge.
pub fn restore_to(worktree: &Path, rev: &str) -> Result<(), PipelineError> {
    git(worktree, &["reset", "--hard", rev])?;
    git(worktree, &["clean", "-fdq"])?;
    Ok(())
}

/// The unified diff of `base..tip` in `worktree` (`git diff base tip`). Used to
/// carry a failed re-code attempt's diff into the next re-brief so the model does
/// not lose the failing work when its worktree is torn down (re-code amnesia fix).
/// The output is capped at [`DIFF_CAP_BYTES`]; an over-cap diff is truncated with a
/// trailing marker so a pathological diff cannot bloat the re-brief prompt.
pub fn diff(worktree: &Path, base: &str, tip: &str) -> Result<String, PipelineError> {
    let out = git(worktree, &["diff", base, tip])?;
    if out.len() > DIFF_CAP_BYTES {
        // Truncate at the last newline at or before the cap, so a hunk is never cut
        // mid-line (a partial `-`/`+` line could read as a spurious edit). Fall back
        // to a char boundary if there is no newline in range (keeps the String
        // valid). Only the retained prefix — not the marker — is bounded by the cap.
        let mut end = DIFF_CAP_BYTES.min(out.len());
        while end > 0 && !out.is_char_boundary(end) {
            end -= 1;
        }
        if let Some(nl) = out[..end].rfind('\n') {
            end = nl;
        }
        Ok(format!(
            "{}\n… [diff truncated at ~{DIFF_CAP_BYTES} bytes]",
            &out[..end]
        ))
    } else {
        Ok(out)
    }
}

/// Cap on the failing-diff snippet folded into a re-brief (item 3). Big enough to
/// carry a real chunk's diff, small enough that an adversarial/huge diff cannot
/// blow up the re-code prompt.
const DIFF_CAP_BYTES: usize = 16 * 1024;

/// Replay the commits in `base..tip` onto the branch checked out in `worktree`
/// with `git cherry-pick` (a 3-way apply, so an independent chunk lands cleanly on
/// a rebuilt integration branch). Used by the provenance-aware rollback (item 1):
/// after resetting `feat/<slug>` to the fork, each kept-done chunk's own commits
/// are cherry-picked back in original order. On a conflict the cherry-pick is
/// aborted so the worktree is left clean, and [`MergeOutcome::Conflict`] is
/// returned rather than an error — the caller decides how to surface it.
///
/// `--empty=drop` (item G) handles a kept chunk whose change is already present
/// on the rebuilt tip (e.g. two chunks made the same edit, or an upstream chunk
/// subsumed it): such a commit would otherwise become empty mid-replay and stop
/// the cherry-pick with `The previous cherry-pick is now empty` — which this
/// helper would misread as a conflict and abort. Dropping the redundant commit
/// replays the rest cleanly; the chunk simply contributes no new commit.
pub fn cherry_pick(worktree: &Path, base: &str, tip: &str) -> Result<MergeOutcome, PipelineError> {
    let out = git_at(worktree)
        .args(["cherry-pick", "--empty=drop", &format!("{base}..{tip}")])
        .output()
        .map_err(|e| {
            PipelineError::Git(format!(
                "could not run git cherry-pick in {}: {e}",
                worktree.display()
            ))
        })?;
    if out.status.success() {
        return Ok(MergeOutcome::Merged {
            commit: head(worktree)?,
        });
    }
    let details = format!(
        "{} {}",
        String::from_utf8_lossy(&out.stdout).trim(),
        String::from_utf8_lossy(&out.stderr).trim()
    )
    .trim()
    .to_string();
    // A cherry-pick that stopped mid-sequence leaves CHERRY_PICK_HEAD; abort it so
    // the worktree is not left in a conflicted state (best-effort cleanup).
    let in_pick = git_at(worktree)
        .args(["rev-parse", "-q", "--verify", "CHERRY_PICK_HEAD"])
        .output()
        .is_ok_and(|o| o.status.success());
    if in_pick {
        let _ = git(worktree, &["cherry-pick", "--abort"]);
        Ok(MergeOutcome::Conflict { details })
    } else {
        Err(PipelineError::Git(format!(
            "git cherry-pick failed in {} (not a content conflict): {details}",
            worktree.display()
        )))
    }
}

/// The outcome of a merge attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeOutcome {
    /// The merge succeeded; carries the resulting commit oid.
    Merged { commit: String },
    /// The merge hit conflicts and was aborted (`git merge --abort`); no commit.
    Conflict { details: String },
}

/// Merge `branch` into the branch checked out in `worktree` with an explicit
/// merge commit (`--no-ff --no-edit`). On conflict the merge is aborted so the
/// worktree is left clean, and [`MergeOutcome::Conflict`] is returned rather
/// than an error — a conflict is a pipeline decision, not a git failure.
pub fn merge_no_ff(
    worktree: &Path,
    branch: &str,
    message: &str,
) -> Result<MergeOutcome, PipelineError> {
    let out = git_at(worktree)
        .args(["merge", "--no-ff", "--no-edit", "-m", message, branch])
        .output()
        .map_err(|e| {
            PipelineError::Git(format!(
                "could not run git merge in {}: {e}",
                worktree.display()
            ))
        })?;
    if out.status.success() {
        return Ok(MergeOutcome::Merged {
            commit: head(worktree)?,
        });
    }
    let details = format!(
        "{} {}",
        String::from_utf8_lossy(&out.stdout).trim(),
        String::from_utf8_lossy(&out.stderr).trim()
    )
    .trim()
    .to_string();
    // Distinguish a genuine content conflict (git entered a merge state with
    // `MERGE_HEAD`) from any other non-zero exit (missing identity, a rejecting
    // hook, disk full, a bad ref). Only the former is a `Conflict` outcome; the
    // rest are hard git errors — misreporting them as "conflict" would send the
    // caller down the resolve-and-retry path for an unrelated failure.
    let in_merge = git_at(worktree)
        .args(["rev-parse", "-q", "--verify", "MERGE_HEAD"])
        .output()
        .is_ok_and(|o| o.status.success());
    if in_merge {
        // Abort so the worktree is not left mid-merge (best-effort cleanup).
        let _ = git(worktree, &["merge", "--abort"]);
        Ok(MergeOutcome::Conflict { details })
    } else {
        Err(PipelineError::Git(format!(
            "git merge failed in {} (not a content conflict): {details}",
            worktree.display()
        )))
    }
}

/// Count of commits reachable from `branch` but not from `base` (`git rev-list
/// --count base..branch`). Zero means `branch` holds no work beyond `base` — the
/// source-relative check that decides whether an unmerged branch is safe to drop
/// (state-integrity invariant 5). Errors are treated as "has work" by the caller.
pub fn commits_ahead_of(dir: &Path, base: &str, branch: &str) -> Result<usize, PipelineError> {
    let out = git(dir, &["rev-list", "--count", &format!("{base}..{branch}")])?;
    out.trim()
        .parse::<usize>()
        .map_err(|e| PipelineError::Git(format!("could not parse rev-list count {out:?}: {e}")))
}

/// The filesystem path of the worktree that currently has `branch` checked out,
/// if any (parsed from `git worktree list --porcelain`). Used to decide where a
/// merge into `source_branch` can run: if it is checked out somewhere clean we
/// merge there, else we materialize a throwaway worktree.
pub fn worktree_for_branch(dir: &Path, branch: &str) -> Result<Option<PathBuf>, PipelineError> {
    let listing = git(dir, &["worktree", "list", "--porcelain"])?;
    let want = format!("refs/heads/{branch}");
    let mut current_path: Option<PathBuf> = None;
    for line in listing.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            // A new worktree block: reset state so a detached-HEAD worktree
            // (which omits the `branch` line) cannot carry the previous block's
            // path into this one.
            current_path = Some(PathBuf::from(p));
        } else if let Some(b) = line.strip_prefix("branch ") {
            if b == want {
                return Ok(current_path);
            }
        }
    }
    Ok(None)
}
