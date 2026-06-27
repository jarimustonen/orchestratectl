//! `schema.runs.<id>` — every run's `manifest.json` deserializes through
//! the `octl_core` schema types at a supported `schema_version`
//! (AGENTS-AI-FIRST-CLI §18 "schema validation").
//!
//! One finding per run (never collapsed into a single summary check), so
//! an agent can pin exactly which run is broken. A half-initialized run
//! directory with no manifest yet is skipped silently — it mirrors
//! `run list`'s tolerance for in-flight creation and is not a schema
//! fault.

use octl_core::{read_manifest_opt, RunPaths};

use crate::doctor::check::CheckResult;

use super::Ctx;

pub fn check(ctx: &Ctx) -> Vec<CheckResult> {
    let Some(root) = ctx.root.as_deref() else {
        // config.home already reported the unresolved-home failure; do not
        // double-report here.
        return Vec::new();
    };
    let runs_dir = root.join("runs");

    let entries = match std::fs::read_dir(&runs_dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return vec![CheckResult::ok(
                "schema.runs",
                "no runs directory (nothing to validate)",
            )];
        }
        Err(e) => {
            return vec![CheckResult::fail(
                "schema.runs",
                format!("cannot read {}: {}", runs_dir.display(), e),
                format!("check permissions on {}", runs_dir.display()),
            )];
        }
    };

    let mut out = Vec::new();
    let mut run_ids: Vec<String> = Vec::new();
    for ent in entries.flatten() {
        if !ent.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        if let Some(name) = ent.file_name().to_str() {
            run_ids.push(name.to_string());
        }
    }
    // Deterministic order so the stream is stable across runs.
    run_ids.sort();

    for run_id in &run_ids {
        let paths = RunPaths::new(runs_dir.join(run_id));
        let id = format!("schema.runs.{run_id}");
        match read_manifest_opt(&paths) {
            Ok(Some(_)) => out.push(CheckResult::ok(id, "manifest valid")),
            // No manifest yet: a run mid-creation, not a schema fault.
            Ok(None) => {}
            Err(e) => out.push(CheckResult::fail(
                id,
                format!("manifest invalid: {e}"),
                format!(
                    "orchestratectl run cancel {run_id} (or repair runs/{run_id}/manifest.json manually)"
                ),
            )),
        }
    }

    if out.is_empty() {
        out.push(CheckResult::ok(
            "schema.runs",
            "no run manifests to validate",
        ));
    }
    out
}
