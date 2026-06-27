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

use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

use octl_core::report::{validate_report_payload, ReportValidationError};
use octl_core::{
    ensure_root, find_prior_with_key, read_manifest_opt, read_node_opt, PriorEvent, RunLock,
    RunPaths,
};

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

    validate_report_payload(&data).map_err(map_report_validation_error)?;

    let root = crate::home::root_dir()?;
    let paths = run_paths(&root, &run_id)?;

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
        Replayed(PriorEvent),
        Appended(u64),
    }
    let outcome = RunLock::with_lock(&paths.lock(), || {
        if let Some(key) = idempotency_key.as_deref() {
            if let Some(prior) = find_prior_report(&paths, key)? {
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
            format!("--from-file exceeds maximum of {MAX_FROM_FILE_BYTES} bytes"),
        )
        .with_invalid_value(path.display().to_string()));
    }
    Ok(buf)
}

/// Map a domain [`ReportValidationError`] to the CLI's `schema_violation`
/// envelope. The validator itself lives in `octl_core::report` so the
/// supervisor can reuse it (handoff D3); this is the CLI-boundary
/// translation, preserving the structured `expected` hints the agent UX
/// relied on.
fn map_report_validation_error(err: ReportValidationError) -> CliError {
    let mut cli = CliError::user("schema_violation", err.to_string());
    if let Some(expected) = err.expected() {
        cli = cli.with_expected(expected);
    }
    cli
}

/// Locate a prior `node.report` event with this idempotency `key`. Thin
/// wrapper over [`octl_core::find_prior_with_key`] pinned to the
/// `node.report` kind — see there for the torn-line policy and the
/// requirement that the caller hold the run's `flock`.
fn find_prior_report(paths: &RunPaths, key: &str) -> octl_core::Result<Option<PriorEvent>> {
    find_prior_with_key(paths, "node.report", key)
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
                Some(s) => println!("event_seq: {s}"),
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
    use serde_json::json;

    // Deep, per-field validation now lives in `octl_core::report`. These
    // tests exercise the CLI boundary: that `validate_report_payload`'s
    // domain errors map to the `schema_violation` envelope and that the
    // structured `expected` hints survive the translation.

    /// Run the core validator and translate as the CLI does at the call site.
    fn validate(v: &Value) -> Result<(), CliError> {
        validate_report_payload(v).map_err(map_report_validation_error)
    }

    #[test]
    fn valid_payload_passes_through() {
        let v = json!({"success": true});
        assert!(validate(&v).is_ok());
    }

    #[test]
    fn domain_error_maps_to_schema_violation() {
        let v = json!({"summary": "no success field"});
        let err = validate(&v).unwrap_err();
        assert_eq!(err.code, "schema_violation");
        // The missing-`success` hint must survive the boundary mapping.
        assert_eq!(
            err.expected,
            Some(json!({"field": "success", "type": "boolean"}))
        );
    }

    #[test]
    fn unknown_kind_hint_survives_mapping() {
        let v = json!({
            "success": true,
            "spinoff_proposals": [{"proposed_title": "x", "proposed_kind": "not-a-kind"}],
        });
        let err = validate(&v).unwrap_err();
        assert_eq!(err.code, "schema_violation");
        // The closed-set of known kinds is carried through to the envelope.
        assert!(err.expected.is_some());
    }

    #[test]
    fn error_without_hint_maps_with_no_expected() {
        let v = json!({"success": true, "summary": 42});
        let err = validate(&v).unwrap_err();
        assert_eq!(err.code, "schema_violation");
        assert!(err.expected.is_none());
    }
}
