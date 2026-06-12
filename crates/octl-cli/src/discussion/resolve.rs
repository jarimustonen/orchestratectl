//! `discussion resolve` — domain verb (design.md §2.4).
//!
//! Emits a `discussion.resolved` event under the run's `flock` and updates
//! `discussions/<id>.json` via the reducer in the same locked window.
//!
//! Idempotency rules:
//! - Already-resolved with the **same** `--choice`: return success no-op.
//! - Already-resolved with a **different** `--choice`: exit 1 with
//!   `discussion_already_resolved` and include the existing resolution
//!   in the error envelope `details`.
//! - `--idempotency-key` retries with mismatching `--choice`/`--note`
//!   produce `idempotency_conflict`, mirroring `event create`.

use serde::Serialize;
use serde_json::{json, Value};

use octl_core::{ensure_root, read_discussion_opt, DiscussionStatus, RunLock};

use crate::error::CliError;
use crate::output;
use crate::run::{from_core, require_nonempty, require_safe_id, run_paths};

pub struct Args<'a> {
    pub run_id: String,
    pub discussion_id: String,
    pub choice: String,
    pub note: Option<String>,
    pub idempotency_key: Option<String>,
    pub dry_run: bool,
    pub json: bool,
    pub warnings: &'a [String],
}

#[derive(Serialize)]
struct ResolvePayload<'a> {
    run_id: &'a str,
    discussion_id: &'a str,
    choice: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seq: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    no_op: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    idempotent_replay: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dry_run: Option<bool>,
    projections: Vec<String>,
}

pub fn run(args: Args<'_>) -> Result<(), CliError> {
    let run_id = require_safe_id(&args.run_id, "run-id")?;
    let discussion_id = require_safe_id(&args.discussion_id, "discussion-id")?;
    let choice = require_nonempty(&args.choice, "choice")?;
    let note = match args.note.as_deref() {
        Some(n) => Some(require_nonempty(n, "note")?),
        None => None,
    };

    let root = crate::home::root_dir()?;
    let paths = run_paths(&root, &run_id);

    if !paths.root.is_dir() {
        return Err(
            CliError::user("run_not_found", format!("no run with id {run_id}"))
                .with_invalid_value(&run_id),
        );
    }

    // Pre-flock probe: provide a clear `discussion_not_found` early so
    // callers do not need to grep stderr to distinguish it from
    // `run_not_found`. The authoritative check happens again under the
    // lock below — between this read and the locked critical section a
    // concurrent writer could in principle create or delete the
    // discussion file.
    if read_discussion_opt(&paths, &discussion_id)
        .map_err(from_core)?
        .is_none()
    {
        return Err(CliError::user(
            "discussion_not_found",
            format!("no discussion {discussion_id} in run {run_id}"),
        )
        .with_invalid_value(&discussion_id));
    }

    let projections = vec![
        format!("discussions/{discussion_id}.json"),
        "manifest.json".to_string(),
    ];

    if args.dry_run {
        let payload = ResolvePayload {
            run_id: &run_id,
            discussion_id: &discussion_id,
            choice: &choice,
            note: note.as_deref(),
            seq: None,
            no_op: None,
            idempotent_replay: None,
            dry_run: Some(true),
            projections,
        };
        return emit(&payload, args.json, args.warnings);
    }

    ensure_root(&root).map_err(from_core)?;

    enum Outcome {
        Appended(u64),
        NoOp,
    }

    let outcome = RunLock::with_lock(&paths.lock(), || {
        // Authoritative state read inside the lock.
        let disc = match read_discussion_opt(&paths, &discussion_id)? {
            Some(d) => d,
            None => {
                // Discussion was removed between probe and lock. Surface
                // as a core::Error so the caller maps to system/IO; the
                // pre-flock probe already covers the common case.
                return Err(octl_core::Error::io(
                    paths.discussion(&discussion_id),
                    std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "discussion vanished between probe and lock",
                    ),
                ));
            }
        };

        if matches!(disc.status, DiscussionStatus::Resolved) {
            // Already resolved — same choice is a success no-op; a
            // different choice signals conflict, but we have to return
            // via the cli-error layer so it can synthesise the envelope.
            return Ok((Outcome::NoOp, Some(disc)));
        }

        let mut data = json!({
            "discussion_id": discussion_id,
            "resolution": choice,
        });
        if let Some(n) = &note {
            data["note"] = Value::String(n.clone());
        }

        let seq = octl_core::append_and_apply_unlocked(
            &paths,
            "discussion.resolved",
            None,
            args.idempotency_key.as_deref(),
            data,
        )?;
        Ok((Outcome::Appended(seq), None))
    })
    .map_err(from_core)?;

    match outcome {
        (Outcome::Appended(seq), _) => {
            let payload = ResolvePayload {
                run_id: &run_id,
                discussion_id: &discussion_id,
                choice: &choice,
                note: note.as_deref(),
                seq: Some(seq),
                no_op: None,
                idempotent_replay: None,
                dry_run: None,
                projections,
            };
            emit(&payload, args.json, args.warnings)
        }
        (Outcome::NoOp, Some(existing)) => {
            let existing_choice = existing.resolution.clone().unwrap_or_default();
            if existing_choice == choice {
                let payload = ResolvePayload {
                    run_id: &run_id,
                    discussion_id: &discussion_id,
                    choice: &choice,
                    note: existing.note.as_deref(),
                    seq: None,
                    no_op: Some(true),
                    idempotent_replay: None,
                    dry_run: None,
                    projections: Vec::new(),
                };
                emit(&payload, args.json, args.warnings)
            } else {
                let mut details = json!({
                    "existing_resolution": existing_choice,
                });
                if let Some(n) = &existing.note {
                    details["existing_note"] = Value::String(n.clone());
                }
                if let Some(t) = existing.resolved_at {
                    details["resolved_at"] = Value::String(t.to_rfc3339());
                }
                Err(CliError::user(
                    "discussion_already_resolved",
                    format!(
                        "discussion {discussion_id} is already resolved with a different choice"
                    ),
                )
                .with_invalid_value(&choice)
                .with_expected(details))
            }
        }
        (Outcome::NoOp, None) => unreachable!("NoOp branch always supplies the existing record"),
    }
}

fn emit(payload: &ResolvePayload<'_>, json: bool, warnings: &[String]) -> Result<(), CliError> {
    if json {
        output::emit_json(payload, warnings)
            .map_err(|e| CliError::system("internal_serialize", e.to_string()))?;
    } else {
        println!("run-id:        {}", payload.run_id);
        println!("discussion-id: {}", payload.discussion_id);
        println!("choice:        {}", payload.choice);
        if let Some(n) = payload.note {
            println!("note:          {}", n);
        }
        match payload.seq {
            Some(s) => println!("seq:           {}", s),
            None if payload.dry_run == Some(true) => {
                println!("seq:           (assigned on apply)");
                println!("note:          --dry-run (no filesystem changes)");
            }
            None => println!("seq:           (already resolved — no-op)"),
        }
        if !payload.projections.is_empty() {
            println!("projections:");
            for p in &payload.projections {
                println!("  - {}", p);
            }
        }
        for w in warnings {
            eprintln!("warning: {}", w);
        }
    }
    Ok(())
}
