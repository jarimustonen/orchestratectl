//! `--fix` applier: run the checks, then apply the §18 safe subset of
//! `fix_suggestion`s.
//!
//! Only findings carrying a [`FixAction`](super::check::FixAction) are
//! touched — that is the deliberately small, non-destructive subset
//! (drifted-skill re-install, stale supervisor-PID removal). Everything
//! else stays advisory.
//!
//! `--fix --dry-run` emits the §11 planning envelope (`would: [...]`) and
//! changes nothing; a real `--fix` executes each action and returns a
//! per-action outcome the caller renders alongside the check results.

use std::process::Command;

use serde::Serialize;

use super::check::{CheckResult, FixAction};

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
        FixAction::RemoveFile(path) => match std::fs::remove_file(path) {
            Ok(()) => (true, format!("removed {}", path.display())),
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
    let output = Command::new(exe)
        .args(["skill", "install", name, "--force", "--output", "json"])
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
