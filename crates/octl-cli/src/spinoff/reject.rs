//! `spinoff reject` — emit `spinoff.rejected`.

use serde::Serialize;
use serde_json::Value;

use octl_core::{
    append_and_apply_unlocked, read_manifest_opt, read_spinoff_opt, ProposalId, RunLock,
    SpinoffStatus,
};

use crate::error::CliError;
use crate::output::{self, OutputFormat, OutputSpec};
use crate::run::{from_core, parse_proposal_id, run_paths};
use crate::spinoff::validate_reason_like;

pub struct Args<'a> {
    pub run_id: String,
    pub proposal_id: String,
    pub reason: Option<String>,
    pub idempotency_key: Option<String>,
    pub dry_run: bool,
    pub spec: &'a OutputSpec,
    pub warnings: &'a [String],
}

#[derive(Serialize)]
struct RejectPayload {
    run_id: String,
    proposal_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seq: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    idempotent_replay: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dry_run: Option<bool>,
}

/// Authoritative outcome of the locked critical section. Returning a
/// typed enum keeps the caller from treating a concurrent
/// approve/reject win as a successful idempotent replay.
enum Outcome {
    Applied { seq: u64 },
    AlreadyRejected { reason: Option<String> },
    AlreadyApproved,
    ProposalNotFound,
    RunNotFound,
}

pub fn run(args: Args<'_>) -> Result<(), CliError> {
    let run_id = args.run_id.clone();
    let proposal_id = parse_proposal_id(&args.proposal_id)?;
    let reason = match args.reason.as_deref() {
        Some(r) => Some(validate_reason_like(r, "reason")?),
        None => None,
    };

    let root = crate::home::root_dir()?;
    let paths = run_paths(&root, &run_id)?;

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
            .with_invalid_value(proposal_id.as_str()));
        }
    };

    // Pre-lock fast-path. The lock-time recheck below re-evaluates
    // these conditions authoritatively.
    match proposal.status {
        SpinoffStatus::Rejected => {
            if proposal.rejected_reason.as_deref() == reason.as_deref() {
                return emit_rejected(
                    &run_id,
                    &proposal_id,
                    proposal.rejected_reason.clone(),
                    None,
                    Some(true),
                    None,
                    args.spec,
                    args.warnings,
                );
            }
            return Err(CliError::user(
                "proposal_already_rejected",
                format!(
                    "proposal {proposal_id} was already rejected with a different reason \
                     (prior: {:?}, current: {:?})",
                    proposal.rejected_reason, reason
                ),
            )
            .with_invalid_value(proposal_id.as_str()));
        }
        SpinoffStatus::Approved => {
            return Err(CliError::user(
                "proposal_already_approved",
                format!("proposal {proposal_id} was already approved; cannot reject"),
            )
            .with_invalid_value(proposal_id.as_str()));
        }
        SpinoffStatus::Proposed => {}
    }

    if args.dry_run {
        return emit_rejected(
            &run_id,
            &proposal_id,
            reason,
            None,
            None,
            Some(true),
            args.spec,
            args.warnings,
        );
    }

    let mut data = serde_json::Map::new();
    data.insert("proposal_id".into(), Value::String(proposal_id.to_string()));
    if let Some(r) = &reason {
        data.insert("reason".into(), Value::String(r.clone()));
    }
    let data = Value::Object(data);

    let reason_for_lock = reason.clone();
    let outcome = RunLock::with_lock(&paths.lock(), || {
        if read_manifest_opt(&paths)?.is_none() {
            return Ok(Outcome::RunNotFound);
        }
        let cur = match read_spinoff_opt(&paths, &proposal_id)? {
            Some(p) => p,
            None => return Ok(Outcome::ProposalNotFound),
        };
        match cur.status {
            SpinoffStatus::Rejected => {
                return Ok(Outcome::AlreadyRejected {
                    reason: cur.rejected_reason.clone(),
                });
            }
            SpinoffStatus::Approved => {
                return Ok(Outcome::AlreadyApproved);
            }
            SpinoffStatus::Proposed => {}
        }
        let _ = &reason_for_lock; // captured for reason-mismatch reporting if needed
        let seq = append_and_apply_unlocked(
            &paths,
            "spinoff.rejected",
            None,
            args.idempotency_key.as_deref(),
            data,
        )?;
        Ok(Outcome::Applied { seq })
    })
    .map_err(from_core)?;

    match outcome {
        Outcome::Applied { seq } => emit_rejected(
            &run_id,
            &proposal_id,
            reason,
            Some(seq),
            None,
            None,
            args.spec,
            args.warnings,
        ),
        Outcome::AlreadyRejected { reason: persisted } => {
            if persisted.as_deref() == reason.as_deref() {
                emit_rejected(
                    &run_id,
                    &proposal_id,
                    persisted,
                    None,
                    Some(true),
                    None,
                    args.spec,
                    args.warnings,
                )
            } else {
                Err(CliError::user(
                    "proposal_already_rejected",
                    format!(
                        "proposal {proposal_id} was rejected by a concurrent caller with a \
                         different reason (prior: {persisted:?}, current: {reason:?})"
                    ),
                )
                .with_invalid_value(proposal_id.as_str()))
            }
        }
        Outcome::AlreadyApproved => Err(CliError::user(
            "proposal_already_approved",
            format!("proposal {proposal_id} was approved by a concurrent caller; cannot reject"),
        )
        .with_invalid_value(proposal_id.as_str())),
        Outcome::ProposalNotFound => Err(CliError::user(
            "proposal_not_found",
            format!("proposal {proposal_id} disappeared from run {run_id}"),
        )
        .with_invalid_value(proposal_id.as_str())),
        Outcome::RunNotFound => Err(CliError::user(
            "run_not_found",
            format!("run {run_id} disappeared"),
        )
        .with_invalid_value(&run_id)),
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_rejected(
    run_id: &str,
    proposal_id: &ProposalId,
    reason: Option<String>,
    seq: Option<u64>,
    idempotent_replay: Option<bool>,
    dry_run: Option<bool>,
    spec: &OutputSpec,
    warnings: &[String],
) -> Result<(), CliError> {
    let payload = RejectPayload {
        run_id: run_id.to_string(),
        proposal_id: proposal_id.to_string(),
        reason,
        seq,
        idempotent_replay,
        dry_run,
    };
    match spec.format {
        OutputFormat::Json | OutputFormat::Jsonl => {
            output::emit_envelope(&payload, spec, warnings)?;
        }
        OutputFormat::Text => {
            println!("run-id:      {}", payload.run_id);
            println!("proposal-id: {}", payload.proposal_id);
            if let Some(r) = &payload.reason {
                println!("reason:      {r}");
            }
            match payload.seq {
                Some(s) => println!("seq:         {s}"),
                None if payload.dry_run == Some(true) => {
                    println!("seq:         (assigned on apply)");
                }
                None => println!("seq:         (no-op; already rejected)"),
            }
            if payload.dry_run == Some(true) {
                println!("note:        --dry-run (no filesystem changes)");
            }
            if payload.idempotent_replay == Some(true) {
                println!("note:        idempotent replay (already rejected)");
            }
            output::emit_text_warnings(warnings);
        }
    }
    Ok(())
}
