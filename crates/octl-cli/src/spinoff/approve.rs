//! `spinoff approve` — emit `spinoff.approved`, optionally materialize
//! an `issuectl` issue.
//!
//! V10 design intent: a missing `issuectl` binary is not fatal — the
//! approval still records, only the auto-materialization path is
//! skipped, and the caller sees a `warnings[]` entry so they can run
//! `issuectl new` themselves.

use std::process::Command;

use serde::Serialize;
use serde_json::{json, Value};

use octl_core::{
    append_and_apply_unlocked, read_manifest_opt, read_spinoff_opt, RunLock, SpinoffStatus,
};

use crate::error::CliError;
use crate::output;
use crate::run::{from_core, kind_kebab, require_safe_id, run_paths};

pub struct Args<'a> {
    pub run_id: String,
    pub proposal_id: String,
    pub issue_slug: Option<String>,
    pub idempotency_key: Option<String>,
    pub dry_run: bool,
    pub json: bool,
    pub warnings: &'a [String],
}

#[derive(Serialize)]
struct ApprovePayload<'a> {
    run_id: &'a str,
    proposal_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    issue_slug: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seq: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    idempotent_replay: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dry_run: Option<bool>,
    /// Per-call warnings (issuectl-missing, issuectl-failed). Merges
    /// with the global envelope `warnings` array in `output::emit_json`.
    warnings: Vec<String>,
}

pub fn run(args: Args<'_>) -> Result<(), CliError> {
    let run_id = require_safe_id(&args.run_id, "run-id")?;
    let proposal_id = require_safe_id(&args.proposal_id, "proposal-id")?;

    let root = crate::home::root_dir()?;
    let paths = run_paths(&root, &run_id);

    if read_manifest_opt(&paths).map_err(from_core)?.is_none() {
        return Err(
            CliError::user("run_not_found", format!("no run with id {run_id}"))
                .with_invalid_value(&run_id),
        );
    }

    let proposal = match read_spinoff_opt(&paths, &proposal_id).map_err(from_core)? {
        Some(p) => p,
        None => {
            return Err(CliError::user(
                "proposal_not_found",
                format!("no proposal with id {proposal_id} in run {run_id}"),
            )
            .with_invalid_value(&proposal_id));
        }
    };

    match proposal.status {
        SpinoffStatus::Approved => {
            // Idempotent re-approve. Return the prior decision.
            return emit(
                ApprovePayload {
                    run_id: &run_id,
                    proposal_id: &proposal_id,
                    issue_slug: proposal.accepted_as_issue_slug.clone(),
                    seq: None,
                    idempotent_replay: Some(true),
                    dry_run: None,
                    warnings: Vec::new(),
                },
                args.json,
                args.warnings,
            );
        }
        SpinoffStatus::Rejected => {
            return Err(CliError::user(
                "proposal_already_rejected",
                format!("proposal {proposal_id} was already rejected; cannot approve"),
            )
            .with_invalid_value(&proposal_id));
        }
        SpinoffStatus::Proposed => {}
    }

    // Resolve the issue slug, possibly via issuectl. Run this before
    // taking the flock — issuectl operates on a different repo and may
    // be slow; we don't want to block the run's event log on it.
    let mut local_warnings: Vec<String> = Vec::new();
    let issue_slug: Option<String> = if let Some(s) = args.issue_slug.as_deref() {
        Some(s.to_string())
    } else if args.dry_run {
        // Planning envelope: don't fork issuectl on dry-run.
        None
    } else {
        match materialize_via_issuectl(
            &proposal.proposed_title,
            kind_kebab(proposal.proposed_kind),
            proposal.rationale.as_deref(),
        ) {
            Ok(Some(slug)) => Some(slug),
            Ok(None) => None,
            Err(w) => {
                local_warnings.push(w);
                None
            }
        }
    };

    if args.dry_run {
        return emit(
            ApprovePayload {
                run_id: &run_id,
                proposal_id: &proposal_id,
                issue_slug,
                seq: None,
                idempotent_replay: None,
                dry_run: Some(true),
                warnings: local_warnings,
            },
            args.json,
            args.warnings,
        );
    }

    let mut data = serde_json::Map::new();
    data.insert("proposal_id".into(), Value::String(proposal_id.clone()));
    if let Some(s) = &issue_slug {
        data.insert("issue_slug".into(), Value::String(s.clone()));
    }
    let data = Value::Object(data);

    let seq = RunLock::with_lock(&paths.lock(), || {
        // Re-check status under the lock: a concurrent approve could
        // have raced us between the unlocked read above and now.
        if let Some(cur) = read_spinoff_opt(&paths, &proposal_id)? {
            if matches!(
                cur.status,
                SpinoffStatus::Approved | SpinoffStatus::Rejected
            ) {
                return Ok(None);
            }
        }
        let seq = append_and_apply_unlocked(
            &paths,
            "spinoff.approved",
            None,
            args.idempotency_key.as_deref(),
            data,
        )?;
        Ok(Some(seq))
    })
    .map_err(from_core)?;

    let (seq, replayed) = match seq {
        Some(s) => (Some(s), false),
        None => (None, true),
    };

    emit(
        ApprovePayload {
            run_id: &run_id,
            proposal_id: &proposal_id,
            issue_slug,
            seq,
            idempotent_replay: if replayed { Some(true) } else { None },
            dry_run: None,
            warnings: local_warnings,
        },
        args.json,
        args.warnings,
    )
}

/// Try to materialize an issue via `issuectl new`. Returns:
///
/// - `Ok(Some(slug))` on success.
/// - `Ok(None)` if `issuectl` is not on PATH (no warning — design
///   intent is that `issuectl` is optional).
/// - `Err(warning)` if `issuectl` is on PATH but failed; the caller
///   should attach the message to the response `warnings` array.
fn materialize_via_issuectl(
    title: &str,
    kind: &str,
    rationale: Option<&str>,
) -> Result<Option<String>, String> {
    let mut cmd = Command::new("issuectl");
    cmd.args(["--json", "new", "--type", "feature", "--title", title]);
    let description = rationale.map(str::to_string).unwrap_or_else(|| {
        format!("Auto-materialized spin-off ({kind}) approved via orchestratectl.")
    });
    cmd.args(["--description", &description]);

    let output = match cmd.output() {
        Ok(o) => o,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("issuectl spawn failed: {e}")),
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "issuectl exited {:?}: {}",
            output.status.code(),
            stderr.trim()
        ));
    }
    let v: Value = serde_json::from_slice(&output.stdout).map_err(|e| {
        format!(
            "issuectl returned non-JSON: {e}; raw: {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })?;
    match v.get("slug").and_then(Value::as_str) {
        Some(s) => Ok(Some(s.to_string())),
        None => Err(format!("issuectl JSON missing `slug` field: {v}")),
    }
}

fn emit(payload: ApprovePayload<'_>, json: bool, warnings: &[String]) -> Result<(), CliError> {
    if json {
        // Merge per-call warnings into the envelope. `output::emit_json`
        // owns the envelope's `warnings` field, so we hand it the union.
        let merged: Vec<String> = warnings
            .iter()
            .cloned()
            .chain(payload.warnings.iter().cloned())
            .collect();
        let body = json!({
            "run_id": payload.run_id,
            "proposal_id": payload.proposal_id,
            "issue_slug": payload.issue_slug,
            "seq": payload.seq,
            "idempotent_replay": payload.idempotent_replay,
            "dry_run": payload.dry_run,
            "warnings": payload.warnings,
        });
        output::emit_json(&body, &merged)
            .map_err(|e| CliError::system("internal_serialize", e.to_string()))?;
    } else {
        println!("run-id:      {}", payload.run_id);
        println!("proposal-id: {}", payload.proposal_id);
        if let Some(s) = &payload.issue_slug {
            println!("issue-slug:  {}", s);
        }
        match payload.seq {
            Some(s) => println!("seq:         {}", s),
            None if payload.dry_run == Some(true) => {
                println!("seq:         (assigned on apply)")
            }
            None => println!("seq:         (no-op; already approved)"),
        }
        if payload.dry_run == Some(true) {
            println!("note:        --dry-run (no filesystem changes)");
        }
        if payload.idempotent_replay == Some(true) {
            println!("note:        idempotent replay (already approved)");
        }
        for w in &payload.warnings {
            eprintln!("warning: {}", w);
        }
        for w in warnings {
            eprintln!("warning: {}", w);
        }
    }
    Ok(())
}
