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

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use octl_core::{ensure_root, read_manifest_opt, read_node_opt, Kind, RunLock};

use crate::error::CliError;
use crate::output::{self, OutputFormat, OutputSpec};
use crate::run::{from_core, require_nonempty, require_safe_id, run_paths};

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
    pub spec: &'a OutputSpec,
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
    // Reject empty-or-whitespace keys. An empty key would silently
    // collapse unrelated retries (every "no key" caller would share
    // the same dedup slot).
    let idempotency_key = match args.idempotency_key {
        Some(k) => Some(require_nonempty(&k, "idempotency-key")?),
        None => None,
    };

    let bytes = read_capped(&args.from_file)?;
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
        return emit(&payload, args.spec, args.warnings);
    }

    ensure_root(&root).map_err(from_core)?;

    // Idempotency lookup + append must share one lock window so a
    // concurrent retry can't see "no prior event" and double-append.
    enum Outcome {
        Replayed(PriorReport),
        Appended(u64),
    }
    let outcome = RunLock::with_lock(&paths.lock(), || {
        if let Some(key) = idempotency_key.as_deref() {
            if let Some(prior) = find_prior_report(&paths.events(), key)? {
                return Ok(Outcome::Replayed(prior));
            }
        }
        let seq = octl_core::append_and_apply_unlocked(
            &paths,
            "node.report",
            Some(&node_id),
            idempotency_key.as_deref(),
            data.clone(),
        )?;
        Ok(Outcome::Appended(seq))
    })
    .map_err(from_core)?;

    let (event_seq, replayed) = match outcome {
        Outcome::Appended(seq) => (seq, false),
        Outcome::Replayed(prior) => {
            // Stripe-style: re-using a key with a different payload is
            // a client error, not a silent replay. Matches `event create`.
            let prior_node = prior.node_id.as_deref().unwrap_or("<none>");
            if prior_node != node_id.as_str() {
                return Err(CliError::user(
                    "idempotency_conflict",
                    format!(
                        "idempotency-key was previously used for a different --node-id \
                         (prior: {prior_node}, current: {node_id})"
                    ),
                ));
            }
            if prior.data != data {
                return Err(CliError::user(
                    "idempotency_conflict",
                    "idempotency-key was previously used with a different --from-file payload",
                ));
            }
            (prior.seq, true)
        }
    };

    let payload = ReportPayload {
        run_id: &run_id,
        node_id: &node_id,
        event_seq: Some(event_seq),
        idempotent_replay: if replayed { Some(true) } else { None },
        dry_run: None,
    };
    emit(&payload, args.spec, args.warnings)
}

/// Read `--from-file` while enforcing the size cap during the read
/// itself, not via a separate `metadata()` stat. `metadata()` followed
/// by `read()` is TOCTOU-vulnerable — the file could grow between the
/// two calls and we'd OOM. `take(MAX + 1)` defends without an extra
/// syscall round-trip.
fn read_capped(path: &Path) -> Result<Vec<u8>, CliError> {
    let mut f = std::fs::File::open(path).map_err(|e| {
        CliError::user(
            "from_file_unreadable",
            format!("open {}: {}", path.display(), e),
        )
        .with_invalid_value(path.display().to_string())
    })?;
    let meta = f.metadata().map_err(|e| {
        CliError::user(
            "from_file_unreadable",
            format!("stat {}: {}", path.display(), e),
        )
        .with_invalid_value(path.display().to_string())
    })?;
    if !meta.is_file() {
        return Err(CliError::user(
            "from_file_unreadable",
            format!("{} is not a regular file", path.display()),
        )
        .with_invalid_value(path.display().to_string()));
    }
    let mut buf = Vec::new();
    f.by_ref()
        .take(MAX_FROM_FILE_BYTES + 1)
        .read_to_end(&mut buf)
        .map_err(|e| {
            CliError::user(
                "from_file_unreadable",
                format!("read {}: {}", path.display(), e),
            )
            .with_invalid_value(path.display().to_string())
        })?;
    if (buf.len() as u64) > MAX_FROM_FILE_BYTES {
        return Err(CliError::user(
            "from_file_too_large",
            format!(
                "--from-file exceeds maximum of {} bytes",
                MAX_FROM_FILE_BYTES
            ),
        )
        .with_invalid_value(path.display().to_string()));
    }
    Ok(buf)
}

/// §7.3 payload validation. Rejects anything obviously not a report
/// before the reducer ever sees it, so the error envelope can name the
/// offending field instead of bubbling a generic `CorruptEventLog`.
fn validate_report_payload(data: &Value) -> Result<(), CliError> {
    let obj = data.as_object().ok_or_else(|| {
        CliError::user("schema_violation", "report payload must be a JSON object")
    })?;

    // `success` is the one strictly required field per §7.3. A cancel-
    // synthesized report (§7.7) may carry `cancelled: true` AND
    // `success: false` — both are still booleans on the wire.
    let success = obj.get("success").ok_or_else(|| {
        CliError::user(
            "schema_violation",
            "report payload missing required field `success`",
        )
        .with_expected(json!({"field": "success", "type": "boolean"}))
    })?;
    if !success.is_boolean() {
        return Err(
            CliError::user("schema_violation", "field `success` must be a boolean")
                .with_expected(json!({"field": "success", "type": "boolean"})),
        );
    }

    if let Some(v) = obj.get("summary") {
        if !v.is_string() && !v.is_null() {
            return Err(CliError::user(
                "schema_violation",
                "field `summary` must be a string",
            ));
        }
    }
    let cancelled = match obj.get("cancelled") {
        None | Some(Value::Null) => false,
        Some(v) => v.as_bool().ok_or_else(|| {
            CliError::user("schema_violation", "field `cancelled` must be a boolean")
        })?,
    };
    let reason = match obj.get("reason") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_str().ok_or_else(|| {
            CliError::user("schema_violation", "field `reason` must be a string")
        })?),
    };

    // §7.7: a cancel-synthesized report carries `cancelled: true,
    // success: false, reason: <non-empty>`. Allowing `success: true`
    // alongside `cancelled: true` would persist a contradiction (the
    // reducer prioritizes `cancelled`, so the node would be cancelled
    // while `last_report.success == true`).
    if cancelled {
        if success.as_bool().unwrap() {
            return Err(CliError::user(
                "schema_violation",
                "`cancelled: true` requires `success: false`",
            ));
        }
        match reason {
            Some(s) if !s.trim().is_empty() => {}
            _ => {
                return Err(CliError::user(
                    "schema_violation",
                    "`cancelled: true` requires a non-empty `reason` string",
                ));
            }
        }
    }

    validate_discussion_items(obj.get("discussion_items"))?;
    validate_spinoff_proposals(obj.get("spinoff_proposals"))?;
    validate_string_array(
        obj.get("wrap_up_recommendations"),
        "wrap_up_recommendations",
    )?;
    Ok(())
}

fn validate_discussion_items(v: Option<&Value>) -> Result<(), CliError> {
    let arr = match v {
        Some(Value::Array(a)) => a,
        Some(_) => {
            return Err(CliError::user(
                "schema_violation",
                "field `discussion_items` must be an array",
            ));
        }
        None => return Ok(()),
    };
    for (i, item) in arr.iter().enumerate() {
        let obj = item.as_object().ok_or_else(|| {
            CliError::user(
                "schema_violation",
                format!("discussion_items[{i}] must be a JSON object"),
            )
        })?;
        let topic = obj.get("topic").and_then(Value::as_str);
        if topic.is_none() || topic.unwrap().trim().is_empty() {
            return Err(CliError::user(
                "schema_violation",
                format!("discussion_items[{i}].topic must be a non-empty string"),
            ));
        }
        if let Some(sev) = obj.get("severity") {
            let _ = sev.as_str().ok_or_else(|| {
                CliError::user(
                    "schema_violation",
                    format!("discussion_items[{i}].severity must be a string"),
                )
            })?;
            // §7.3 example lists "discuss|critical" but the design
            // calls for forward-compatibility — accept any string and
            // let the supervisor interpret unknown severities. (A
            // CLI-side closed-set check would deadlock agents shipped
            // ahead of a CLI release; see review #2/DeepSeek and #15
            // /Claude.)
        }
        if let Some(opts) = obj.get("options") {
            validate_string_array_at(opts, &format!("discussion_items[{i}].options"))?;
        }
    }
    Ok(())
}

fn validate_spinoff_proposals(v: Option<&Value>) -> Result<(), CliError> {
    let arr = match v {
        Some(Value::Array(a)) => a,
        Some(_) => {
            return Err(CliError::user(
                "schema_violation",
                "field `spinoff_proposals` must be an array",
            ));
        }
        None => return Ok(()),
    };
    for (i, item) in arr.iter().enumerate() {
        let obj = item.as_object().ok_or_else(|| {
            CliError::user(
                "schema_violation",
                format!("spinoff_proposals[{i}] must be a JSON object"),
            )
        })?;
        let title = obj.get("proposed_title").and_then(Value::as_str);
        if title.is_none() || title.unwrap().trim().is_empty() {
            return Err(CliError::user(
                "schema_violation",
                format!("spinoff_proposals[{i}].proposed_title must be a non-empty string"),
            ));
        }
        let kind_str = obj
            .get("proposed_kind")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                CliError::user(
                    "schema_violation",
                    format!("spinoff_proposals[{i}].proposed_kind must be a string"),
                )
            })?;
        // Reject unknown kinds at the CLI boundary so the supervisor
        // never has to translate a generic `CorruptEventLog` for the
        // user. Mirrors the `Kind` enum's `rename_all = "kebab-case"`
        // serde routing.
        if serde_json::from_value::<Kind>(Value::String(kind_str.to_string())).is_err() {
            return Err(CliError::user(
                "schema_violation",
                format!("spinoff_proposals[{i}].proposed_kind `{kind_str}` is not a known kind"),
            )
            .with_expected(json!([
                "code",
                "spinoff",
                "orchestrated",
                "research",
                "technical-decision",
                "make-skill",
                "fan-out",
                "bugfix"
            ])));
        }
        if let Some(rationale) = obj.get("rationale") {
            if !rationale.is_string() && !rationale.is_null() {
                return Err(CliError::user(
                    "schema_violation",
                    format!("spinoff_proposals[{i}].rationale must be a string"),
                ));
            }
        }
    }
    Ok(())
}

/// Path-aware string-array validator. Used for nested fields where
/// the caller wants to embed an index in the error message.
fn validate_string_array_at(v: &Value, path: &str) -> Result<(), CliError> {
    let arr = v
        .as_array()
        .ok_or_else(|| CliError::user("schema_violation", format!("{path} must be an array")))?;
    for (i, item) in arr.iter().enumerate() {
        if !item.is_string() {
            return Err(CliError::user(
                "schema_violation",
                format!("{path}[{i}] must be a string"),
            ));
        }
    }
    Ok(())
}

fn validate_string_array(v: Option<&Value>, field: &str) -> Result<(), CliError> {
    let arr = match v {
        Some(Value::Array(a)) => a,
        Some(_) => {
            return Err(CliError::user(
                "schema_violation",
                format!("field `{field}` must be an array"),
            ));
        }
        None => return Ok(()),
    };
    for (i, item) in arr.iter().enumerate() {
        if !item.is_string() {
            return Err(CliError::user(
                "schema_violation",
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

fn emit(
    payload: &ReportPayload<'_>,
    spec: &OutputSpec,
    warnings: &[String],
) -> Result<(), CliError> {
    match spec.format {
        OutputFormat::Json | OutputFormat::Jsonl => {
            output::emit_envelope(payload, spec, warnings)?;
        }
        OutputFormat::Text => {
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
            output::emit_text_warnings(warnings);
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
        assert_eq!(err.code, "schema_violation");
    }

    #[test]
    fn non_object_root_rejected() {
        let v = json!([1, 2, 3]);
        let err = validate_report_payload(&v).unwrap_err();
        assert_eq!(err.code, "schema_violation");
    }

    #[test]
    fn discussion_item_missing_topic_rejected() {
        let v = json!({
            "success": true,
            "discussion_items": [{"severity": "discuss"}],
        });
        let err = validate_report_payload(&v).unwrap_err();
        assert_eq!(err.code, "schema_violation");
    }

    #[test]
    fn discussion_item_unknown_severity_accepted_for_forward_compat() {
        // Forward-compat: a supervisor may add new severities without
        // a CLI release. The validator only enforces that severity is
        // a string, not a closed set.
        let v = json!({
            "success": true,
            "discussion_items": [{"topic": "x", "severity": "info"}],
        });
        assert!(validate_report_payload(&v).is_ok());
    }

    #[test]
    fn discussion_item_non_string_severity_rejected() {
        let v = json!({
            "success": true,
            "discussion_items": [{"topic": "x", "severity": 42}],
        });
        let err = validate_report_payload(&v).unwrap_err();
        assert_eq!(err.code, "schema_violation");
    }

    #[test]
    fn discussion_item_options_must_be_strings() {
        let v = json!({
            "success": true,
            "discussion_items": [{"topic": "x", "options": [1, 2]}],
        });
        let err = validate_report_payload(&v).unwrap_err();
        assert_eq!(err.code, "schema_violation");
    }

    #[test]
    fn spinoff_unknown_proposed_kind_rejected() {
        let v = json!({
            "success": true,
            "spinoff_proposals": [{"proposed_title": "x", "proposed_kind": "not-a-kind"}],
        });
        let err = validate_report_payload(&v).unwrap_err();
        assert_eq!(err.code, "schema_violation");
    }

    #[test]
    fn cancelled_requires_success_false() {
        let v = json!({"success": true, "cancelled": true, "reason": "x"});
        let err = validate_report_payload(&v).unwrap_err();
        assert_eq!(err.code, "schema_violation");
    }

    #[test]
    fn cancelled_requires_reason() {
        let v = json!({"success": false, "cancelled": true});
        let err = validate_report_payload(&v).unwrap_err();
        assert_eq!(err.code, "schema_violation");
    }

    #[test]
    fn spinoff_missing_kind_rejected() {
        let v = json!({
            "success": true,
            "spinoff_proposals": [{"proposed_title": "x"}],
        });
        let err = validate_report_payload(&v).unwrap_err();
        assert_eq!(err.code, "schema_violation");
    }

    #[test]
    fn wrap_up_must_be_string_array() {
        let v = json!({
            "success": true,
            "wrap_up_recommendations": ["ok", 42],
        });
        let err = validate_report_payload(&v).unwrap_err();
        assert_eq!(err.code, "schema_violation");
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
