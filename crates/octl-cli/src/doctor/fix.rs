//! `--fix` applier: run the checks, then apply the §18 safe subset of
//! `fix_suggestion`s.
//!
//! Only findings carrying a [`FixAction`] are
//! touched — that is the deliberately small, non-destructive subset
//! (drifted-skill re-install, stale supervisor-PID removal). Everything
//! else stays advisory.
//!
//! `--fix --dry-run` emits the §11 planning envelope (`would: [...]`) and
//! changes nothing; a real `--fix` executes each action and returns a
//! per-action outcome the caller renders alongside the check results.

use std::process::{Command, Stdio};

use serde::Serialize;

use super::check::{CheckResult, FixAction};
use crate::supervise::pid_file;

/// One entry in the `--fix --dry-run` plan (§11 `would` array).
#[derive(Debug, Serialize)]
pub struct PlannedFix {
    pub check_id: String,
    pub action: &'static str,
    pub resource: &'static str,
    pub target: String,
}

/// Outcome of applying one safe fix during a real `--fix` run.
#[derive(Debug, Serialize)]
pub struct AppliedFix {
    pub check_id: String,
    pub action: &'static str,
    pub resource: &'static str,
    pub target: String,
    pub applied: bool,
    pub message: String,
}

/// Build the dry-run plan: every finding with a safe fix, in check order.
pub fn plan(results: &[CheckResult]) -> Vec<PlannedFix> {
    results
        .iter()
        .filter_map(|r| {
            let fix = r.safe_fix.as_ref()?;
            let (action, resource, target) = fix.describe();
            Some(PlannedFix {
                check_id: r.id.clone(),
                action,
                resource,
                target,
            })
        })
        .collect()
}

/// Apply every safe fix and report per-action outcomes. Failures are
/// recorded (`applied: false`) rather than aborting the batch — a fix
/// that cannot run should not mask the ones that can.
pub fn apply(results: &[CheckResult]) -> Vec<AppliedFix> {
    results
        .iter()
        .filter_map(|r| r.safe_fix.as_ref().map(|f| (r.id.clone(), f)))
        .map(|(check_id, fix)| {
            let (action, resource, target) = fix.describe();
            let (applied, message) = execute(fix);
            AppliedFix {
                check_id,
                action,
                resource,
                target,
                applied,
                message,
            }
        })
        .collect()
}

/// Execute a single safe action. Returns `(applied, message)`.
fn execute(fix: &FixAction) -> (bool, String) {
    match fix {
        FixAction::InstallSkill(name) => install_skill(name),
        FixAction::RemoveStaleSupervisorPid { path, observed_pid } => {
            remove_stale_supervisor_pid(path, *observed_pid)
        }
    }
}

/// Remove a stale supervisor PID file, re-validating immediately before
/// the unlink to close the TOCTOU window between the check and the apply.
/// A supervisor may have restarted in that gap and rewritten the file
/// with a fresh, *live* PID; removing that would detach a healthy
/// supervisor. So we refuse unless the file still holds exactly the PID
/// we observed and that PID is still dead.
fn remove_stale_supervisor_pid(path: &std::path::Path, observed_pid: u32) -> (bool, String) {
    match pid_file::read_pid(path) {
        None => (
            false,
            format!(
                "{} no longer holds a PID; refusing to remove",
                path.display()
            ),
        ),
        Some(current) if current != observed_pid => (
            false,
            format!(
                "{} changed (pid {current} != observed {observed_pid}); refusing to remove",
                path.display()
            ),
        ),
        Some(current) if pid_file::pid_alive(current) => (
            false,
            format!(
                "pid {current} is now alive; refusing to remove {}",
                path.display()
            ),
        ),
        Some(_) => match std::fs::remove_file(path) {
            Ok(()) => (
                true,
                format!("removed stale supervisor pid file {}", path.display()),
            ),
            Err(e) => (false, format!("could not remove {}: {e}", path.display())),
        },
    }
}

/// Re-install a drifted skill by re-invoking this binary's own `skill
/// install <name> --force`. Shelling out (rather than calling the install
/// fn directly) keeps the install's own success/error envelope off
/// doctor's stdout — we only care about the exit status here.
fn install_skill(name: &str) -> (bool, String) {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            return (
                false,
                format!("cannot locate self to re-install {name}: {e}"),
            )
        }
    };
    // Detach stdin: this is a non-interactive repair path and must never
    // block on a prompt. (`skill install` is already non-interactive, but
    // closing stdin makes the no-hang guarantee structural.)
    let output = Command::new(exe)
        .args(["skill", "install", name, "--force", "--output", "json"])
        .stdin(Stdio::null())
        .output();
    match output {
        Ok(out) if out.status.success() => (true, format!("re-installed skill {name}")),
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            (
                false,
                format!(
                    "skill install {name} failed (exit {:?}): {}",
                    out.status.code(),
                    stderr.trim()
                ),
            )
        }
        Err(e) => (false, format!("could not spawn skill install {name}: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn pid_file(content: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempdir().unwrap();
        let p = dir.path().join("supervisor.pid");
        std::fs::write(&p, content).unwrap();
        (dir, p)
    }

    #[test]
    fn removes_pid_file_that_is_still_dead() {
        // i32::MAX is effectively never a live PID.
        let (_d, p) = pid_file("2147483647");
        let (applied, msg) = remove_stale_supervisor_pid(&p, 2_147_483_647);
        assert!(applied, "should remove a still-dead pid: {msg}");
        assert!(!p.exists(), "file must be gone after removal");
    }

    #[test]
    fn refuses_when_pid_revived_under_same_value() {
        // Simulate a supervisor that restarted and re-claimed the same
        // recorded PID: the applier observed this PID, but it is now alive.
        let own = std::process::id();
        let (_d, p) = pid_file(&own.to_string());
        let (applied, msg) = remove_stale_supervisor_pid(&p, own);
        assert!(!applied, "must not remove a now-live supervisor pid");
        assert!(msg.contains("alive"), "msg: {msg}");
        assert!(p.exists(), "file must be preserved");
    }

    #[test]
    fn refuses_when_pid_changed_since_check() {
        // The file was rewritten with a different (live) PID between the
        // check and the apply — refuse rather than delete the new marker.
        let own = std::process::id();
        let (_d, p) = pid_file(&own.to_string());
        let (applied, msg) = remove_stale_supervisor_pid(&p, 999_999);
        assert!(!applied);
        assert!(msg.contains("changed"), "msg: {msg}");
        assert!(p.exists());
    }

    #[test]
    fn refuses_when_file_disappeared() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("supervisor.pid");
        let (applied, msg) = remove_stale_supervisor_pid(&p, 2_147_483_647);
        assert!(!applied);
        assert!(msg.contains("no longer holds a PID"), "msg: {msg}");
    }
}
