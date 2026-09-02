//! `schema.runs.<id>` — every run's `manifest.json` deserializes through
//! the `taskfleet_core` schema types at a supported `schema_version`
//! (AGENTS-AI-FIRST-CLI §18 "schema validation").
//!
//! One finding per run (never collapsed into a single summary check), so
//! an agent can pin exactly which run is broken. A half-initialized run
//! directory with no manifest yet is skipped silently — it mirrors
//! `run list`'s tolerance for in-flight creation and is not a schema
//! fault.

use taskfleet_core::{read_manifest_opt, RunPaths};

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
        if !ent.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        if let Some(name) = ent.file_name().to_str() {
            run_ids.push(name.to_string());
        }
    }
    // Deterministic order so the stream is stable across runs.
    run_ids.sort();

    for run_id in &run_ids {
        let id = format!("schema.runs.{run_id}");
        let paths = match RunPaths::new(runs_dir.join(run_id), run_id.clone()) {
            Ok(p) => p,
            // A directory under runs/ whose name isn't a valid run id is an
            // anomaly the doctor must surface, not hide: a failed migration,
            // manual damage, or a foreign dir dropped in the runs tree.
            Err(e) => {
                out.push(CheckResult::warn(
                    id,
                    format!("runs/{run_id} is not a valid run directory: {e}"),
                    format!(
                        "remove runs/{run_id} if it is not a run, or rename it to a valid run id"
                    ),
                ));
                continue;
            }
        };
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
