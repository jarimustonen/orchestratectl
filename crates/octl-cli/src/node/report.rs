//! `node report` — agent self-submission of a structured §7.3 terminal
//! report. Domain verb (design.md §2.0, §2.2).
//!
//! Validates the §7.3 payload schema (`success` required; optional
//! `summary`, `discussion_items`, `spinoff_proposals`,
//! `wrap_up_recommendations`, plus `cancelled` / `reason` when a cancel
//! synthesizes a terminal report), appends `node.report` under the
//! run's `flock`, and lets the reducer update `nodes/<node-id>.json`
//! and propagate terminal status.
//!
//! `--idempotency-key` dedup mirrors `event create`: scan
//! `events.jsonl` under the same lock window for a prior `node.report`
//! event with the same key, return its `seq` instead of appending again.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use octl_core::{ensure_root, read_manifest_opt, read_node_opt, RunLock};

use crate::error::CliError;
use crate::output;
use crate::run::{from_core, require_safe_id, run_paths};

/// Mirror of `event create`'s 1 MiB cap. `node.report` is the largest
/// realistic payload (design.md §1.4 cites 10-50 KB); 1 MiB still
/// bounds CLI memory against an accidental `/dev/zero`.
const MAX_FROM_FILE_BYTES: u64 = 1024 * 1024;

pub struct Args<'a> {
    pub run_id: String,
    pub node_id: String,
    pub from_file: PathBuf,
    pub idempotency_key: Option<String>,
    pub dry_run: bool,
    pub json: bool,
    pub warnings: &'a [String],
}

#[derive(Serialize)]
struct ReportPayload<'a> {
    run_id: &'a str,
    node_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    event_seq: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    idempotent_replay: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dry_run: Option<bool>,
}

pub fn run(args: Args<'_>) -> Result<(), CliError> {
    let run_id = require_safe_id(&args.run_id, "run-id")?;
    let node_id = require_safe_id(&args.node_id, "node-id")?;

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

    let bytes = std::fs::read(&args.from_file).map_err(|e| {
        CliError::user(
            "from_file_unreadable",
            format!("read {}: {}", args.from_file.display(), e),
        )
        .with_invalid_value(args.from_file.display().to_string())
    })?;
    let data: Value = serde_json::from_slice(&bytes).map_err(|e| {
        CliError::user(
            "from_file_invalid_json",
            format!("parse {}: {}", args.from_file.display(), e),
        )
        .with_invalid_value(args.from_file.display().to_string())
    })?;

    validate_report_payload(&data)?;

    let root = crate::home::root_dir()?;
    let paths = run_paths(&root, &run_id);

    // Both manifest and node must exist — `node report` is only meant
    // to update a live node, not bootstrap one. Reporting against a
    // node that doesn't exist would silently no-op in the reducer
    // (`apply_node_report` returns Ok if the node file is missing) and
    // the caller would never learn their `--node-id` was wrong.
    if read_manifest_opt(&paths).map_err(from_core)?.is_none() {
        return Err(
            CliError::user("run_not_found", format!("no run with id {run_id}"))
                .with_invalid_value(&run_id),
        );
    }
    if read_node_opt(&paths, &node_id)
        .map_err(from_core)?
        .is_none()
    {
        return Err(CliError::user(
            "node_not_found",
            format!("no node {node_id} in run {run_id}"),
        )
        .with_invalid_value(&node_id));
    }

    if args.dry_run {
        let payload = ReportPayload {
            run_id: &run_id,
            node_id: &node_id,
            event_seq: None,
            idempotent_replay: None,
            dry_run: Some(true),
        };
        return emit(&payload, args.json, args.warnings);
    }

    ensure_root(&root).map_err(from_core)?;

    // Idempotency lookup + append must share one lock window so a
    // concurrent retry can't see "no prior event" and double-append.
    let (seq, replayed) = RunLock::with_lock(&paths.lock(), || {
        if let Some(key) = args.idempotency_key.as_deref() {
            if let Some(prior) = find_prior_report(&paths.events(), key)? {
                return Ok((prior, true));
            }
        }
        let seq = octl_core::append_and_apply_unlocked(
            &paths,
            "node.report",
            Some(&node_id),
            args.idempotency_key.as_deref(),
            data.clone(),
        )?;
        Ok((
            PriorReport {
                seq,
                node_id: Some(node_id.clone()),
                data: data.clone(),
            },
            false,
        ))
    })
    .map_err(from_core)?;

    if replayed {
        // Stripe-style: re-using a key with a different payload is a
        // client error, not a silent replay. Matches `event create`.
        if seq.node_id.as_deref() != Some(node_id.as_str()) {
            return Err(CliError::user(
                "idempotency_conflict",
                format!(
                    "idempotency-key was previously used for a different --node-id \
                     (prior: {:?}, current: {})",
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

    let payload = ReportPayload {
        run_id: &run_id,
        node_id: &node_id,
        event_seq: Some(seq.seq),
        idempotent_replay: if replayed { Some(true) } else { None },
        dry_run: None,
    };
    emit(&payload, args.json, args.warnings)
}

/// §7.3 payload validation. Rejects anything obviously not a report
/// before the reducer ever sees it, so the error envelope can name the
/// offending field instead of bubbling a generic `CorruptEventLog`.
fn validate_report_payload(data: &Value) -> Result<(), CliError> {
    let obj = data.as_object().ok_or_else(|| {
        CliError::user("schema-violation", "report payload must be a JSON object")
    })?;

    // `success` is the one strictly required field per §7.3. A cancel-
    // synthesized report (§7.7) may carry `cancelled: true` AND
    // `success: false` — both are still booleans on the wire.
    let success = obj.get("success").ok_or_else(|| {
        CliError::user(
            "schema-violation",
            "report payload missing required field `success`",
        )
        .with_expected(json!({"field": "success", "type": "boolean"}))
    })?;
    if !success.is_boolean() {
        return Err(
            CliError::user("schema-violation", "field `success` must be a boolean")
                .with_expected(json!({"field": "success", "type": "boolean"})),
        );
    }

    if let Some(v) = obj.get("summary") {
        if !v.is_string() && !v.is_null() {
            return Err(CliError::user(
                "schema-violation",
                "field `summary` must be a string",
            ));
        }
    }
    if let Some(v) = obj.get("cancelled") {
        if !v.is_boolean() {
            return Err(CliError::user(
                "schema-violation",
                "field `cancelled` must be a boolean",
            ));
        }
    }
    if let Some(v) = obj.get("reason") {
        if !v.is_string() && !v.is_null() {
            return Err(CliError::user(
                "schema-violation",
                "field `reason` must be a string",
            ));
        }
    }

    validate_discussion_items(obj.get("discussion_items"))?;
    validate_spinoff_proposals(obj.get("spinoff_proposals"))?;
    validate_string_array(
        obj.get("wrap_up_recommendations"),
        "wrap_up_recommendations",
    )?;
    // `decisions` is mentioned in the issue summary as an optional
    // field; design.md §7.3 doesn't pin its shape, so we accept any
    // array and let the supervisor interpret it. Reject non-array so
    // we still catch typos.
    if let Some(v) = obj.get("decisions") {
        if !v.is_array() {
            return Err(CliError::user(
                "schema-violation",
                "field `decisions` must be an array",
            ));
        }
    }
    Ok(())
}

fn validate_discussion_items(v: Option<&Value>) -> Result<(), CliError> {
    let arr = match v {
        Some(Value::Array(a)) => a,
        Some(_) => {
            return Err(CliError::user(
                "schema-violation",
                "field `discussion_items` must be an array",
            ));
        }
        None => return Ok(()),
    };
    for (i, item) in arr.iter().enumerate() {
        let obj = item.as_object().ok_or_else(|| {
            CliError::user(
                "schema-violation",
                format!("discussion_items[{i}] must be a JSON object"),
            )
        })?;
        let topic = obj.get("topic").and_then(Value::as_str);
        if topic.is_none() || topic.unwrap().trim().is_empty() {
            return Err(CliError::user(
                "schema-violation",
                format!("discussion_items[{i}].topic must be a non-empty string"),
            ));
        }
        if let Some(sev) = obj.get("severity") {
            let s = sev.as_str().ok_or_else(|| {
                CliError::user(
                    "schema-violation",
                    format!("discussion_items[{i}].severity must be a string"),
                )
            })?;
            // §7.3 example lists "discuss|critical"; keep enforcement
            // soft (warn-by-rejecting unknown) so the supervisor can
            // add new severities without coordinating a CLI release.
            if !matches!(s, "discuss" | "critical") {
                return Err(CliError::user(
                    "schema-violation",
                    format!(
                        "discussion_items[{i}].severity must be `discuss` or `critical` (got `{s}`)"
                    ),
                )
                .with_expected(json!(["discuss", "critical"])));
            }
        }
        if let Some(opts) = obj.get("options") {
            if !opts.is_array() {
                return Err(CliError::user(
                    "schema-violation",
                    format!("discussion_items[{i}].options must be an array"),
                ));
            }
        }
    }
    Ok(())
}

fn validate_spinoff_proposals(v: Option<&Value>) -> Result<(), CliError> {
    let arr = match v {
        Some(Value::Array(a)) => a,
        Some(_) => {
            return Err(CliError::user(
                "schema-violation",
                "field `spinoff_proposals` must be an array",
            ));
        }
        None => return Ok(()),
    };
    for (i, item) in arr.iter().enumerate() {
        let obj = item.as_object().ok_or_else(|| {
            CliError::user(
                "schema-violation",
                format!("spinoff_proposals[{i}] must be a JSON object"),
            )
        })?;
        let title = obj.get("proposed_title").and_then(Value::as_str);
        if title.is_none() || title.unwrap().trim().is_empty() {
            return Err(CliError::user(
                "schema-violation",
                format!("spinoff_proposals[{i}].proposed_title must be a non-empty string"),
            ));
        }
        let kind = obj.get("proposed_kind").and_then(Value::as_str);
        if kind.is_none() {
            return Err(CliError::user(
                "schema-violation",
                format!("spinoff_proposals[{i}].proposed_kind must be a string"),
            ));
        }
        if let Some(rationale) = obj.get("rationale") {
            if !rationale.is_string() && !rationale.is_null() {
                return Err(CliError::user(
                    "schema-violation",
                    format!("spinoff_proposals[{i}].rationale must be a string"),
                ));
            }
        }
    }
    Ok(())
}

fn validate_string_array(v: Option<&Value>, field: &str) -> Result<(), CliError> {
    let arr = match v {
        Some(Value::Array(a)) => a,
        Some(_) => {
            return Err(CliError::user(
                "schema-violation",
                format!("field `{field}` must be an array"),
            ));
        }
        None => return Ok(()),
    };
    for (i, item) in arr.iter().enumerate() {
        if !item.is_string() {
            return Err(CliError::user(
                "schema-violation",
                format!("{field}[{i}] must be a string"),
            ));
        }
    }
    Ok(())
}

/// Mirrors `event create`'s `find_prior_event` but specialized to
/// `node.report`. Could be DRY'd with that helper in a follow-up — see
/// `handoff.md`-style spin-off note in the issue closing message.
fn find_prior_report(events_path: &Path, key: &str) -> octl_core::Result<Option<PriorReport>> {
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
            // Tolerate a torn final line the same way `recover_last_seq`
            // does — an in-flight crash mid-write shouldn't wedge
            // idempotency lookup.
            Err(_) => continue,
        };
        if probe.kind != "node.report" || probe.idempotency_key.as_deref() != Some(key) {
            continue;
        }
        let full: FullEventForReplay =
            serde_json::from_str(&line).map_err(|e| octl_core::Error::json(events_path, e))?;
        return Ok(Some(PriorReport {
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

struct PriorReport {
    seq: u64,
    node_id: Option<String>,
    data: Value,
}

fn emit(payload: &ReportPayload<'_>, json: bool, warnings: &[String]) -> Result<(), CliError> {
    if json {
        output::emit_json(payload, warnings)
            .map_err(|e| CliError::system("internal_serialize", e.to_string()))?;
    } else {
        println!("run-id:    {}", payload.run_id);
        println!("node-id:   {}", payload.node_id);
        match payload.event_seq {
            Some(s) => println!("event_seq: {}", s),
            None => println!("event_seq: (assigned on apply)"),
        }
        if payload.dry_run == Some(true) {
            println!("note:      --dry-run (no filesystem changes)");
        }
        if payload.idempotent_replay == Some(true) {
            println!("note:      returned from idempotency-key cache");
        }
        for w in warnings {
            eprintln!("warning: {}", w);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_minimal_success_payload() {
        let v = json!({"success": true});
        assert!(validate_report_payload(&v).is_ok());
    }

    #[test]
    fn missing_success_rejected() {
        let v = json!({"summary": "no success field"});
        let err = validate_report_payload(&v).unwrap_err();
        assert_eq!(err.code, "schema-violation");
    }

    #[test]
    fn non_object_root_rejected() {
        let v = json!([1, 2, 3]);
        let err = validate_report_payload(&v).unwrap_err();
        assert_eq!(err.code, "schema-violation");
    }

    #[test]
    fn discussion_item_missing_topic_rejected() {
        let v = json!({
            "success": true,
            "discussion_items": [{"severity": "discuss"}],
        });
        let err = validate_report_payload(&v).unwrap_err();
        assert_eq!(err.code, "schema-violation");
    }

    #[test]
    fn discussion_item_unknown_severity_rejected() {
        let v = json!({
            "success": true,
            "discussion_items": [{"topic": "x", "severity": "panic"}],
        });
        let err = validate_report_payload(&v).unwrap_err();
        assert_eq!(err.code, "schema-violation");
    }

    #[test]
    fn spinoff_missing_kind_rejected() {
        let v = json!({
            "success": true,
            "spinoff_proposals": [{"proposed_title": "x"}],
        });
        let err = validate_report_payload(&v).unwrap_err();
        assert_eq!(err.code, "schema-violation");
    }

    #[test]
    fn wrap_up_must_be_string_array() {
        let v = json!({
            "success": true,
            "wrap_up_recommendations": ["ok", 42],
        });
        let err = validate_report_payload(&v).unwrap_err();
        assert_eq!(err.code, "schema-violation");
    }

    #[test]
    fn cancel_synthesized_report_shape_ok() {
        // Mirror of run cancel's synthesized payload (run/cancel.rs).
        let v = json!({
            "success": false,
            "cancelled": true,
            "reason": "cancelled by user",
            "summary": "Run cancelled before agent reported.",
            "discussion_items": [],
            "spinoff_proposals": [],
            "wrap_up_recommendations": [],
        });
        assert!(validate_report_payload(&v).is_ok());
    }
}
