//! `event create` — sanctioned write path for arbitrary event kinds.
//!
//! Validates `--kind` against the closed MVP event-kind set (design.md
//! §1.4), enforces `--node-id` for kinds that reference a specific node,
//! reads the `data` payload from `--from-file`, then appends + applies the
//! event under one `flock` window via `octl_core::append_and_apply`.
//!
//! `--idempotency-key` dedup scans the existing event log under the same
//! lock so concurrent retries can't race past each other.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use octl_core::{ensure_root, RunLock};

use crate::error::CliError;
use crate::output::{self, OutputFormat, OutputSpec};
use crate::run::{from_core, require_safe_id, run_paths};

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
    /// Best-effort guess of projection files this event will / would
    /// touch. Hand-maintained mirror of the reducer — moving this into
    /// `octl-core::reducer` is tracked as `projected-paths-into-reducer`.
    projections: Vec<String>,
}

/// Closed set of event kinds per design.md §1.4. `node.heartbeat` is
/// intentionally absent — design.md §7.5 names it "future opt-in" and
/// no reducer case exists yet. Adding a new kind here is intentional:
/// the reducer must learn it first.
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
    "supervisor.exited",
    "supervisor.reattach-requested",
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
        "node.created" | "node.status" | "node.report" | "child.spawned"
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
    // Event kinds are enumerated explicitly; some distinct kinds map to the same
    // projection-file set, which clippy would flag as duplicate arms.
    #[allow(clippy::match_same_arms)]
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
        "discussion.opened" | "discussion.resolved" => {
            if let Some(id) = data.get("discussion_id").and_then(Value::as_str) {
                out.push(format!("discussions/{id}.json"));
            }
            out.push("manifest.json".into());
        }
        "spinoff.proposed" | "spinoff.approved" | "spinoff.rejected" => {
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
        // supervisor.* are recorded facts only; no projection files
        // change today.
        _ => {}
    }
    out
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
        require_safe_id(s, "data.node_id")?;
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
            require_safe_id(s, "data.discussion_id")?;
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
            require_safe_id(s, "data.proposal_id")?;
        }
    }
    if kind == "child.spawned" {
        if let Some(v) = data.get("child_run_id") {
            let s = v.as_str().ok_or_else(|| {
                CliError::user("invalid_data_id", "data.child_run_id must be a string")
            })?;
            require_safe_id(s, "data.child_run_id")?;
        }
        if let Some(v) = data.get("child_node_id") {
            let s = v.as_str().ok_or_else(|| {
                CliError::user("invalid_data_id", "data.child_node_id must be a string")
            })?;
            require_safe_id(s, "data.child_node_id")?;
        }
    }
    Ok(())
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

    let projections = projected_paths(kind, node_id.as_deref(), &data);

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

    // Idempotency check + append must run inside one lock window so a
    // concurrent retry can't see "no prior event" and double-append.
    let (seq, replayed) = RunLock::with_lock(&paths.lock(), || {
        if let Some(key) = args.idempotency_key.as_deref() {
            if let Some(prior) = find_prior_event(&paths.events(), kind, key)? {
                return Ok((prior, true));
            }
        }
        let seq = octl_core::append_and_apply_unlocked(
            &paths,
            kind,
            node_id.as_deref(),
            args.idempotency_key.as_deref(),
            data.clone(),
        )?;
        Ok((
            PriorEvent {
                seq,
                node_id: node_id.clone(),
                data: data.clone(),
            },
            false,
        ))
    })
    .map_err(from_core)?;

    if replayed {
        // Stripe-style: the same idempotency key with a different
        // payload is a client error, not a silent replay. The CLI
        // would otherwise return the original seq + the new request's
        // projections, lying about what was recorded.
        if seq.node_id.as_deref() != node_id.as_deref() {
            return Err(CliError::user(
                "idempotency_conflict",
                format!(
                    "idempotency-key was previously used for a different --node-id \
                     (prior: {:?}, current: {:?})",
                    seq.node_id, node_id
                ),
            ));
        }
        if seq.data != data {
            return Err(CliError::user(
                "idempotency_conflict",
                "idempotency-key was previously used with a different --from-file payload",
            ));
        }
    }

    let payload = CreatedPayload {
        run_id: &run_id,
        kind,
        node_id: node_id.as_deref(),
        seq: Some(seq.seq),
        idempotent_replay: if replayed { Some(true) } else { None },
        dry_run: None,
        projections,
    };
    emit(&payload, args.spec, args.warnings)
}

/// What `find_prior_event` returns when an idempotency-key hit lands.
/// We need more than the bare seq so the caller can validate that the
/// retry payload matches.
struct PriorEvent {
    seq: u64,
    node_id: Option<String>,
    data: Value,
}

/// Stream-scan `events.jsonl` for an event with matching `kind` and
/// `idempotency_key`. Deserialises only the fields the lookup needs so
/// the cost stays bounded in `data` size, not log size × payload size.
///
/// Caller must hold the run's `flock`.
fn find_prior_event(
    events_path: &std::path::Path,
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
        // Skim past lines whose `kind` / `idempotency_key` don't match
        // without ever parsing the (potentially large) `data` payload.
        let probe: ProbeFields = match serde_json::from_str(&line) {
            Ok(p) => p,
            // A torn final line is tolerated by `recover_last_seq`;
            // mirror that tolerance here so an idempotency lookup
            // doesn't itself wedge on the same condition.
            Err(_) => continue,
        };
        if probe.kind != kind || probe.idempotency_key.as_deref() != Some(key) {
            continue;
        }
        let full: FullEventForReplay =
            serde_json::from_str(&line).map_err(|e| octl_core::Error::json(events_path, e))?;
        return Ok(Some(PriorEvent {
            seq: full.seq,
            node_id: full.node_id,
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
    node_id: Option<String>,
    data: Value,
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
