//! `spinoff reject` — emit `spinoff.rejected`.

use serde::Serialize;
use serde_json::Value;

use octl_core::{
    append_and_apply_unlocked, read_manifest_opt, read_spinoff_opt, RunLock, SpinoffStatus,
};

use crate::error::CliError;
use crate::output;
use crate::run::{from_core, require_safe_id, run_paths};

pub struct Args<'a> {
    pub run_id: String,
    pub proposal_id: String,
    pub reason: Option<String>,
    pub idempotency_key: Option<String>,
    pub dry_run: bool,
    pub json: bool,
    pub warnings: &'a [String],
}

#[derive(Serialize)]
struct RejectPayload<'a> {
    run_id: &'a str,
    proposal_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seq: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    idempotent_replay: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dry_run: Option<bool>,
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
        SpinoffStatus::Rejected => {
            // Idempotent only if the reason matches.
            if proposal.rejected_reason.as_deref() == args.reason.as_deref() {
                return emit(
                    RejectPayload {
                        run_id: &run_id,
                        proposal_id: &proposal_id,
                        reason: proposal.rejected_reason.as_deref(),
                        seq: None,
                        idempotent_replay: Some(true),
                        dry_run: None,
                    },
                    args.json,
                    args.warnings,
                );
            }
            return Err(CliError::user(
                "proposal_already_rejected",
                format!(
                    "proposal {proposal_id} was already rejected with a different reason \
                     (prior: {:?}, current: {:?})",
                    proposal.rejected_reason, args.reason
                ),
            )
            .with_invalid_value(&proposal_id));
        }
        SpinoffStatus::Approved => {
            return Err(CliError::user(
                "proposal_already_approved",
                format!("proposal {proposal_id} was already approved; cannot reject"),
            )
            .with_invalid_value(&proposal_id));
        }
        SpinoffStatus::Proposed => {}
    }

    if args.dry_run {
        return emit(
            RejectPayload {
                run_id: &run_id,
                proposal_id: &proposal_id,
                reason: args.reason.as_deref(),
                seq: None,
                idempotent_replay: None,
                dry_run: Some(true),
            },
            args.json,
            args.warnings,
        );
    }

    let mut data = serde_json::Map::new();
    data.insert("proposal_id".into(), Value::String(proposal_id.clone()));
    if let Some(r) = args.reason.as_deref() {
        data.insert("reason".into(), Value::String(r.to_string()));
    }
    let data = Value::Object(data);

    let seq = RunLock::with_lock(&paths.lock(), || {
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
            "spinoff.rejected",
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
        RejectPayload {
            run_id: &run_id,
            proposal_id: &proposal_id,
            reason: args.reason.as_deref(),
            seq,
            idempotent_replay: if replayed { Some(true) } else { None },
            dry_run: None,
        },
        args.json,
        args.warnings,
    )
}

fn emit(payload: RejectPayload<'_>, json: bool, warnings: &[String]) -> Result<(), CliError> {
    if json {
        output::emit_json(&payload, warnings)
            .map_err(|e| CliError::system("internal_serialize", e.to_string()))?;
    } else {
        println!("run-id:      {}", payload.run_id);
        println!("proposal-id: {}", payload.proposal_id);
        if let Some(r) = payload.reason {
            println!("reason:      {}", r);
        }
        match payload.seq {
            Some(s) => println!("seq:         {}", s),
            None if payload.dry_run == Some(true) => {
                println!("seq:         (assigned on apply)")
            }
            None => println!("seq:         (no-op; already rejected)"),
        }
        if payload.dry_run == Some(true) {
            println!("note:        --dry-run (no filesystem changes)");
        }
        if payload.idempotent_replay == Some(true) {
            println!("note:        idempotent replay (already rejected)");
        }
        for w in warnings {
            eprintln!("warning: {}", w);
        }
    }
    Ok(())
}
