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

/// Files changed between `base` and `tip` (`git diff --name-only`). Uses `-z`
/// (NUL-delimited) so paths with spaces/tabs/newlines survive intact — the same
/// discipline the harness adapter uses. A git failure is propagated, never
/// masked as an empty diff (an empty list would silently pass the file-scope
/// gate).
pub fn changed_files(repo: &Path, base: &str, tip: &str) -> Result<Vec<PathBuf>, FloorError> {
    let out = Command::new(git_bin())
        .arg("-C")
        .arg(repo)
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
pub fn file_at_ref(repo: &Path, r#ref: &str, path: &Path) -> Result<Option<String>, FloorError> {
    let spec = format!("{}:{}", r#ref, path.display());
    let out = Command::new(git_bin())
        .arg("-C")
        .arg(repo)
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
        // git show exits non-zero when the path doesn't exist at the ref. We
        // treat "path not in tree" as absent rather than an error; distinguish
        // it from a bad ref by checking the message.
        let stderr = String::from_utf8_lossy(&out.stderr);
        if stderr.contains("does not exist")
            || stderr.contains("exists on disk, but not in")
            || stderr.contains("path '")
        {
            Ok(None)
        } else {
            Err(FloorError::Git {
                message: format!("git show {spec} failed: {}", stderr.trim()),
            })
        }
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
