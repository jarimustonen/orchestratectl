//! Thin git helpers the floor's capture layer needs (design.md §4 file-scope +
//! assertion-density signals). Impure by nature — they shell out — but kept
//! minimal and separate so the [`super::gates`] stay pure. Honours the
//! crate-wide `GIT_BIN` override (mirrors `harness::aider::git_bin` and
//! `supervise::cleanup::git_bin`), so tests can point at a fixture.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::FloorError;

/// The `git` binary, honouring the `GIT_BIN` override.
fn git_bin() -> String {
    std::env::var("GIT_BIN").unwrap_or_else(|_| "git".to_string())
}

/// A `git -C <repo>` command with a stabilized environment: `LC_ALL=C` so any
/// message we must read is locale-independent, and `core.quotePath=false` so
/// path output is not re-encoded. All floor git shell-outs go through here.
fn git_at(repo: &Path) -> Command {
    let mut cmd = Command::new(git_bin());
    cmd.arg("-C")
        .arg(repo)
        .args(["-c", "core.quotePath=false"])
        .env("LC_ALL", "C");
    cmd
}

/// Files changed between `base` and `tip` (`git diff --name-only`). Uses `-z`
/// (NUL-delimited) so paths with spaces/tabs/newlines survive intact — the same
/// discipline the harness adapter uses. A git failure is propagated, never
/// masked as an empty diff (an empty list would silently pass the file-scope
/// gate).
pub fn changed_files(repo: &Path, base: &str, tip: &str) -> Result<Vec<PathBuf>, FloorError> {
    let out = git_at(repo)
        .args(["diff", "--name-only", "-z", &format!("{base}..{tip}")])
        .output()
        .map_err(|e| FloorError::Git {
            message: format!("could not run git diff in {}: {e}", repo.display()),
        })?;
    if !out.status.success() {
        return Err(FloorError::Git {
            message: format!(
                "git diff {base}..{tip} failed in {}: {}",
                repo.display(),
                String::from_utf8_lossy(&out.stderr).trim()
            ),
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .split('\0')
        .filter(|l| !l.is_empty())
        .map(PathBuf::from)
        .collect())
}

/// The contents of `path` as of `r#ref` (`git show <ref>:<path>`), or `None`
/// when the path did not exist at that ref. A path absent at baseline is not an
/// error — it is a brand-new file with no baseline assertion count to regress
/// from. Any other git failure (bad ref, not a repo) is surfaced.
///
/// Existence is decided by **exit codes**, not by matching git's English
/// stderr (which is locale- and version-dependent): first the ref is verified
/// as a commit (`rev-parse --verify --quiet <ref>^{commit}`) — a failure there
/// is a real error — then the blob's presence is probed with
/// `cat-file -e <commit>:<path>`, whose non-zero exit means "absent" and only
/// that.
pub fn file_at_ref(repo: &Path, r#ref: &str, path: &Path) -> Result<Option<String>, FloorError> {
    // 1. Resolve the ref to a commit oid. A bad ref / non-repo is a hard error.
    let commit = {
        let rev = format!("{ref}^{{commit}}", r#ref = r#ref);
        let out = git_at(repo)
            .args(["rev-parse", "--verify", "--quiet", &rev])
            .output()
            .map_err(|e| FloorError::Git {
                message: format!("could not run git rev-parse in {}: {e}", repo.display()),
            })?;
        if !out.status.success() {
            let ref_label = r#ref;
            return Err(FloorError::Git {
                message: format!("ref {ref_label:?} does not resolve to a commit"),
            });
        }
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };

    let spec = format!("{commit}:{}", path.display());

    // 2. Probe existence by exit code (0 = present, non-zero = absent).
    let exists = git_at(repo)
        .args(["cat-file", "-e", &spec])
        .output()
        .map_err(|e| FloorError::Git {
            message: format!("could not run git cat-file in {}: {e}", repo.display()),
        })?
        .status
        .success();
    if !exists {
        return Ok(None);
    }

    // 3. Read the blob.
    let out = git_at(repo)
        .args(["show", &spec])
        .output()
        .map_err(|e| FloorError::Git {
            message: format!("could not run git show in {}: {e}", repo.display()),
        })?;
    if out.status.success() {
        // A tracked source file is valid UTF-8 for our purposes; lossy is safe
        // for an assertion *count*.
        Ok(Some(String::from_utf8_lossy(&out.stdout).into_owned()))
    } else {
        Err(FloorError::Git {
            message: format!(
                "git show {spec} failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;
    use tempfile::TempDir;

    /// A tiny git repo with two commits so diff/show have something to read.
    /// Serialized-friendly: uses `-c` config so it needs no global git identity.
    fn git_in(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args([
                "-c",
                "user.email=t@t",
                "-c",
                "user.name=t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .expect("git runs")
            .status
            .success();
        assert!(ok, "git {args:?} failed");
    }

    fn init_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        git_in(dir.path(), &["init", "-q", "-b", "main"]);
        dir
    }

    #[test]
    fn changed_files_lists_diff_between_commits() {
        let repo = init_repo();
        let p = repo.path();
        fs::write(p.join("a.rs"), "fn a() {}\n").unwrap();
        git_in(p, &["add", "."]);
        git_in(p, &["commit", "-qm", "base"]);
        let base = String::from_utf8_lossy(
            &Command::new("git")
                .arg("-C")
                .arg(p)
                .args(["rev-parse", "HEAD"])
                .output()
                .unwrap()
                .stdout,
        )
        .trim()
        .to_string();

        fs::write(p.join("a.rs"), "fn a() { let _ = 1; }\n").unwrap();
        fs::write(p.join("b.rs"), "fn b() {}\n").unwrap();
        git_in(p, &["add", "."]);
        git_in(p, &["commit", "-qm", "tip"]);

        let mut changed = changed_files(p, &base, "HEAD").unwrap();
        changed.sort();
        assert_eq!(changed, vec![PathBuf::from("a.rs"), PathBuf::from("b.rs")]);
    }

    #[test]
    fn file_at_ref_reads_old_content_and_reports_absent() {
        let repo = init_repo();
        let p = repo.path();
        fs::write(p.join("a.rs"), "assert!(x);\n").unwrap();
        git_in(p, &["add", "."]);
        git_in(p, &["commit", "-qm", "base"]);

        let content = file_at_ref(p, "HEAD", &PathBuf::from("a.rs")).unwrap();
        assert_eq!(content.as_deref(), Some("assert!(x);\n"));

        // A path that never existed at the ref is `None`, not an error.
        let absent = file_at_ref(p, "HEAD", &PathBuf::from("nope.rs")).unwrap();
        assert!(absent.is_none());
    }

    #[test]
    fn bad_ref_is_an_error() {
        let repo = init_repo();
        let p = repo.path();
        fs::write(p.join("a.rs"), "x\n").unwrap();
        git_in(p, &["add", "."]);
        git_in(p, &["commit", "-qm", "base"]);
        assert!(changed_files(p, "deadbeef", "HEAD").is_err());
    }
}
