//! `spinoff approve` — emit `spinoff.approved`, optionally materialize
//! an `issuectl` issue.
//!
//! Auto-materialization is best-effort: a missing `issuectl` binary is
//! intentionally silent (the tool is optional), an `issuectl` *failure*
//! surfaces as a warning entry in the success envelope so the caller
//! can decide whether to retry `issuectl new` themselves. The approval
//! is recorded either way.
//!
//! ## `--idempotency-key` scope
//!
//! The key dedupes the *local* `spinoff.approved` event-log write
//! only. It does NOT plumb through to `issuectl new`, so retrying an
//! approve with the same key after a partial failure can still create
//! a second issuectl issue. For retry-safe materialization, pass
//! `--issue-slug <slug>` — that skips the `issuectl` call entirely
//! and binds the approval to a known-existing issue.

use std::process::Command;

use serde::Serialize;
use serde_json::Value;

use octl_core::{
    append_and_apply_unlocked, read_manifest_opt, read_spinoff_opt, RunLock, SpinoffStatus,
};

use crate::error::CliError;
use crate::output::{self, OutputFormat, OutputSpec};
use crate::run::{from_core, kind_kebab, require_safe_id, run_paths};
use crate::spinoff::require_safe_slug;

pub struct Args<'a> {
    pub run_id: String,
    pub proposal_id: String,
    pub issue_slug: Option<String>,
    pub idempotency_key: Option<String>,
    pub dry_run: bool,
    pub spec: &'a OutputSpec,
    pub warnings: &'a [String],
}

#[derive(Serialize)]
struct ApprovePayload {
    run_id: String,
    proposal_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    issue_slug: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seq: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    idempotent_replay: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dry_run: Option<bool>,
}

/// What the locked critical section decided. Returning a typed enum
/// (rather than `Option<u64>`) so the caller never confuses a race-loss
/// for an idempotent replay of its own request — the loser must surface
/// the *persisted* state, not its locally-computed slug.
enum Outcome {
    Applied {
        seq: u64,
    },
    AlreadyApproved {
        issue_slug: Option<String>,
    },
    AlreadyRejected {
        reason: Option<String>,
    },
    /// The proposal vanished between the unlocked pre-check and the
    /// authoritative re-check inside the lock. Race against deletion
    /// or a corrupt projection.
    ProposalNotFound,
    RunNotFound,
}

pub fn run(args: Args<'_>) -> Result<(), CliError> {
    let run_id = require_safe_id(&args.run_id, "run-id")?;
    let proposal_id = require_safe_id(&args.proposal_id, "proposal-id")?;
    let issue_slug_arg = match args.issue_slug.as_deref() {
        Some(s) => Some(require_safe_slug(s, "issue-slug")?),
        None => None,
    };

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

    // Pre-lock fast-path responses. The lock-time recheck still
    // enforces these invariants authoritatively below.
    match proposal.status {
        SpinoffStatus::Approved => {
            if let Some(err) = mismatch_error(
                &proposal_id,
                issue_slug_arg.as_deref(),
                proposal.accepted_as_issue_slug.as_deref(),
            ) {
                return Err(err);
            }
            return emit_approved(
                &run_id,
                &proposal_id,
                proposal.accepted_as_issue_slug.clone(),
                None,
                Some(true),
                None,
                args.spec,
                args.warnings,
                &[],
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

    // Resolve the issue slug, possibly via issuectl. Calling issuectl
    // *before* the lock means two concurrent approvers can both create
    // external issues (one orphan, one canonical). Mitigations:
    //   - the lock-time recheck below returns the persisted slug, never
    //     the locally-computed one, when the loser detects a race;
    //   - the user can pass `--issue-slug` to skip materialization;
    //   - a follow-up issue (`spinoff-issuectl-materialization-arch`)
    //     redesigns this into a reserve→materialize→attach flow.
    let mut local_warnings: Vec<String> = Vec::new();
    let issue_slug: Option<String> = if let Some(s) = &issue_slug_arg {
        Some(s.clone())
    } else if args.dry_run {
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
        return emit_approved(
            &run_id,
            &proposal_id,
            issue_slug,
            None,
            None,
            Some(true),
            args.spec,
            args.warnings,
            &local_warnings,
        );
    }

    let mut data = serde_json::Map::new();
    data.insert("proposal_id".into(), Value::String(proposal_id.clone()));
    if let Some(s) = &issue_slug {
        data.insert("issue_slug".into(), Value::String(s.clone()));
    }
    let data = Value::Object(data);

    let outcome = RunLock::with_lock(&paths.lock(), || {
        // Re-validate run + proposal under the lock — the unlocked reads
        // above are advisory. A concurrent run-delete or
        // projection-corruption between unlocked check and lock is rare
        // but valid.
        if read_manifest_opt(&paths)?.is_none() {
            return Ok(Outcome::RunNotFound);
        }
        let cur = match read_spinoff_opt(&paths, &proposal_id)? {
            Some(p) => p,
            None => return Ok(Outcome::ProposalNotFound),
        };
        match cur.status {
            SpinoffStatus::Approved => {
                return Ok(Outcome::AlreadyApproved {
                    issue_slug: cur.accepted_as_issue_slug.clone(),
                });
            }
            SpinoffStatus::Rejected => {
                return Ok(Outcome::AlreadyRejected {
                    reason: cur.rejected_reason.clone(),
                });
            }
            SpinoffStatus::Proposed => {}
        }
        let seq = append_and_apply_unlocked(
            &paths,
            "spinoff.approved",
            None,
            args.idempotency_key.as_deref(),
            data,
        )?;
        Ok(Outcome::Applied { seq })
    })
    .map_err(from_core)?;

    match outcome {
        Outcome::Applied { seq } => emit_approved(
            &run_id,
            &proposal_id,
            issue_slug,
            Some(seq),
            None,
            None,
            args.spec,
            args.warnings,
            &local_warnings,
        ),
        Outcome::AlreadyApproved {
            issue_slug: persisted,
        } => {
            if let Some(err) =
                mismatch_error(&proposal_id, issue_slug_arg.as_deref(), persisted.as_deref())
            {
                return Err(err);
            }
            emit_approved(
                &run_id,
                &proposal_id,
                persisted,
                None,
                Some(true),
                None,
                args.spec,
                args.warnings,
                &local_warnings,
            )
        }
        Outcome::AlreadyRejected { reason } => Err(CliError::user(
            "proposal_already_rejected",
            format!(
                "proposal {proposal_id} was rejected by a concurrent caller \
                 (reason: {:?}); cannot approve",
                reason
            ),
        )
        .with_invalid_value(&proposal_id)),
        Outcome::ProposalNotFound => Err(CliError::user(
            "proposal_not_found",
            format!("proposal {proposal_id} disappeared from run {run_id}"),
        )
        .with_invalid_value(&proposal_id)),
        Outcome::RunNotFound => Err(CliError::user(
            "run_not_found",
            format!("run {run_id} disappeared"),
        )
        .with_invalid_value(&run_id)),
    }
}

/// If the proposal is already approved and the caller provided an
/// `--issue-slug` that does not match the recorded slug, return a
/// `proposal_already_approved` error. Silent ignores here would let
/// the caller believe their slug was attached when it wasn't.
fn mismatch_error(
    proposal_id: &str,
    requested: Option<&str>,
    recorded: Option<&str>,
) -> Option<CliError> {
    let requested = requested?;
    if recorded == Some(requested) {
        return None;
    }
    let recorded_repr = recorded.unwrap_or("<none>");
    Some(
        CliError::user(
            "proposal_already_approved",
            format!(
                "proposal {proposal_id} is already approved with issue-slug \
                 {recorded_repr:?}; cannot re-approve with a different slug \
                 {requested:?}"
            ),
        )
        .with_invalid_value(requested)
        .with_expected(Value::String(recorded_repr.to_string())),
    )
}

/// Try to materialize an issue via `issuectl new`. Returns:
///
/// - `Ok(Some(slug))` on success.
/// - `Ok(None)` if `issuectl` is not on PATH — silent because
///   `issuectl` is intentionally optional.
/// - `Err(warning)` if `issuectl` was found but failed; the caller
///   attaches the message to the response `warnings` array.
fn materialize_via_issuectl(
    title: &str,
    kind: &str,
    rationale: Option<&str>,
) -> Result<Option<String>, String> {
    let description = rationale.map(str::to_string).unwrap_or_else(|| {
        format!("Auto-materialized spin-off ({kind}) approved via orchestratectl.")
    });
    let mut cmd = Command::new("issuectl");
    // `--` terminates clap option parsing so an LLM-generated title
    // beginning with `--` doesn't get reinterpreted as an issuectl
    // flag. issuectl supports the standard `--` sentinel.
    cmd.args(["--json", "new", "--type", "feature"]);
    cmd.args(["--title", title, "--description", &description]);

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

#[allow(clippy::too_many_arguments)]
fn emit_approved(
    run_id: &str,
    proposal_id: &str,
    issue_slug: Option<String>,
    seq: Option<u64>,
    idempotent_replay: Option<bool>,
    dry_run: Option<bool>,
    spec: &OutputSpec,
    warnings: &[String],
    local_warnings: &[String],
) -> Result<(), CliError> {
    let payload = ApprovePayload {
        run_id: run_id.to_string(),
        proposal_id: proposal_id.to_string(),
        issue_slug: issue_slug.clone(),
        seq,
        idempotent_replay,
        dry_run,
    };
    match spec.format {
        OutputFormat::Json | OutputFormat::Jsonl => {
            let merged: Vec<String> = warnings
                .iter()
                .cloned()
                .chain(local_warnings.iter().cloned())
                .collect();
            output::emit_envelope(&payload, spec, &merged)?;
        }
        OutputFormat::Text => {
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
            output::emit_text_warnings(local_warnings);
            output::emit_text_warnings(warnings);
        }
    }
    Ok(())
}
