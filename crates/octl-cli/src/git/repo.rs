//! The typed [`Git`] backend — one method per git subprocess the supervisor /
//! merge paths issue. See the [module docs](super) for provenance and the
//! fork-and-own drift policy.
//!
//! Every method is a mechanical extraction of a `Command::new(git)` call site
//! that used to live inline in [`crate::supervise::cleanup`]: same subcommand,
//! same args (including `-C <repo>`, `--end-of-options`, and the `--` before a
//! branch name), same stream redirection, and the same conservative-on-error
//! contract (a git error or non-zero exit reads as the *safe* verdict for that
//! call — "nothing unmerged" / "not merged" / "not clean" — never the reverse).
//! Do not soften these: they encode the branch-preservation and ancestry
//! invariants (root CLAUDE.md "State integrity invariants").

use std::process::{Command, Stdio};

use tracing::{info, warn};

/// Typed git backend, pinned to a specific binary. Construct with
/// [`Git::with_bin`], threading the caller's already-resolved binary name (the
/// supervisor resolves it once via [`crate::supervise::cleanup::git_bin`], which
/// honors the `GIT_BIN` test override) — the same seam tests inject a fake git
/// through.
pub struct Git {
    bin: String,
}

impl Git {
    /// A backend pinned to `bin` (the supervisor's already-resolved binary; also
    /// the seam tests inject a fake/real git through).
    pub fn with_bin(bin: impl Into<String>) -> Self {
        Self { bin: bin.into() }
    }

    /// A `Command` for `self.bin` rooted at `repo` (`git -C <repo> …`), matching
    /// how every extracted call site scoped its operation to a repo/worktree.
    fn at(&self, repo: &str) -> Command {
        let mut cmd = Command::new(&self.bin);
        cmd.arg("-C").arg(repo);
        cmd
    }

    /// `git -C <repo> rev-list --count <from>..<to>` → the number of commits
    /// reachable from `to` but not from `from`. `None` on a git error, a
    /// non-zero exit, or unparseable output, so a caller declines/decides
    /// conservatively rather than guess. This is the primitive behind both the
    /// source-relative unmerged-work check and the forward-advance check.
    pub fn rev_list_count(&self, repo: &str, from: &str, to: &str) -> Option<u64> {
        let out = self
            .at(repo)
            .args(["rev-list", "--count", &format!("{from}..{to}")])
            .stderr(Stdio::null())
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        String::from_utf8_lossy(&out.stdout)
            .trim()
            .parse::<u64>()
            .ok()
    }

    /// `git -C <repo> merge-base --is-ancestor <ancestor> <descendant>` — true
    /// when the command exits 0 (`ancestor` is reachable from `descendant`). A
    /// non-zero exit (not an ancestor, or exit 128 for an unknown ref) or a spawn
    /// failure → false. Parameters are named by their TOPOLOGICAL role: the
    /// "merged into source?" check passes `(branch, source)` while the
    /// "fast-forwards?" check passes `(source, branch)` — argument ORDER is what
    /// distinguishes them.
    pub fn is_ancestor(&self, repo: &str, ancestor: &str, descendant: &str) -> bool {
        self.at(repo)
            .args(["merge-base", "--is-ancestor", ancestor, descendant])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }

    /// `git -C <repo> merge-tree --write-tree <source> <branch>` — an in-memory
    /// three-way merge that writes only to the object store (never a worktree);
    /// exit 0 = clean, 1 = conflicts, ≥2 = git error. `true` only on a clean
    /// exit; any spawn failure or non-zero exit reads as "does not merge
    /// cleanly", so a transient git error never over-reports recoverability.
    /// Requires git ≥ 2.38 (`--write-tree`, Oct 2022).
    pub fn merge_tree_clean(&self, repo: &str, source: &str, branch: &str) -> bool {
        self.at(repo)
            .args(["merge-tree", "--write-tree", source, branch])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }

    /// `git -C <dir> status --porcelain` → true when the output is empty (no
    /// tracked, staged, or untracked changes). A `dir` whose `git status` cannot
    /// be read is conservatively treated as **dirty** (returns false), so a
    /// transient git failure never green-lights tearing a live tree down. The
    /// caller owns the "path is None / no longer exists → clean" guard.
    pub fn worktree_is_clean(&self, dir: &str) -> bool {
        match self
            .at(dir)
            .args(["status", "--porcelain"])
            .stderr(Stdio::null())
            .output()
        {
            Ok(out) if out.status.success() => out.stdout.iter().all(u8::is_ascii_whitespace),
            _ => false,
        }
    }

    /// The main worktree path for a linked worktree, read from the FIRST
    /// `worktree <path>` line of `git -C <dir> worktree list --porcelain` (git
    /// always lists the main worktree first). `None` if git is unavailable, the
    /// path is no longer a worktree, or the output is unparseable.
    pub fn main_worktree(&self, dir: &str) -> Option<String> {
        let out = self
            .at(dir)
            .args(["worktree", "list", "--porcelain"])
            .stderr(Stdio::null())
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .find_map(|l| l.strip_prefix("worktree ").map(|s| s.trim().to_string()))
            .filter(|s| !s.is_empty())
    }

    /// `git -C <repo> worktree remove --force <worktree_path>` — lenient.
    /// `--force` because on the paths that reach it the branch is either merged
    /// or provably has no unmerged work vs its source (unmerged work is preserved
    /// upstream in `cleanup_node`), so any untracked/modified scratch left behind
    /// is disposable; without it git refuses to remove a dirty tree and the
    /// cascade orphans the worktree AND branch (issue
    /// `supervisor-worktree-remove-no-force`). Returns `true` on success so the
    /// caller can distinguish an already-gone worktree from a genuine refusal.
    pub fn worktree_remove(&self, repo: &str, worktree_path: &str) -> bool {
        let mut cmd = self.at(repo);
        cmd.args(["worktree", "remove", "--force", worktree_path]);
        run_lenient(cmd, &format!("git worktree remove --force {worktree_path}"))
    }

    /// `git -C <repo> branch -{d|D} -- <branch>` — lenient. `force` selects the
    /// flag and is the defense-in-depth safety net against the silent data loss
    /// of issue `blocked-report-deletes-branch`:
    ///
    /// - `force == true` (a confirmed `run merge`, report carries
    ///   `via: "explicit-merge"`) → `-D`, force-delete. The merge is confirmed
    ///   and the branch may already be gone from the main worktree's vantage
    ///   point, so the force is safe and necessary.
    /// - `force == false` → `-d`, the LAST-resort backstop. The caller has
    ///   already preserved a branch with source-unmerged commits via its stronger
    ///   source-relative check, so a branch reaching here is expected clean; `-d`
    ///   still refuses a branch not merged into `HEAD`/upstream, catching the
    ///   residual case where the source check could not run. `-d`'s check is
    ///   ambient-`HEAD`-relative and weaker, which is why it is only the fallback.
    ///
    /// The branch name is passed after `--` so a name beginning with `-` can
    /// never be misparsed as a flag. Returns the captured failure detail (`Some`)
    /// when the delete did not succeed, `None` on success, so the caller can both
    /// record an audit event and surface the incompletion as a warning.
    pub fn branch_delete(&self, repo: &str, branch: &str, force: bool) -> Option<String> {
        let flag = if force { "-D" } else { "-d" };
        let mut cmd = self.at(repo);
        cmd.args(["branch", flag, "--", branch]);
        run_lenient_detail(cmd, &format!("git branch {flag} -- {branch}"))
    }
}

/// Run a best-effort git command, logging its outcome to both `tracing` and
/// stderr (captured to `supervisor.stderr.log`) so the teardown is auditable.
/// Returns `true` only when the command exited successfully; a non-zero exit or
/// spawn error is logged at `warn`, swallowed, and reported as `false` — cleanup
/// is best-effort by contract, but the boolean lets a caller fall back rather
/// than leak. Mirrors the tmux-side lenient runner in [`crate::multiplexer`];
/// git ops live here so the git module owns its own execution.
fn run_lenient(cmd: Command, label: &str) -> bool {
    run_lenient_detail(cmd, label).is_none()
}

/// Like [`run_lenient`] but returns the captured failure detail on a non-zero
/// exit or spawn error (`None` on success), so a caller can record it in an
/// audit event (e.g. `cleanup.branch_remove_failed`). Logging is identical.
fn run_lenient_detail(mut cmd: Command, label: &str) -> Option<String> {
    cmd.stdout(Stdio::null()).stderr(Stdio::piped());
    match cmd.output() {
        Ok(out) if out.status.success() => {
            info!(target: "orchestratectl::supervise", step = label, "cleanup step ok");
            eprintln!("supervisor cleanup: {label}: ok");
            None
        }
        Ok(out) => {
            let detail = String::from_utf8_lossy(&out.stderr).trim().to_string();
            warn!(
                target: "orchestratectl::supervise",
                step = label,
                code = out.status.code(),
                detail = %detail,
                "cleanup step non-zero (treated as already-done/refused; continuing)"
            );
            eprintln!("supervisor cleanup: {label}: non-zero exit (continuing): {detail}");
            Some(detail)
        }
        Err(e) => {
            warn!(
                target: "orchestratectl::supervise",
                step = label,
                error = %e,
                "cleanup step could not spawn (continuing)"
            );
            eprintln!("supervisor cleanup: {label}: spawn failed (continuing): {e}");
            Some(format!("spawn failed: {e}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;
    use tempfile::TempDir;

    /// Run real `git <args>` in `cwd`, asserting success. Sets up fixture repos
    /// for the tests below against a real git binary.
    fn git(cwd: &std::path::Path, args: &[&str]) {
        let ok = Command::new("git")
            .current_dir(cwd)
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?} failed in {cwd:?}");
    }

    /// A real repo with one commit on `main` and a linked worktree on `wt/foo`,
    /// returning `(repo, worktree)`.
    fn init_repo_with_worktree(tmp: &TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.email", "t@example.com"]);
        git(&repo, &["config", "user.name", "t"]);
        std::fs::write(repo.join("README"), "x").unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-qm", "init"]);
        let wt = tmp.path().join("wt");
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "wt/foo",
                wt.to_str().unwrap(),
            ],
        );
        (repo, wt)
    }

    fn branch_exists(repo: &std::path::Path, branch: &str) -> bool {
        Command::new("git")
            .current_dir(repo)
            .args(["rev-parse", "--verify", "--quiet", branch])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success()
    }

    /// Write an executable fake `git` that always exits non-zero, to exercise the
    /// conservative-on-error branches without a real repo.
    fn failing_git(dir: &std::path::Path) -> String {
        let p = dir.join("fake-git.sh");
        std::fs::write(&p, "#!/bin/sh\nexit 3\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p.to_str().unwrap().to_string()
    }

    #[test]
    fn rev_list_count_counts_commits_ahead() {
        let tmp = TempDir::new().unwrap();
        let (repo, wt) = init_repo_with_worktree(&tmp);
        let g = Git::with_bin("git");
        let repo_s = repo.to_str().unwrap();
        // wt/foo == main → 0 ahead of main.
        assert_eq!(g.rev_list_count(repo_s, "main", "wt/foo"), Some(0));
        // Add a commit on wt/foo → 1 ahead of main.
        std::fs::write(wt.join("f"), "y").unwrap();
        git(&wt, &["add", "-A"]);
        git(&wt, &["commit", "-qm", "work"]);
        assert_eq!(g.rev_list_count(repo_s, "main", "wt/foo"), Some(1));
    }

    #[test]
    fn rev_list_count_none_on_git_error() {
        let tmp = TempDir::new().unwrap();
        let g = Git::with_bin(failing_git(tmp.path()));
        assert_eq!(g.rev_list_count("/nonexistent", "main", "wt/foo"), None);
    }

    #[test]
    fn is_ancestor_reflects_topology() {
        let tmp = TempDir::new().unwrap();
        let (repo, wt) = init_repo_with_worktree(&tmp);
        let g = Git::with_bin("git");
        let repo_s = repo.to_str().unwrap();
        // Fresh branch == main: each is an ancestor of the other.
        assert!(g.is_ancestor(repo_s, "main", "wt/foo"));
        assert!(g.is_ancestor(repo_s, "wt/foo", "main"));
        // Advance wt/foo: main is still an ancestor of wt/foo, but not vice versa.
        std::fs::write(wt.join("f"), "y").unwrap();
        git(&wt, &["add", "-A"]);
        git(&wt, &["commit", "-qm", "work"]);
        assert!(g.is_ancestor(repo_s, "main", "wt/foo"));
        assert!(!g.is_ancestor(repo_s, "wt/foo", "main"));
    }

    #[test]
    fn is_ancestor_false_on_unknown_ref() {
        let tmp = TempDir::new().unwrap();
        let (repo, _wt) = init_repo_with_worktree(&tmp);
        let g = Git::with_bin("git");
        assert!(!g.is_ancestor(repo.to_str().unwrap(), "main", "does/not/exist"));
    }

    #[test]
    fn worktree_is_clean_tracks_dirtiness() {
        let tmp = TempDir::new().unwrap();
        let (_repo, wt) = init_repo_with_worktree(&tmp);
        let g = Git::with_bin("git");
        assert!(g.worktree_is_clean(wt.to_str().unwrap()));
        std::fs::write(wt.join("scratch"), "dirt").unwrap();
        assert!(!g.worktree_is_clean(wt.to_str().unwrap()));
    }

    #[test]
    fn worktree_is_clean_false_on_git_error() {
        // A non-repo directory: `git status` fails → conservatively dirty.
        let tmp = TempDir::new().unwrap();
        let g = Git::with_bin("git");
        assert!(!g.worktree_is_clean(tmp.path().to_str().unwrap()));
    }

    #[test]
    fn main_worktree_resolves_from_linked() {
        let tmp = TempDir::new().unwrap();
        let (repo, wt) = init_repo_with_worktree(&tmp);
        let g = Git::with_bin("git");
        let main = g.main_worktree(wt.to_str().unwrap()).unwrap();
        assert_eq!(
            std::fs::canonicalize(&main).unwrap(),
            std::fs::canonicalize(&repo).unwrap()
        );
    }

    #[test]
    fn worktree_remove_removes_and_is_lenient() {
        let tmp = TempDir::new().unwrap();
        let (repo, wt) = init_repo_with_worktree(&tmp);
        let g = Git::with_bin("git");
        let repo_s = repo.to_str().unwrap();
        assert!(g.worktree_remove(repo_s, wt.to_str().unwrap()));
        assert!(!wt.exists());
        // Second remove of an already-gone worktree is a lenient no-op (false).
        assert!(!g.worktree_remove(repo_s, wt.to_str().unwrap()));
    }

    /// The crux invariant: `-d` (force == false) REFUSES an unmerged branch,
    /// `-D` (force == true) force-deletes it.
    #[test]
    fn branch_delete_d_refuses_unmerged_but_big_d_forces() {
        let tmp = TempDir::new().unwrap();
        let (repo, wt) = init_repo_with_worktree(&tmp);
        let g = Git::with_bin("git");
        let repo_s = repo.to_str().unwrap();
        // Give wt/foo an unmerged commit, then detach the worktree so the branch
        // is deletable-in-principle from the main repo.
        std::fs::write(wt.join("f"), "y").unwrap();
        git(&wt, &["add", "-A"]);
        git(&wt, &["commit", "-qm", "unmerged work"]);
        assert!(g.worktree_remove(repo_s, wt.to_str().unwrap()));
        assert!(branch_exists(&repo, "wt/foo"));

        // `-d` refuses: the branch has commits not merged into HEAD (main).
        let detail = g.branch_delete(repo_s, "wt/foo", false);
        assert!(detail.is_some(), "-d must refuse an unmerged branch");
        assert!(
            branch_exists(&repo, "wt/foo"),
            "branch preserved by -d refusal"
        );

        // `-D` force-deletes it.
        assert_eq!(g.branch_delete(repo_s, "wt/foo", true), None);
        assert!(!branch_exists(&repo, "wt/foo"));
    }

    #[test]
    fn branch_delete_d_succeeds_on_merged_branch() {
        let tmp = TempDir::new().unwrap();
        let (repo, wt) = init_repo_with_worktree(&tmp);
        let g = Git::with_bin("git");
        let repo_s = repo.to_str().unwrap();
        // wt/foo is at main (fully merged); detach worktree then `-d` succeeds.
        assert!(g.worktree_remove(repo_s, wt.to_str().unwrap()));
        assert_eq!(g.branch_delete(repo_s, "wt/foo", false), None);
        assert!(!branch_exists(&repo, "wt/foo"));
    }
}
