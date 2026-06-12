//! `event create` — sanctioned write path for arbitrary event kinds.
//!
//! Validates `--kind` against the closed MVP event-kind set (design.md
//! §1.4), enforces `--node-id` for kinds that reference a specific node,
//! reads the `data` payload from `--from-file`, then appends + applies the
//! event under one `flock` window via `octl_core::append_and_apply`.
//!
//! `--idempotency-key` dedup scans the existing event log under the same
//! lock so concurrent retries can't race past each other.

use std::path::PathBuf;

use serde::Serialize;
use serde_json::{json, Value};

use octl_core::{ensure_root, read_all_events, RunLock};

use crate::error::CliError;
use crate::output;
use crate::run::{from_core, require_safe_id, run_paths};

pub struct Args<'a> {
    pub run_id: String,
    pub kind: String,
    pub node_id: Option<String>,
    pub from_file: PathBuf,
    pub idempotency_key: Option<String>,
    pub dry_run: bool,
    pub json: bool,
    pub warnings: &'a [String],
}

#[derive(Serialize)]
struct CreatedPayload<'a> {
    run_id: &'a str,
    kind: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    node_id: Option<&'a str>,
    seq: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    idempotent_replay: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dry_run: Option<bool>,
    /// Paths (relative to the run dir) that would be / were written to.
    projections: Vec<String>,
}

/// Closed set of event kinds per design.md §1.4. Adding a new kind here
/// is intentional — the reducer must learn it first.
const ALLOWED_KINDS: &[&str] = &[
    "run.created",
    "run.status",
    "node.created",
    "node.status",
    "node.report",
    "node.heartbeat",
    "discussion.opened",
    "discussion.resolved",
    "spinoff.proposed",
    "spinoff.approved",
    "spinoff.rejected",
    "child.spawned",
    "supervisor.exited",
    "supervisor.reattach-requested",
];

/// Kinds where the reducer requires a top-level `node_id` to be useful.
/// Without it, the append succeeds but the reducer silently no-ops.
fn requires_node_id(kind: &str) -> bool {
    matches!(
        kind,
        "node.created" | "node.status" | "node.report" | "node.heartbeat" | "child.spawned"
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

/// Best-guess of the projection files this event will touch. Used by
/// `--dry-run` and by the success envelope to give callers a visible
/// trace of the reducer side-effects.
fn projected_paths(kind: &str, node_id: Option<&str>, data: &Value) -> Vec<String> {
    let mut out = Vec::new();
    match kind {
        "run.created" | "run.status" => out.push("manifest.json".into()),
        "node.created" => {
            if let Some(n) = node_id {
                out.push(format!("nodes/{n}.json"));
            }
            out.push("manifest.json".into());
        }
        "node.status" | "node.report" => {
            if let Some(n) = node_id {
                out.push(format!("nodes/{n}.json"));
            }
        }
        "discussion.opened" => {
            if let Some(id) = data.get("discussion_id").and_then(Value::as_str) {
                out.push(format!("discussions/{id}.json"));
            }
            out.push("manifest.json".into());
        }
        "discussion.resolved" => {
            if let Some(id) = data.get("discussion_id").and_then(Value::as_str) {
                out.push(format!("discussions/{id}.json"));
            }
            out.push("manifest.json".into());
        }
        "spinoff.proposed" => {
            if let Some(id) = data.get("proposal_id").and_then(Value::as_str) {
                out.push(format!("spinoffs/{id}.json"));
            }
            out.push("manifest.json".into());
        }
        "spinoff.approved" | "spinoff.rejected" => {
            if let Some(id) = data.get("proposal_id").and_then(Value::as_str) {
                out.push(format!("spinoffs/{id}.json"));
            }
            out.push("manifest.json".into());
        }
        "child.spawned" => {
            if let Some(n) = node_id {
                out.push(format!("nodes/{n}.json"));
            }
        }
        // node.heartbeat / supervisor.* are recorded facts only; no
        // projection files change today.
        _ => {}
    }
    out
}

pub fn run(args: Args<'_>) -> Result<(), CliError> {
    let run_id = require_safe_id(&args.run_id, "run-id")?;

    if !ALLOWED_KINDS.contains(&args.kind.as_str()) {
        return Err(CliError::user(
            "unknown_event_kind",
            format!("unknown event kind: {}", args.kind),
        )
        .with_invalid_value(&args.kind)
        .with_expected(json!(ALLOWED_KINDS)));
    }
    let kind = args.kind.as_str();

    // Reject `--node-id` for kinds that can't reference one — silently
    // accepting it would let callers paper over a typo (writing
    // `--kind run.status --node-id n-0001` and then wondering why the
    // node didn't change).
    if args.node_id.is_some() && !allows_node_id(kind) {
        return Err(CliError::user(
            "unexpected_flag",
            format!("--node-id is not accepted for kind `{kind}`"),
        )
        .with_invalid_value(kind));
    }

    let node_id = match args.node_id.as_deref() {
        Some(v) => Some(require_safe_id(v, "node-id")?),
        None => None,
    };

    if requires_node_id(kind) && node_id.is_none() {
        return Err(CliError::user(
            "missing_required_flag",
            format!("--node-id is required for kind `{kind}`"),
        )
        .with_expected(json!({"flag": "--node-id"})));
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

    let root = crate::home::root_dir()?;
    let paths = run_paths(&root, &run_id);

    // The run dir must already exist — `event create` is a write path
    // for an existing run; it does NOT bootstrap one. (run.created on a
    // fresh dir is technically allowed, but only when the run dir has
    // been pre-created. The skill-shim contract uses `run create` for
    // that path.)
    if !paths.root.exists() {
        return Err(
            CliError::user("run_not_found", format!("no run with id {run_id}"))
                .with_invalid_value(&run_id),
        );
    }

    let projections = projected_paths(kind, node_id.as_deref(), &data);

    if args.dry_run {
        let payload = CreatedPayload {
            run_id: &run_id,
            kind,
            node_id: node_id.as_deref(),
            seq: 0,
            idempotent_replay: None,
            dry_run: Some(true),
            projections,
        };
        return emit(&payload, args.json, args.warnings);
    }

    ensure_root(&root).map_err(from_core)?;

    // Idempotency check + append must run inside one lock window so a
    // concurrent retry can't see "no prior event" and double-append.
    let (seq, replayed) = RunLock::with_lock(&paths.lock(), || {
        if let Some(key) = args.idempotency_key.as_deref() {
            if let Some(prior_seq) = find_prior_seq(&paths.events(), kind, key)? {
                return Ok((prior_seq, true));
            }
        }
        let seq = octl_core::append_and_apply_unlocked(
            &paths,
            kind,
            node_id.as_deref(),
            args.idempotency_key.as_deref(),
            data.clone(),
        )?;
        Ok((seq, false))
    })
    .map_err(from_core)?;

    let payload = CreatedPayload {
        run_id: &run_id,
        kind,
        node_id: node_id.as_deref(),
        seq,
        idempotent_replay: if replayed { Some(true) } else { None },
        dry_run: None,
        projections,
    };
    emit(&payload, args.json, args.warnings)
}

/// Scan `events.jsonl` for an event with matching `kind` and
/// `idempotency_key`. Returns its `seq` on hit.
///
/// Caller must hold the run's `flock`.
fn find_prior_seq(
    events_path: &std::path::Path,
    kind: &str,
    key: &str,
) -> octl_core::Result<Option<u64>> {
    let events = read_all_events(events_path)?;
    Ok(events
        .into_iter()
        .find(|e| e.kind == kind && e.idempotency_key.as_deref() == Some(key))
        .map(|e| e.seq))
}

fn emit(payload: &CreatedPayload<'_>, json: bool, warnings: &[String]) -> Result<(), CliError> {
    if json {
        output::emit_json(payload, warnings)
            .map_err(|e| CliError::system("internal_serialize", e.to_string()))?;
    } else {
        println!("run-id:      {}", payload.run_id);
        println!("kind:        {}", payload.kind);
        if let Some(n) = payload.node_id {
            println!("node-id:     {}", n);
        }
        if payload.dry_run == Some(true) {
            println!("seq:         (assigned on apply)");
            println!("note:        --dry-run (no filesystem changes)");
        } else {
            println!("seq:         {}", payload.seq);
        }
        if payload.idempotent_replay == Some(true) {
            println!("note:        returned from idempotency-key cache");
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
