//! Result types shared by every `doctor` check.
//!
//! A check produces zero or more [`CheckResult`]s, each with a stable
//! `id` (so an agent can pin which checks it expects to see), a
//! [`CheckStatus`], a one-line `message` naming the observed state, and —
//! for `WARN`/`FAIL` — a human-readable `fix_suggestion`. When a fix is
//! in the §18 "safe subset" the result also carries a [`FixAction`] the
//! `--fix` applier can execute autonomously; everything else only reports
//! the suggestion text and requires manual action.

use std::path::PathBuf;

use serde::Serialize;

/// Per-check outcome (AGENTS-AI-FIRST-CLI §18).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    Ok,
    Warn,
    Fail,
}

/// A single diagnostic finding. `safe_fix` is the internal handle the
/// `--fix` applier branches on; it is never serialized. `details` carries
/// optional check-specific structured observations, so agents do not need to
/// scrape values such as commit hashes from `message`.
#[derive(Debug, Clone, Serialize)]
pub struct CheckResult {
    pub id: String,
    pub status: CheckStatus,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix_suggestion: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
    #[serde(skip)]
    pub safe_fix: Option<FixAction>,
}

impl CheckResult {
    pub fn ok(id: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            status: CheckStatus::Ok,
            message: message.into(),
            fix_suggestion: None,
            details: None,
            safe_fix: None,
        }
    }

    pub fn warn(
        id: impl Into<String>,
        message: impl Into<String>,
        fix_suggestion: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            status: CheckStatus::Warn,
            message: message.into(),
            fix_suggestion: Some(fix_suggestion.into()),
            details: None,
            safe_fix: None,
        }
    }

    pub fn fail(
        id: impl Into<String>,
        message: impl Into<String>,
        fix_suggestion: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            status: CheckStatus::Fail,
            message: message.into(),
            fix_suggestion: Some(fix_suggestion.into()),
            details: None,
            safe_fix: None,
        }
    }

    /// Attach stable, check-specific machine-readable observations.
    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }

    /// Attach a safe, auto-applicable fix (the §18 `--fix` subset).
    pub fn with_safe_fix(mut self, fix: FixAction) -> Self {
        self.safe_fix = Some(fix);
        self
    }
}

/// The concrete, side-effecting action the `--fix` applier may run for a
/// finding. Only the §18 safe subset is representable here, and each
/// variant is *domain-specific* — there is deliberately no generic
/// "remove any file" so the safe subset cannot quietly grow into
/// something destructive:
///
/// - [`FixAction::InstallSkill`] re-installs a drifted companion skill
///   (`taskfleet skill install <name> --force`).
/// - [`FixAction::RemoveStaleSupervisorPid`] deletes a clearly-dead
///   supervisor PID file. It carries `observed_pid` so the applier can
///   re-validate (TOCTOU: a supervisor may have restarted between the
///   check and the apply) before removing.
///
/// Anything destructive of run data (e.g. repairing a corrupt
/// manifest.json) is deliberately *not* representable: it stays a
/// `fix_suggestion` string requiring manual action.
#[derive(Debug, Clone)]
pub enum FixAction {
    /// Re-install a drifted skill via `skill install <name> --force`.
    InstallSkill(String),
    /// Remove a >24h-dead supervisor PID file, re-validating that the
    /// file still holds `observed_pid` and that it is still dead.
    RemoveStaleSupervisorPid { path: PathBuf, observed_pid: u32 },
}

impl FixAction {
    /// Short, stable verb/noun describing the action — surfaced in the
    /// §11 dry-run plan and the apply report.
    pub fn describe(&self) -> (&'static str, &'static str, String) {
        match self {
            FixAction::InstallSkill(name) => ("install", "skill", name.clone()),
            FixAction::RemoveStaleSupervisorPid { path, .. } => {
                ("remove", "supervisor-pid", path.display().to_string())
            }
        }
    }
}

/// Tally of statuses across every check.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct Summary {
    pub ok: usize,
    pub warn: usize,
    pub fail: usize,
}

impl Summary {
    pub fn tally(results: &[CheckResult]) -> Self {
        let mut s = Summary::default();
        for r in results {
            match r.status {
                CheckStatus::Ok => s.ok += 1,
                CheckStatus::Warn => s.warn += 1,
                CheckStatus::Fail => s.fail += 1,
            }
        }
        s
    }

    /// §18 exit semantics: any `fail` → exit 1; otherwise exit 0.
    /// Warnings never flip the exit code.
    pub fn any_fail(&self) -> bool {
        self.fail > 0
    }
}
