//! `discussion resolve` — domain verb (design.md §2.4).
//!
//! Emits a `discussion.resolved` event under the run's `flock` and updates
//! `discussions/<id>.json` via the reducer in the same locked window.
//!
//! Outcome contract (single `outcome` discriminator in the success payload):
//! - `appended` — first resolution; new event written.
//! - `idempotent-replay` — `--idempotency-key` matched a prior event with
//!   identical payload; original `seq` returned.
//! - `no-op` — discussion already resolved with the same `--choice` (no
//!   key, or no prior key match).
//! - `dry-run` — preflight; the would-be outcome of the real call is
//!   reported in `would_be`.
//!
//! Conflict matrix:
//! - same `--idempotency-key`, different payload → `idempotency_conflict`.
//! - already-resolved, no key, different `--choice` → `discussion_already_resolved`.

use std::io::{BufRead, BufReader};
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use octl_core::{ensure_root, read_discussion_opt, Discussion, DiscussionStatus, RunLock};

use crate::error::CliError;
use crate::output::{self, OutputFormat, OutputSpec};
use crate::run::{from_core, parse_discussion_id, require_nonempty, run_paths};

/// Hard caps on free-form CLI strings written into the event log.
/// `event create --from-file` enforces a 1 MiB cap on its JSON payload;
/// matching bounds here prevent a misconfigured agent from bloating the
/// canonical log via the domain-verb path.
const MAX_CHOICE_BYTES: usize = 1024;
const MAX_NOTE_BYTES: usize = 16 * 1024;

pub struct Args<'a> {
    pub run_id: String,
    pub discussion_id: String,
    pub choice: String,
    pub note: Option<String>,
    pub idempotency_key: Option<String>,
    pub dry_run: bool,
    pub spec: &'a OutputSpec,
    pub warnings: &'a [String],
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "kebab-case")]
enum Outcome {
    Appended,
    IdempotentReplay,
    NoOp,
    DryRun,
}

#[derive(Serialize)]
struct ResolvePayload<'a> {
    run_id: &'a str,
    discussion_id: &'a str,
    node_id: &'a str,
    choice: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<&'a str>,
    outcome: Outcome,
    /// `seq` of the appended (or replayed) event; `None` for no-op and
    /// dry-run.
    #[serde(skip_serializing_if = "Option::is_none")]
    seq: Option<u64>,
    /// On `dry-run`, the outcome the real call would produce (`appended`,
    /// `no-op`, or `idempotent-replay`). Conflict cases still surface as
    /// errors even in dry-run, never as a success envelope.
    #[serde(skip_serializing_if = "Option::is_none")]
    would_be: Option<Outcome>,
    /// Files the reducer (would) touch. Empty on no-op since no
    /// projection changes.
    projections: Vec<String>,
}

pub fn run(args: Args<'_>) -> Result<(), CliError> {
    let run_id = args.run_id.clone();
    let discussion_id = parse_discussion_id(&args.discussion_id)?;
    let choice = require_nonempty(&args.choice, "choice")?;
    if choice.len() > MAX_CHOICE_BYTES {
        return Err(CliError::user(
            "invalid_value",
            format!(
                "--choice is {} bytes; max is {} bytes",
                choice.len(),
                MAX_CHOICE_BYTES
            ),
        )
        .with_invalid_value(format!("{} bytes", choice.len())));
    }
    let note = match args.note.as_deref() {
        Some(n) => {
            let n = require_nonempty(n, "note")?;
            if n.len() > MAX_NOTE_BYTES {
                return Err(CliError::user(
                    "invalid_value",
                    format!(
                        "--note is {} bytes; max is {} bytes",
                        n.len(),
                        MAX_NOTE_BYTES
                    ),
                )
                .with_invalid_value(format!("{} bytes", n.len())));
            }
            Some(n)
        }
        None => None,
    };

    let root = crate::home::root_dir()?;
    let paths = run_paths(&root, &run_id)?;

    if !paths.root.is_dir() {
        return Err(
            CliError::user("run_not_found", format!("no run with id {run_id}"))
                .with_invalid_value(&run_id),
        );
    }

    let projections_for = |id: &str| {
        vec![
            format!("discussions/{id}.json"),
            "manifest.json".to_string(),
        ]
    };

    ensure_root(&root).map_err(from_core)?;

    // All authoritative reads happen inside the lock so a concurrent
    // writer can't race the idempotency / domain-state decisions.
    enum LockResult {
        Missing,
        Conflict {
            existing: Discussion,
        },
        IdempotencyConflict {
            prior_seq: u64,
            prior_choice: Option<String>,
            prior_note: Option<String>,
            prior_discussion_id: Option<String>,
        },
        IdempotentReplay {
            seq: u64,
            existing: Discussion,
        },
        NoOp {
            existing: Discussion,
        },
        WouldAppend {
            existing: Discussion,
        },
        Appended {
            seq: u64,
            node_id: String,
        },
    }

    let result = RunLock::with_lock(&paths.lock(), || {
        // Idempotency-key scan must run BEFORE the domain status check so
        // that mismatched payloads return `idempotency_conflict`, not
        // `discussion_already_resolved`. A keyed replay then short-circuits
        // even if a concurrent writer raced ahead with the same key.
        if let Some(key) = args.idempotency_key.as_deref() {
            if let Some(prior) = find_prior_event(&paths.events(), "discussion.resolved", key)? {
                let prior_discussion_id = prior
                    .data
                    .get("discussion_id")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let prior_choice = prior
                    .data
                    .get("resolution")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let prior_note = prior
                    .data
                    .get("note")
                    .and_then(Value::as_str)
                    .map(str::to_string);

                let matches = prior_discussion_id.as_deref() == Some(discussion_id.as_str())
                    && prior_choice.as_deref() == Some(&choice)
                    && prior_note.as_deref() == note.as_deref();

                if !matches {
                    return Ok(LockResult::IdempotencyConflict {
                        prior_seq: prior.seq,
                        prior_choice,
                        prior_note,
                        prior_discussion_id,
                    });
                }

                // Idempotent replay — recover the projection state for
                // the success envelope.
                match read_discussion_opt(&paths, &discussion_id)? {
                    Some(d) => {
                        return Ok(LockResult::IdempotentReplay {
                            seq: prior.seq,
                            existing: d,
                        })
                    }
                    None => return Ok(LockResult::Missing),
                }
            }
        }

        let disc = match read_discussion_opt(&paths, &discussion_id)? {
            Some(d) => d,
            None => return Ok(LockResult::Missing),
        };

        if matches!(disc.status, DiscussionStatus::Resolved) {
            if disc.resolution.as_deref() == Some(&choice) {
                return Ok(LockResult::NoOp { existing: disc });
            }
            return Ok(LockResult::Conflict { existing: disc });
        }

        // Dry-run stops here — we know the operation would append, but
        // the locked critical section is the right place to confirm it.
        if args.dry_run {
            return Ok(LockResult::WouldAppend { existing: disc });
        }

        let mut data = json!({
            "discussion_id": discussion_id.as_str(),
            "resolution": choice,
        });
        if let Some(n) = &note {
            data["note"] = Value::String(n.clone());
        }

        let node_id = disc.node_id.clone();
        let seq = octl_core::append_and_apply_unlocked(
            &paths,
            "discussion.resolved",
            Some(node_id.as_str()),
            args.idempotency_key.as_deref(),
            data,
        )?;
        Ok(LockResult::Appended {
            seq,
            node_id: node_id.to_string(),
        })
    })
    .map_err(from_core)?;

    match result {
        LockResult::Missing => Err(CliError::user(
            "discussion_not_found",
            format!("no discussion {discussion_id} in run {run_id}"),
        )
        .with_invalid_value(discussion_id.as_str())),

        LockResult::IdempotencyConflict {
            prior_seq,
            prior_choice,
            prior_note,
            prior_discussion_id,
        } => {
            let mut details = json!({
                "prior_seq": prior_seq,
            });
            if let Some(id) = prior_discussion_id {
                details["prior_discussion_id"] = Value::String(id);
            }
            if let Some(c) = prior_choice {
                details["prior_resolution"] = Value::String(c);
            }
            if let Some(n) = prior_note {
                details["prior_note"] = Value::String(n);
            }
            Err(CliError::user(
                "idempotency_conflict",
                "idempotency-key was previously used with a different payload",
            )
            .with_invalid_value(args.idempotency_key.as_deref().unwrap_or(""))
            .with_expected(details))
        }

        LockResult::Conflict { existing } => {
            let existing_choice = existing.resolution.clone().unwrap_or_default();
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
                format!("discussion {discussion_id} is already resolved with a different choice"),
            )
            .with_invalid_value(&choice)
            .with_expected(details))
        }

        LockResult::IdempotentReplay { seq, existing } => {
            let payload = ResolvePayload {
                run_id: &run_id,
                discussion_id: discussion_id.as_str(),
                node_id: existing.node_id.as_str(),
                choice: &choice,
                note: note.as_deref(),
                outcome: Outcome::IdempotentReplay,
                seq: Some(seq),
                would_be: None,
                projections: Vec::new(),
            };
            emit(&payload, args.spec, args.warnings)
        }

        LockResult::NoOp { existing } => {
            let payload = ResolvePayload {
                run_id: &run_id,
                discussion_id: discussion_id.as_str(),
                node_id: existing.node_id.as_str(),
                choice: &choice,
                note: existing.note.as_deref(),
                outcome: Outcome::NoOp,
                seq: None,
                would_be: None,
                projections: Vec::new(),
            };
            emit(&payload, args.spec, args.warnings)
        }

        LockResult::WouldAppend { existing } => {
            let payload = ResolvePayload {
                run_id: &run_id,
                discussion_id: discussion_id.as_str(),
                node_id: existing.node_id.as_str(),
                choice: &choice,
                note: note.as_deref(),
                outcome: Outcome::DryRun,
                seq: None,
                would_be: Some(Outcome::Appended),
                projections: projections_for(discussion_id.as_str()),
            };
            emit(&payload, args.spec, args.warnings)
        }

        LockResult::Appended { seq, node_id } => {
            let payload = ResolvePayload {
                run_id: &run_id,
                discussion_id: discussion_id.as_str(),
                node_id: &node_id,
                choice: &choice,
                note: note.as_deref(),
                outcome: Outcome::Appended,
                seq: Some(seq),
                would_be: None,
                projections: projections_for(discussion_id.as_str()),
            };
            emit(&payload, args.spec, args.warnings)
        }
    }
}

/// Cached fields extracted from a prior `discussion.resolved` event so
/// the caller can decide replay vs. conflict.
struct PriorEvent {
    seq: u64,
    data: Value,
}

/// Stream-scan `events.jsonl` for an event with matching `kind` and
/// `idempotency_key`. Deserialises only the lookup fields up front to
/// avoid parsing the (potentially large) `data` payload on every line.
///
/// Caller must hold the run's `flock`. This is structurally the same
/// scan that `event create` performs; centralising both into
/// `octl_core::events` is tracked as a spin-off (see
/// `assessment-discussion-cli.md` F14).
fn find_prior_event(
    events_path: &Path,
    kind: &str,
    key: &str,
) -> octl_core::Result<Option<PriorEvent>> {
    let f = match std::fs::File::open(events_path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(octl_core::Error::io(events_path, e)),
    };
    let reader = BufReader::new(f);
    for line in reader.lines() {
        let line = line.map_err(|e| octl_core::Error::io(events_path, e))?;
        if line.is_empty() {
            continue;
        }
        let probe: ProbeFields = match serde_json::from_str(&line) {
            Ok(p) => p,
            Err(_) => continue,
        };
        if probe.kind != kind || probe.idempotency_key.as_deref() != Some(key) {
            continue;
        }
        let full: FullEventForReplay =
            serde_json::from_str(&line).map_err(|e| octl_core::Error::json(events_path, e))?;
        return Ok(Some(PriorEvent {
            seq: full.seq,
            data: full.data,
        }));
    }
    Ok(None)
}

#[derive(Deserialize)]
struct ProbeFields {
    kind: String,
    idempotency_key: Option<String>,
}

#[derive(Deserialize)]
struct FullEventForReplay {
    seq: u64,
    data: Value,
}

fn emit(
    payload: &ResolvePayload<'_>,
    spec: &OutputSpec,
    warnings: &[String],
) -> Result<(), CliError> {
    match spec.format {
        OutputFormat::Json | OutputFormat::Jsonl => {
            return output::emit_envelope(payload, spec, warnings);
        }
        OutputFormat::Text => {}
    }
    {
        println!("run-id:        {}", payload.run_id);
        println!("discussion-id: {}", payload.discussion_id);
        println!("node-id:       {}", payload.node_id);
        println!("choice:        {}", payload.choice);
        if let Some(n) = payload.note {
            println!("note:          {n}");
        }
        let outcome_label = match payload.outcome {
            Outcome::Appended => "appended",
            Outcome::IdempotentReplay => "idempotent-replay",
            Outcome::NoOp => "no-op (already resolved with same choice)",
            Outcome::DryRun => "dry-run (no filesystem changes)",
        };
        println!("outcome:       {outcome_label}");
        if let Some(s) = payload.seq {
            println!("seq:           {s}");
        }
        if let Some(w) = payload.would_be {
            let w_label = match w {
                Outcome::Appended => "appended",
                Outcome::IdempotentReplay => "idempotent-replay",
                Outcome::NoOp => "no-op",
                Outcome::DryRun => "dry-run",
            };
            println!("would-be:      {w_label}");
        }
        if !payload.projections.is_empty() {
            println!("projections:");
            for p in &payload.projections {
                println!("  - {p}");
            }
        }
        output::emit_text_warnings(warnings);
    }
    Ok(())
}
