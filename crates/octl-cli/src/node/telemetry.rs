//! Harness-neutral advisory worker-telemetry update endpoint.
//!
//! This module is intentionally write-only with respect to telemetry. It
//! validates one bounded request and delegates to `octl-core`; no run lifecycle
//! command imports or consumes it.

use std::io::Read;
use std::path::{Path, PathBuf};

use octl_core::{
    parse_telemetry_update, update_telemetry, RunPaths, TelemetryError, TelemetryState,
    TelemetryUpdate, TELEMETRY_MAX_BYTES, TELEMETRY_PROTOCOL_VERSION, TELEMETRY_SCHEMA_VERSION,
};
use serde::Serialize;

use crate::error::CliError;
use crate::output::{self, OutputFormat, OutputSpec};
use crate::run::runs_root;

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
#[clap(rename_all = "snake_case")]
pub enum StateArg {
    AgentActive,
    ToolRunning,
    Settled,
    Shutdown,
}

impl From<StateArg> for TelemetryState {
    fn from(value: StateArg) -> Self {
        match value {
            StateArg::AgentActive => Self::AgentActive,
            StateArg::ToolRunning => Self::ToolRunning,
            StateArg::Settled => Self::Settled,
            StateArg::Shutdown => Self::Shutdown,
        }
    }
}

pub struct Args<'a> {
    pub run_id: Option<String>,
    pub node_id: Option<String>,
    pub attempt: Option<u32>,
    pub state: Option<StateArg>,
    pub active_tool_count: Option<u8>,
    pub tool_name: Option<String>,
    pub input_file: Option<PathBuf>,
    pub spec: &'a OutputSpec,
    pub warnings: &'a [String],
}

#[derive(Serialize)]
struct UpdatePayload {
    accepted: bool,
    run_id: octl_core::RunId,
    node_id: octl_core::NodeId,
    attempt: u32,
    received_at: chrono::DateTime<chrono::Utc>,
    expires_at: chrono::DateTime<chrono::Utc>,
}

pub fn update(args: Args<'_>) -> Result<(), CliError> {
    let request = if let Some(path) = args.input_file.as_deref() {
        let bytes = read_bounded(path)?;
        parse_telemetry_update(&bytes).map_err(from_telemetry)?
    } else {
        flags_request(&args)?
    };

    let root = crate::home::root_dir()?;
    let paths = RunPaths::new(
        runs_root(&root).join(request.run_id.as_str()),
        request.run_id.as_str(),
    )
    .map_err(crate::run::from_core)?;
    let accepted = update_telemetry(&paths, &request).map_err(from_telemetry)?;
    let payload = UpdatePayload {
        accepted: accepted.accepted,
        run_id: accepted.run_id,
        node_id: accepted.node_id,
        attempt: accepted.attempt,
        received_at: accepted.received_at,
        expires_at: accepted.expires_at,
    };
    match args.spec.format {
        OutputFormat::Json | OutputFormat::Jsonl => {
            output::emit_envelope(&payload, args.spec, args.warnings)?;
        }
        OutputFormat::Text => {
            println!("accepted:      true");
            println!("run-id:        {}", payload.run_id);
            println!("node-id:       {}", payload.node_id);
            println!("attempt:       {}", payload.attempt);
            println!("received-at:   {}", payload.received_at);
            println!("expires-at:    {}", payload.expires_at);
            println!("note:          advisory telemetry updated; run status unchanged");
            output::emit_text_warnings(args.warnings);
        }
    }
    Ok(())
}

fn flags_request(args: &Args<'_>) -> Result<TelemetryUpdate, CliError> {
    let run_id = args
        .run_id
        .as_deref()
        .ok_or_else(|| {
            CliError::user(
                "invalid_arguments",
                "--run-id, --node-id, --attempt, and --state are required without --input-file",
            )
        })?
        .parse()
        .map_err(|error| {
            CliError::user("invalid_run_id", format!("invalid --run-id: {error}"))
                .with_invalid_value(args.run_id.as_deref().unwrap_or_default())
        })?;
    let node_id = args
        .node_id
        .as_deref()
        .ok_or_else(|| {
            CliError::user(
                "invalid_arguments",
                "--run-id, --node-id, --attempt, and --state are required without --input-file",
            )
        })?
        .parse()
        .map_err(|error| {
            CliError::user("invalid_node_id", format!("invalid --node-id: {error}"))
                .with_invalid_value(args.node_id.as_deref().unwrap_or_default())
        })?;
    let request = TelemetryUpdate {
        schema_version: TELEMETRY_SCHEMA_VERSION,
        protocol_version: TELEMETRY_PROTOCOL_VERSION,
        run_id,
        node_id,
        attempt: args.attempt.ok_or_else(|| {
            CliError::user(
                "invalid_arguments",
                "--run-id, --node-id, --attempt, and --state are required without --input-file",
            )
        })?,
        state: args
            .state
            .ok_or_else(|| {
                CliError::user(
                    "invalid_arguments",
                    "--run-id, --node-id, --attempt, and --state are required without --input-file",
                )
            })?
            .into(),
        active_tool_count: args.active_tool_count,
        tool_name: args.tool_name.clone(),
    };
    // The core update path revalidates metadata and the normalized 4 KiB bound.
    Ok(request)
}

fn read_bounded(path: &Path) -> Result<Vec<u8>, CliError> {
    let mut bytes = Vec::new();
    let limit = (TELEMETRY_MAX_BYTES as u64) + 1;
    if path == Path::new("-") {
        std::io::stdin()
            .lock()
            .take(limit)
            .read_to_end(&mut bytes)
            .map_err(|error| CliError::system("io_error", format!("read stdin: {error}")))?;
    } else {
        let file = std::fs::File::open(path).map_err(|error| {
            CliError::user(
                "telemetry_input_unreadable",
                format!("open {}: {error}", path.display()),
            )
            .with_invalid_value(path.display().to_string())
        })?;
        file.take(limit).read_to_end(&mut bytes).map_err(|error| {
            CliError::user(
                "telemetry_input_unreadable",
                format!("read {}: {error}", path.display()),
            )
            .with_invalid_value(path.display().to_string())
        })?;
    }
    if bytes.len() > TELEMETRY_MAX_BYTES {
        return Err(CliError::user(
            "telemetry_input_too_large",
            format!(
                "telemetry input exceeds {TELEMETRY_MAX_BYTES} bytes (read at least {})",
                bytes.len()
            ),
        )
        .with_invalid_value(bytes.len().to_string())
        .with_expected(serde_json::json!({"maximum_bytes": TELEMETRY_MAX_BYTES})));
    }
    Ok(bytes)
}

fn from_telemetry(error: TelemetryError) -> CliError {
    match error {
        TelemetryError::Core(error) => crate::run::from_core(error),
        TelemetryError::InvalidRequest(error) => CliError::user(
            "invalid_telemetry_request",
            format!("invalid strict telemetry JSON: {error}"),
        ),
        TelemetryError::TooLarge { bytes, .. } => CliError::user(
            "telemetry_input_too_large",
            format!("telemetry input exceeds {TELEMETRY_MAX_BYTES} bytes (got {bytes})"),
        )
        .with_invalid_value(bytes.to_string())
        .with_expected(serde_json::json!({"maximum_bytes": TELEMETRY_MAX_BYTES})),
        TelemetryError::UnsupportedSchema { found } => CliError::user(
            "unsupported_telemetry_schema",
            format!("unsupported telemetry schema_version {found}"),
        )
        .with_invalid_value(found.to_string())
        .with_expected(serde_json::json!([TELEMETRY_SCHEMA_VERSION])),
        TelemetryError::UnsupportedProtocol { found } => CliError::user(
            "unsupported_telemetry_protocol",
            format!("unsupported telemetry protocol_version {found}"),
        )
        .with_invalid_value(found.to_string())
        .with_expected(serde_json::json!([TELEMETRY_PROTOCOL_VERSION])),
        TelemetryError::InvalidMetadata(message) => CliError::user(
            "invalid_telemetry_metadata",
            format!("invalid telemetry tool metadata: {message}"),
        ),
        TelemetryError::RunMismatch { expected, found } => CliError::user(
            "telemetry_run_mismatch",
            format!("telemetry run_id {found} does not match run {expected}"),
        )
        .with_invalid_value(found.to_string())
        .with_expected(serde_json::Value::String(expected.to_string())),
        TelemetryError::RunStateNotCurrent => CliError::system(
            "telemetry_state_not_current",
            "canonical run state is not synchronized; telemetry was not updated",
        ),
        TelemetryError::NodeNotFound { node_id } => {
            CliError::user("node_not_found", format!("no node {node_id} in this run"))
                .with_invalid_value(node_id.to_string())
        }
        TelemetryError::TerminalNode { node_id, status } => CliError::user(
            "telemetry_node_terminal",
            format!("node {node_id} is terminal ({status:?}); telemetry was not updated"),
        )
        .with_invalid_value(node_id.to_string()),
        TelemetryError::AttemptMismatch { expected, found } => CliError::user(
            "telemetry_attempt_mismatch",
            format!("telemetry attempt {found} does not match current attempt {expected}"),
        )
        .with_invalid_value(found.to_string())
        .with_expected(serde_json::Value::from(expected)),
        TelemetryError::ClockOverflow => CliError::system(
            "telemetry_clock_overflow",
            "server clock cannot represent telemetry expiry",
        ),
    }
}
