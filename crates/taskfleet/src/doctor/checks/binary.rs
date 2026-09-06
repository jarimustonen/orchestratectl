//! `binary.commit` — disclose the running binary's build commit and, when
//! invoked from a Taskfleet source checkout,
//! compare it with that checkout's `HEAD`.
//!
//! A mismatch is advisory: released binaries and branch work commonly differ
//! legitimately. Outside a Taskfleet checkout there is no meaningful
//! repository reference, so the check still reports the build commit but marks
//! the comparison non-applicable.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::json;

use crate::doctor::check::CheckResult;

use super::Ctx;

const BUILD_COMMIT: &str = env!("TASKFLEET_GIT_COMMIT");

pub fn check(_ctx: &Ctx) -> Vec<CheckResult> {
    vec![result_for(BUILD_COMMIT, repository_head())]
}

fn result_for(build_commit: &str, reference: RepoHead) -> CheckResult {
    match reference {
        RepoHead::Head(head) if !is_recorded_commit(build_commit) => CheckResult::ok(
            "binary.commit",
            format!(
                "running binary build commit unavailable ({build_commit}); repository comparison unavailable"
            ),
        )
        .with_details(json!({
            "binary_commit": build_commit,
            "repository_head": head,
            "comparison": "unavailable",
        })),
        RepoHead::Head(head) if head == build_commit => CheckResult::ok(
            "binary.commit",
            format!("running binary commit {build_commit} matches repository HEAD"),
        )
        .with_details(json!({
            "binary_commit": build_commit,
            "repository_head": head,
            "comparison": "match",
        })),
        RepoHead::Head(head) => CheckResult::warn(
            "binary.commit",
            format!(
                "running binary commit {build_commit} differs from repository HEAD {head}"
            ),
            "confirm that this is the intended binary for the current Taskfleet checkout",
        )
        .with_details(json!({
            "binary_commit": build_commit,
            "repository_head": head,
            "comparison": "mismatch",
        })),
        RepoHead::Unavailable => CheckResult::ok(
            "binary.commit",
            format!(
                "running binary commit {build_commit}; Taskfleet repository HEAD unavailable"
            ),
        )
        .with_details(json!({
            "binary_commit": build_commit,
            "repository_head": null,
            "comparison": "unavailable",
        })),
        RepoHead::NotApplicable => CheckResult::ok(
            "binary.commit",
            format!(
                "running binary commit {build_commit}; repository comparison not applicable"
            ),
        )
        .with_details(json!({
            "binary_commit": build_commit,
            "repository_head": null,
            "comparison": "not_applicable",
        })),
    }
}

fn is_recorded_commit(commit: &str) -> bool {
    commit.len() == 40 && commit.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[derive(Debug, PartialEq, Eq)]
enum RepoHead {
    Head(String),
    Unavailable,
    NotApplicable,
}

fn repository_head() -> RepoHead {
    let git = std::ffi::OsStr::new("git");
    let cwd = match std::env::current_dir() {
        Ok(cwd) => cwd,
        Err(_) => return RepoHead::Unavailable,
    };

    // Avoid calling git at all in the overwhelmingly common non-repository
    // context. A `.git` directory or worktree indirection file both qualify.
    if !cwd.ancestors().any(|path| path.join(".git").exists()) {
        return RepoHead::NotApplicable;
    }

    let top = match git_stdout(git, &cwd, &["rev-parse", "--show-toplevel"]) {
        Some(top) => PathBuf::from(top),
        None => return RepoHead::Unavailable,
    };
    if !is_taskfleet_checkout(&top) {
        return RepoHead::NotApplicable;
    }
    match git_stdout(git, &top, &["rev-parse", "HEAD"]) {
        Some(head) => RepoHead::Head(head),
        None => RepoHead::Unavailable,
    }
}

fn git_stdout(git: &std::ffi::OsStr, dir: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new(git)
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    let value = stdout.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

/// A remote URL is intentionally not required: forks and offline clones are
/// equally applicable. The expected root manifest and CLI package identity are
/// specific enough to avoid comparing against an unrelated repository.
fn is_taskfleet_checkout(root: &Path) -> bool {
    if !root.join("Cargo.toml").is_file() {
        return false;
    }
    manifest_package_is(&root.join("crates/taskfleet/Cargo.toml"), "taskfleet")
}

fn manifest_package_is(path: &Path, expected: &str) -> bool {
    let Ok(body) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(manifest) = body.parse::<toml::Value>() else {
        return false;
    };
    manifest
        .get("package")
        .and_then(|package| package.get("name"))
        .and_then(toml::Value::as_str)
        == Some(expected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctor::check::CheckStatus;

    #[test]
    fn unrecorded_build_commit_is_not_a_false_mismatch() {
        let result = result_for(
            "unknown",
            RepoHead::Head("1111111111111111111111111111111111111111".into()),
        );
        assert_eq!(result.status, CheckStatus::Ok);
        let details = result.details.unwrap();
        assert_eq!(details["comparison"], "unavailable");
        assert_eq!(
            details["repository_head"],
            "1111111111111111111111111111111111111111"
        );
    }

    #[test]
    fn this_checkout_is_recognised() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root");
        assert!(is_taskfleet_checkout(root), "{}", root.display());
    }
}
