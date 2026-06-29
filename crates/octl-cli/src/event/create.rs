//! `event create` — sanctioned write path for arbitrary event kinds.
//!
//! Validates `--kind` against the closed MVP event-kind set (design.md
//! §1.4), enforces `--node-id` for kinds that reference a specific node,
//! reads the `data` payload from `--from-file`, then appends + applies the
//! event under the run's `flock`.
//!
//! `--idempotency-key` dedup goes through the centralized
//! `octl_core::append_and_apply_idempotent`: it scans the existing event log,
//! classifies the call as a fresh append / idempotent replay / conflicting
//! reuse, and (on a fresh key) appends — all in a single `flock` window so
//! concurrent retries can't race past each other. A keyless create is a plain
//! `append_and_apply_unlocked`.

use std::path::PathBuf;

use serde::Serialize;
use serde_json::{json, Value};

use octl_core::{ensure_root, AppendOutcome, Event, NodeId, RunLock, RunPaths};

use crate::error::CliError;
use crate::output::{self, OutputFormat, OutputSpec};
use crate::run::{
    from_core, parse_discussion_id, parse_node_id, parse_proposal_id, parse_run_id,
    require_nonempty, run_paths,
};

pub struct Args<'a> {
    pub run_id: String,
    pub kind: String,
    pub node_id: Option<String>,
    pub from_file: PathBuf,
    pub idempotency_key: Option<String>,
    pub dry_run: bool,
    pub spec: &'a OutputSpec,
    pub warnings: &'a [String],
}

#[derive(Serialize)]
struct CreatedPayload<'a> {
    run_id: &'a str,
    kind: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    node_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seq: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    idempotent_replay: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dry_run: Option<bool>,
    /// Projection files this event will / would touch, as the reducer reports
    /// them via [`octl_core::plan_projections`] — the single source of truth,
    /// not a CLI-side mirror. Run-root-relative for display.
    projections: Vec<String>,
}

/// Closed set of event kinds per design.md §1.4. `node.heartbeat` is
/// intentionally absent — design.md §7.5 names it "future opt-in" and
/// no reducer case exists yet. Adding a new kind here is intentional:
/// the reducer must learn it first.
///
/// `orchestrator.decision` and `discuss.critical` are append-only audit
/// records emitted by `/orchestrate` (its decision log and pakkopysäytys
/// mechanism). They carry no projection — the reducer folds them to a clean
/// no-op — so the canonical event log is their sole home. They are NOT
/// `node.report`, so the supervisor's terminal-cleanup roll-up ignores them.
const ALLOWED_KINDS: &[&str] = &[
    "run.created",
    "run.status",
    "node.created",
    "node.status",
    "node.report",
    "discussion.opened",
    "discussion.resolved",
    "spinoff.proposed",
    "spinoff.approved",
    "spinoff.rejected",
    "child.spawned",
    "supervisor.attached",
    "supervisor.cursor_advanced",
    "supervisor.exited",
    "supervisor.reattach-requested",
    "orchestrator.decision",
    "discuss.critical",
];

/// `run.created` is the bootstrap event owned by `orchestratectl run
/// create`. Routing it through the generic write path would let a caller
/// append a duplicate bootstrap record to an already-initialised run —
/// the reducer is idempotent against it, but the second record is pure
/// noise in the canonical log.
///
/// `node.report` is a §7.3-shaped domain verb owned by `orchestratectl
/// node report`. The generic write path doesn't run the §7.3 payload
/// validator, so allowing it here would let agents bypass schema
/// enforcement and put malformed terminal reports into the canonical
/// log. Routes to `node report` instead.
const FORBIDDEN_KINDS_FOR_EVENT_CREATE: &[&str] = &["run.created", "node.report"];

/// Upper bound on `--from-file` payload size. `node.report` is the
/// largest realistic event at ~10-50 KB (design.md §1.4); 1 MiB gives
/// a generous ceiling that still bounds CLI memory.
const MAX_FROM_FILE_BYTES: u64 = 1024 * 1024;

/// Kinds where the reducer requires a top-level `node_id` to be useful.
/// Without it, the append succeeds but the reducer silently no-ops.
fn requires_node_id(kind: &str) -> bool {
    matches!(
        kind,
        "node.created"
            | "node.status"
            | "node.report"
            | "child.spawned"
            | "supervisor.attached"
            | "supervisor.cursor_advanced"
    )
}

/// Kinds that may legitimately reference a node but don't require it
/// (e.g. `discussion.opened` accepts `node_id` either at the top level
/// or inside `data`).
fn allows_node_id(kind: &str) -> bool {
    requires_node_id(kind)
        || matches!(
            kind,
            "discussion.opened"
                | "discussion.resolved"
                | "spinoff.proposed"
                | "spinoff.approved"
                | "spinoff.rejected"
        )
}

/// The projection files this event will touch, asked of the reducer itself via
/// [`octl_core::plan_projections`] rather than re-enumerated here. Used by
/// `--dry-run` and the success envelope to give callers a visible trace of the
/// reducer side-effects that can never drift from what the reducer actually
/// writes (issue `projected-paths-into-reducer`).
///
/// Paths are returned run-root-relative (e.g. `nodes/n-0001.json`,
/// `manifest.json`) for display. The plan is read against current projection
/// state, so a state-dependent no-op (a settled node, an already-created
/// projection) correctly yields an empty list. A reducer-level rejection of a
/// malformed payload surfaces here as the same error the real apply would
/// raise.
fn projected_paths(
    paths: &RunPaths,
    kind: &str,
    node_id: Option<&NodeId>,
    data: &Value,
) -> Result<Vec<String>, CliError> {
    // `seq`/`ts`/`idempotency_key` do not affect which projection files an event
    // touches, so a synthetic envelope is sufficient to ask the reducer for the
    // plan; the real seq/ts are stamped by the append path when (and if) we write.
    let ev = Event {
        ts: chrono::Utc::now(),
        seq: 0,
        kind: kind.to_string(),
        run_id: paths.run_id.clone(),
        node_id: node_id.cloned(),
        idempotency_key: None,
        data: data.clone(),
    };
    let abs = octl_core::plan_projections(paths, &ev).map_err(from_core)?;
    Ok(abs
        .iter()
        .map(|p| {
            p.strip_prefix(&paths.root)
                .unwrap_or(p)
                .to_string_lossy()
                .into_owned()
        })
        .collect())
}

/// Validate every data-derived ID at the CLI boundary so the reducer
/// never sees `discussion_id: "../../etc"` and the projection write
/// path can't escape the run dir.
fn validate_data_ids(kind: &str, top_node_id: Option<&str>, data: &Value) -> Result<(), CliError> {
    // `data.node_id` may appear on any kind that `allows_node_id`. If
    // it's a string, sanitize it and reject any disagreement with the
    // top-level `--node-id`.
    if let Some(v) = data.get("node_id") {
        let s = v
            .as_str()
            .ok_or_else(|| CliError::user("invalid_data_id", "data.node_id must be a string"))?;
        parse_node_id(s)?;
        if let Some(top) = top_node_id {
            if top != s {
                return Err(CliError::user(
                    "node_id_mismatch",
                    format!("--node-id ({top}) does not match data.node_id ({s})"),
                ));
            }
        }
    }
    if matches!(kind, "discussion.opened" | "discussion.resolved") {
        if let Some(v) = data.get("discussion_id") {
            let s = v.as_str().ok_or_else(|| {
                CliError::user("invalid_data_id", "data.discussion_id must be a string")
            })?;
            parse_discussion_id(s)?;
        }
    }
    if matches!(
        kind,
        "spinoff.proposed" | "spinoff.approved" | "spinoff.rejected"
    ) {
        if let Some(v) = data.get("proposal_id") {
            let s = v.as_str().ok_or_else(|| {
                CliError::user("invalid_data_id", "data.proposal_id must be a string")
            })?;
            parse_proposal_id(s)?;
        }
    }
    if kind == "child.spawned" {
        if let Some(v) = data.get("child_run_id") {
            let s = v.as_str().ok_or_else(|| {
                CliError::user("invalid_data_id", "data.child_run_id must be a string")
            })?;
            parse_run_id(s)?;
        }
        if let Some(v) = data.get("child_node_id") {
            let s = v.as_str().ok_or_else(|| {
                CliError::user("invalid_data_id", "data.child_node_id must be a string")
            })?;
            parse_node_id(s)?;
        }
    }
    if kind == "supervisor.cursor_advanced" {
        if let Some(v) = data.get("child_run_id") {
            let s = v.as_str().ok_or_else(|| {
                CliError::user("invalid_data_id", "data.child_run_id must be a string")
            })?;
            parse_run_id(s)?;
        }
    }
    Ok(())
}

pub fn run(args: Args<'_>) -> Result<(), CliError> {
    let run_id = args.run_id.clone();

    if !ALLOWED_KINDS.contains(&args.kind.as_str()) {
        return Err(CliError::user(
            "unknown_event_kind",
            format!("unknown event kind: {}", args.kind),
        )
        .with_invalid_value(&args.kind)
        .with_expected(json!(ALLOWED_KINDS)));
    }
    let kind = args.kind.as_str();
    // Reject empty/whitespace-only idempotency keys, matching `node report`.
    // An empty key would otherwise collapse every "no real key" call into one
    // dedup slot once `append_and_apply_event`'s folded scan runs.
    let idempotency_key = match args.idempotency_key.as_deref() {
        Some(k) => Some(require_nonempty(k, "idempotency-key")?),
        None => None,
    };
    if FORBIDDEN_KINDS_FOR_EVENT_CREATE.contains(&kind) {
        return Err(CliError::user(
            "kind_not_routable",
            format!(
                "`{kind}` is a bootstrap event and is owned by `orchestratectl run create`; \
                 it is not accepted via `event create`"
            ),
        )
        .with_invalid_value(kind));
    }

    // Reject `--node-id` for kinds that can't reference one — silently
    // accepting it would let callers paper over a typo (writing
    // `--kind run.status --node-id n-0001` and then wondering why the
    // node didn't change).
    if args.node_id.is_some() && !allows_node_id(kind) {
        let offending = args.node_id.as_deref().unwrap_or("");
        return Err(CliError::user(
            "unexpected_flag",
            format!("--node-id is not accepted for kind `{kind}`"),
        )
        .with_invalid_value(offending)
        .with_expected(json!({"kind": kind})));
    }

    let node_id = match args.node_id.as_deref() {
        Some(v) => Some(parse_node_id(v)?.to_string()),
        None => None,
    };

    if requires_node_id(kind) && node_id.is_none() {
        return Err(CliError::user(
            "missing_required_flag",
            format!("--node-id is required for kind `{kind}`"),
        )
        .with_expected(json!({"flag": "--node-id"})));
    }

    // Cap `--from-file` size before reading. A misconfigured caller
    // pointing this at `/dev/zero` or a multi-gig file would otherwise
    // OOM the CLI; this is the sanctioned bash write path so the bound
    // is the caller's defense too.
    let meta = std::fs::metadata(&args.from_file).map_err(|e| {
        CliError::user(
            "from_file_unreadable",
            format!("stat {}: {}", args.from_file.display(), e),
        )
        .with_invalid_value(args.from_file.display().to_string())
    })?;
    if !meta.is_file() {
        return Err(CliError::user(
            "from_file_unreadable",
            format!("{} is not a regular file", args.from_file.display()),
        )
        .with_invalid_value(args.from_file.display().to_string()));
    }
    if meta.len() > MAX_FROM_FILE_BYTES {
        return Err(CliError::user(
            "from_file_too_large",
            format!(
                "--from-file is {} bytes; max is {} bytes",
                meta.len(),
                MAX_FROM_FILE_BYTES
            ),
        )
        .with_invalid_value(args.from_file.display().to_string()));
    }

    // Read + parse the data payload before doing anything filesystem-y;
    // a malformed file should fail fast with a clear error envelope.
    let data_bytes = std::fs::read(&args.from_file).map_err(|e| {
        CliError::user(
            "from_file_unreadable",
            format!("read {}: {}", args.from_file.display(), e),
        )
        .with_invalid_value(args.from_file.display().to_string())
    })?;
    let data: Value = serde_json::from_slice(&data_bytes).map_err(|e| {
        CliError::user(
            "from_file_invalid_json",
            format!("parse {}: {}", args.from_file.display(), e),
        )
        .with_invalid_value(args.from_file.display().to_string())
    })?;
    if !data.is_object() {
        return Err(CliError::user(
            "from_file_invalid_json",
            "--from-file must contain a JSON object",
        )
        .with_invalid_value(args.from_file.display().to_string()));
    }

    validate_data_ids(kind, node_id.as_deref(), &data)?;

    let root = crate::home::root_dir()?;
    let paths = run_paths(&root, &run_id)?;

    // The run dir must already exist — `event create` is a write path
    // for an existing run; it does NOT bootstrap one. `is_dir()` over
    // plain `exists()` so a stray file at `<root>/runs/<id>` produces
    // the same clear `run_not_found` envelope rather than a later
    // obscure I/O failure.
    if !paths.root.is_dir() {
        return Err(
            CliError::user("run_not_found", format!("no run with id {run_id}"))
                .with_invalid_value(&run_id),
        );
    }

    // `node_id` was validated above; re-parse the canonical string into the
    // typed envelope id the reducer plan and the append APIs both take.
    let envelope_node = node_id.as_deref().map(parse_node_id).transpose()?;

    let projections = projected_paths(&paths, kind, envelope_node.as_ref(), &data)?;

    if args.dry_run {
        let payload = CreatedPayload {
            run_id: &run_id,
            kind,
            node_id: node_id.as_deref(),
            seq: None,
            idempotent_replay: None,
            dry_run: Some(true),
            projections,
        };
        return emit(&payload, args.spec, args.warnings);
    }

    ensure_root(&root).map_err(from_core)?;

    // Keyed creates route through the centralized idempotency primitive, which
    // folds the log scan, conflict detection, and append into one held lock so a
    // concurrent retry can't see "no prior event" and double-append. A keyless
    // create is a plain append under the same lock discipline.
    let (seq, idempotent_replay) = if let Some(key) = idempotency_key.as_deref() {
        let outcome = RunLock::with_lock(&paths, |lock| {
            octl_core::append_and_apply_idempotent(
                &paths,
                lock,
                kind,
                envelope_node.as_ref(),
                key,
                |_seq| Ok(data.clone()),
            )
        })
        .map_err(from_core)?;
        match outcome {
            AppendOutcome::Appended { seq } => (seq, false),
            // Core reports a replay only when both the envelope node and the
            // payload match the prior event, so the request is a true retry.
            AppendOutcome::IdempotentReplay { prior } => (prior.seq, true),
            // Stripe-style: the same key with a different request is a client
            // error, not a silent replay. Distinguish a node-id mismatch from
            // a payload mismatch (node id first, as before) for a precise msg.
            AppendOutcome::Conflict { prior } => {
                if prior.node_id.as_deref() != node_id.as_deref() {
                    return Err(CliError::user(
                        "idempotency_conflict",
                        format!(
                            "idempotency-key was previously used for a different --node-id \
                             (prior: {:?}, current: {:?})",
                            prior.node_id, node_id
                        ),
                    ));
                }
                return Err(CliError::user(
                    "idempotency_conflict",
                    "idempotency-key was previously used with a different --from-file payload",
                ));
            }
        }
    } else {
        let seq = RunLock::with_lock(&paths, |lock| {
            octl_core::append_and_apply_unlocked(
                lock,
                &paths,
                kind,
                envelope_node.as_ref(),
                None,
                data.clone(),
            )
        })
        .map_err(from_core)?;
        (seq, false)
    };

    let payload = CreatedPayload {
        run_id: &run_id,
        kind,
        node_id: node_id.as_deref(),
        seq: Some(seq),
        idempotent_replay: if idempotent_replay { Some(true) } else { None },
        dry_run: None,
        projections,
    };
    emit(&payload, args.spec, args.warnings)
}

fn emit(
    payload: &CreatedPayload<'_>,
    spec: &OutputSpec,
    warnings: &[String],
) -> Result<(), CliError> {
    match spec.format {
        OutputFormat::Json | OutputFormat::Jsonl => {
            output::emit_envelope(payload, spec, warnings)?;
        }
        OutputFormat::Text => {
            println!("run-id:      {}", payload.run_id);
            println!("kind:        {}", payload.kind);
            if let Some(n) = payload.node_id {
                println!("node-id:     {n}");
            }
            match payload.seq {
                Some(s) => println!("seq:         {s}"),
                None => println!("seq:         (assigned on apply)"),
            }
            if payload.dry_run == Some(true) {
                println!("note:        --dry-run (no filesystem changes)");
            }
            if payload.idempotent_replay == Some(true) {
                println!("note:        returned from idempotency-key cache");
            }
            if !payload.projections.is_empty() {
                println!("projections:");
                for p in &payload.projections {
                    println!("  - {p}");
                }
            }
            output::emit_text_warnings(warnings);
        }
    }
    Ok(())
}
