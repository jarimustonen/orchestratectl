//! Bounded, advisory worker-telemetry samples.
//!
//! Telemetry deliberately lives beside the event-sourced run state rather than
//! inside it. Updates take the ordinary per-run lock only to validate the node's
//! current attempt and terminal status atomically with replacing one sample;
//! they never append an event or rewrite a manifest/node projection.

use std::io::Read;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::atomic::write_atomic;
use crate::error::Error;
use crate::lock::{RunLock, Shared};
use crate::paths::{nofollow, reject_symlink, RunPaths};
use crate::projections::read_node;
use crate::schema::{NodeId, RunId, Status};

/// Wire and stored-sample schema version supported by this build.
pub const TELEMETRY_SCHEMA_VERSION: u32 = 1;
/// Worker telemetry protocol version supported by this build.
pub const TELEMETRY_PROTOCOL_VERSION: u32 = 1;
/// Maximum raw request, normalized request, and stored-sample size.
pub const TELEMETRY_MAX_BYTES: usize = 4 * 1024;
/// How long a received sample remains current.
pub const TELEMETRY_FRESHNESS_SECS: i64 = 90;

/// Injectable source of wall-clock time used by telemetry updates and reads.
pub trait TelemetryClock {
    /// Return the current server time.
    fn now(&self) -> DateTime<Utc>;
}

/// Production telemetry clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemTelemetryClock;

impl TelemetryClock for SystemTelemetryClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// The four last-told worker activity states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TelemetryState {
    /// The harness reported an active agent turn.
    AgentActive,
    /// One or more tool executions are open.
    ToolRunning,
    /// Automatic agent work was reported settled; this is not completion.
    Settled,
    /// Session shutdown was reported; this is not completion.
    Shutdown,
}

/// Strict v1 telemetry update request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TelemetryUpdate {
    /// Request schema version; must be [`TELEMETRY_SCHEMA_VERSION`].
    pub schema_version: u32,
    /// Protocol version; must be [`TELEMETRY_PROTOCOL_VERSION`].
    pub protocol_version: u32,
    /// Exact run identity.
    pub run_id: RunId,
    /// Exact node identity.
    pub node_id: NodeId,
    /// Absolute current attempt (`Node::retry_attempts`).
    pub attempt: u32,
    /// Last-told activity.
    pub state: TelemetryState,
    /// Number of active tools, only valid for `tool_running` (1–32).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_tool_count: Option<u8>,
    /// Sanitized single active tool name, allowed only when count is exactly 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
}

/// A successfully accepted telemetry update.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TelemetryAccepted {
    /// The update was stored.
    pub accepted: bool,
    /// Run identity.
    pub run_id: RunId,
    /// Node identity.
    pub node_id: NodeId,
    /// Current attempt accepted.
    pub attempt: u32,
    /// Server receive timestamp.
    pub received_at: DateTime<Utc>,
    /// Server-computed freshness boundary.
    pub expires_at: DateTime<Utc>,
}

/// Validation or state error from a telemetry update.
#[derive(Debug, thiserror::Error)]
pub enum TelemetryError {
    /// Core run-state or filesystem operation failed.
    #[error(transparent)]
    Core(#[from] Error),
    /// Input was not strict, valid JSON for [`TelemetryUpdate`].
    #[error("invalid telemetry request: {0}")]
    InvalidRequest(serde_json::Error),
    /// Raw, normalized, or stored data exceeded the fixed bound.
    #[error("telemetry {what} exceeds {TELEMETRY_MAX_BYTES} bytes (got {bytes})")]
    TooLarge {
        /// Which representation exceeded the bound.
        what: &'static str,
        /// Observed byte count.
        bytes: usize,
    },
    /// A request used an unsupported schema version.
    #[error("unsupported telemetry schema_version {found}; expected {TELEMETRY_SCHEMA_VERSION}")]
    UnsupportedSchema {
        /// Rejected version.
        found: u32,
    },
    /// A request used an unsupported protocol version.
    #[error(
        "unsupported telemetry protocol_version {found}; expected {TELEMETRY_PROTOCOL_VERSION}"
    )]
    UnsupportedProtocol {
        /// Rejected version.
        found: u32,
    },
    /// Tool metadata did not match the activity state or sanitation rules.
    #[error("invalid telemetry tool metadata: {0}")]
    InvalidMetadata(&'static str),
    /// The request run id does not match the run directory.
    #[error("telemetry run_id {found} does not match current run {expected}")]
    RunMismatch {
        /// Run directory identity.
        expected: RunId,
        /// Request identity.
        found: RunId,
    },
    /// Canonical projections and the event log are not synchronized, so exact
    /// attempt/terminal validation cannot be proven.
    #[error("canonical run state is not synchronized with the event log")]
    RunStateNotCurrent,
    /// The named node does not exist.
    #[error("no node {node_id} in this run")]
    NodeNotFound {
        /// Missing node.
        node_id: NodeId,
    },
    /// The node is terminal and cannot accept telemetry.
    #[error("node {node_id} is terminal ({status:?})")]
    TerminalNode {
        /// Terminal node.
        node_id: NodeId,
        /// Current terminal status.
        status: Status,
    },
    /// The supplied attempt is not the exact current attempt.
    #[error("telemetry attempt {found} does not match current attempt {expected}")]
    AttemptMismatch {
        /// Current node attempt.
        expected: u32,
        /// Request attempt.
        found: u32,
    },
    /// The server clock could not represent the expiry timestamp.
    #[error("server clock cannot represent telemetry expiry")]
    ClockOverflow,
}

/// Classification of a telemetry sample for the node's current attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TelemetrySampleStatus {
    /// No sample exists, or the stored sample belongs to an older attempt.
    Absent,
    /// The sample has not reached its expiry boundary.
    Current,
    /// The sample has reached its expiry boundary.
    Stale,
    /// The read clock is behind the server receive timestamp.
    ClockUnreliable,
    /// A sample file exists but is corrupt or violates the strict stored schema.
    Invalid,
}

/// Read view of one node's advisory telemetry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TelemetryView {
    /// Freshness/corruption classification.
    pub sample: TelemetrySampleStatus,
    /// Last-told state when a valid current-attempt sample exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<TelemetryState>,
    /// Age in milliseconds, unavailable when the clock is unreliable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub age_ms: Option<i64>,
    /// Time since this state was first received, unavailable with a bad clock.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_elapsed_ms: Option<i64>,
    /// Sample attempt.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempt: Option<u32>,
    /// Bounded active-tool count.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_tool_count: Option<u8>,
    /// Sanitized single-tool name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
}

impl TelemetryView {
    fn bare(sample: TelemetrySampleStatus) -> Self {
        Self {
            sample,
            state: None,
            age_ms: None,
            state_elapsed_ms: None,
            attempt: None,
            active_tool_count: None,
            tool_name: None,
        }
    }
}

/// Strict stored sample. Private so callers cannot bypass update validation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredTelemetrySample {
    schema_version: u32,
    protocol_version: u32,
    run_id: RunId,
    node_id: NodeId,
    attempt: u32,
    state: TelemetryState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    active_tool_count: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tool_name: Option<String>,
    state_since: DateTime<Utc>,
    received_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
}

/// Parse a strict JSON request, rejecting unknown fields and every representation
/// larger than 4 KiB.
pub fn parse_telemetry_update(bytes: &[u8]) -> Result<TelemetryUpdate, TelemetryError> {
    ensure_size("request", bytes.len())?;
    let update: TelemetryUpdate =
        serde_json::from_slice(bytes).map_err(TelemetryError::InvalidRequest)?;
    validate_update_shape(&update)?;
    let normalized = serde_json::to_vec(&update).map_err(TelemetryError::InvalidRequest)?;
    ensure_size("normalized request", normalized.len())?;
    Ok(update)
}

/// Validate and atomically replace the node's one advisory sample using the
/// production clock.
pub fn update_telemetry(
    paths: &RunPaths,
    update: &TelemetryUpdate,
) -> Result<TelemetryAccepted, TelemetryError> {
    update_telemetry_with_clock(paths, update, &SystemTelemetryClock)
}

/// Validate and atomically replace the node's one advisory sample using an
/// injected clock.
pub fn update_telemetry_with_clock(
    paths: &RunPaths,
    update: &TelemetryUpdate,
    clock: &impl TelemetryClock,
) -> Result<TelemetryAccepted, TelemetryError> {
    validate_update_shape(update)?;
    let normalized = serde_json::to_vec(update).map_err(TelemetryError::InvalidRequest)?;
    ensure_size("normalized request", normalized.len())?;
    if update.run_id != paths.run_id {
        return Err(TelemetryError::RunMismatch {
            expected: paths.run_id.clone(),
            found: update.run_id.clone(),
        });
    }

    // Unlike the canonical creating lock path, telemetry must never resurrect
    // a deleted/unknown run merely by trying to validate it.
    let guard = RunLock::acquire_existing(&paths.lock())?;
    // Telemetry must neither observe stale canonical state nor heal it by
    // advancing applied_seq. Fail closed and let an ordinary event writer own
    // crash-tail recovery.
    if !canonical_projections_current(paths)? {
        return Err(TelemetryError::RunStateNotCurrent);
    }
    let node = match crate::read_node_opt(paths, &update.node_id)? {
        Some(node) => node,
        None => {
            return Err(TelemetryError::NodeNotFound {
                node_id: update.node_id.clone(),
            })
        }
    };
    if node.status.is_terminal() {
        return Err(TelemetryError::TerminalNode {
            node_id: node.node_id,
            status: node.status,
        });
    }
    if update.attempt != node.retry_attempts {
        return Err(TelemetryError::AttemptMismatch {
            expected: node.retry_attempts,
            found: update.attempt,
        });
    }

    let received_at = clock.now();
    let expires_at = received_at
        .checked_add_signed(Duration::seconds(TELEMETRY_FRESHNESS_SECS))
        .ok_or(TelemetryError::ClockOverflow)?;
    let path = checked_telemetry_file(paths, &update.node_id)?;
    let prior = match read_stored(&path)? {
        StoredRead::Valid(sample) => Some(sample),
        StoredRead::Absent | StoredRead::Corrupt => None,
    };
    let state_since = prior
        .filter(|sample| {
            valid_stored_shape(sample, paths, &update.node_id)
                && sample.attempt == update.attempt
                && sample.state == update.state
        })
        .map_or(received_at, |sample| sample.state_since);
    let sample = StoredTelemetrySample {
        schema_version: TELEMETRY_SCHEMA_VERSION,
        protocol_version: TELEMETRY_PROTOCOL_VERSION,
        run_id: update.run_id.clone(),
        node_id: update.node_id.clone(),
        attempt: update.attempt,
        state: update.state,
        active_tool_count: update.active_tool_count,
        tool_name: update.tool_name.clone(),
        state_since,
        received_at,
        expires_at,
    };
    let stored = serde_json::to_vec(&sample).map_err(TelemetryError::InvalidRequest)?;
    ensure_size("stored sample", stored.len())?;
    write_atomic(&path, &stored)?;
    drop(guard);

    Ok(TelemetryAccepted {
        accepted: true,
        run_id: update.run_id.clone(),
        node_id: update.node_id.clone(),
        attempt: update.attempt,
        received_at,
        expires_at,
    })
}

/// Read and classify a sample using the production clock.
pub fn read_telemetry(paths: &RunPaths, node_id: &NodeId) -> Result<TelemetryView, TelemetryError> {
    read_telemetry_with_clock(paths, node_id, &SystemTelemetryClock)
}

/// Read and classify a sample under the run's shared lock using an injected
/// clock. An old-attempt sample is intentionally rendered as absent.
pub fn read_telemetry_with_clock(
    paths: &RunPaths,
    node_id: &NodeId,
    clock: &impl TelemetryClock,
) -> Result<TelemetryView, TelemetryError> {
    let result = RunLock::<Shared>::with_shared_lock(&paths.lock(), || {
        if !canonical_projections_current(paths)? {
            return Ok(None);
        }
        read_telemetry_locked(paths, node_id, clock.now()).map(Some)
    })?;
    result.ok_or(TelemetryError::RunStateNotCurrent)
}

/// Read every projected node's telemetry under one shared lock and one
/// canonical-currency check. This is the bounded read-surface API for callers
/// that need per-run rows or counts; it avoids recursively locking and rescanning
/// the event log once per node.
///
/// A crash-tailed canonical projection cannot prove current attempts and
/// returns [`TelemetryError::RunStateNotCurrent`]. Malformed bounded sample
/// contents still classify as `invalid`; canonical projection, path-integrity,
/// and operational I/O errors propagate instead of masquerading as sample
/// corruption.
pub fn read_all_telemetry(
    paths: &RunPaths,
) -> Result<Vec<(NodeId, TelemetryView)>, TelemetryError> {
    read_all_telemetry_with_clock(paths, &SystemTelemetryClock)
}

/// [`read_all_telemetry`] with an injected read clock.
pub fn read_all_telemetry_with_clock(
    paths: &RunPaths,
    clock: &impl TelemetryClock,
) -> Result<Vec<(NodeId, TelemetryView)>, TelemetryError> {
    let result = RunLock::<Shared>::with_shared_lock(&paths.lock(), || {
        let node_ids = projected_node_ids(paths)?;
        if !canonical_projections_current(paths)? {
            return Ok(None);
        }
        let now = clock.now();
        let mut rows = Vec::with_capacity(node_ids.len());
        for node_id in node_ids {
            let view = read_telemetry_locked(paths, &node_id, now)?;
            rows.push((node_id, view));
        }
        Ok(Some(rows))
    })?;
    result.ok_or(TelemetryError::RunStateNotCurrent)
}

fn read_telemetry_locked(
    paths: &RunPaths,
    node_id: &NodeId,
    now: DateTime<Utc>,
) -> Result<TelemetryView, Error> {
    let node = read_node(paths, node_id)?;
    let path = checked_telemetry_file(paths, node_id)?;
    let sample = match read_stored(&path)? {
        StoredRead::Absent => return Ok(TelemetryView::bare(TelemetrySampleStatus::Absent)),
        StoredRead::Corrupt => return Ok(TelemetryView::bare(TelemetrySampleStatus::Invalid)),
        StoredRead::Valid(sample) => sample,
    };
    if !valid_stored_shape(&sample, paths, node_id) {
        return Ok(TelemetryView::bare(TelemetrySampleStatus::Invalid));
    }
    if sample.attempt != node.retry_attempts {
        return Ok(TelemetryView::bare(TelemetrySampleStatus::Absent));
    }
    let clock_bad = now < sample.received_at || now < sample.state_since;
    let status = if clock_bad {
        TelemetrySampleStatus::ClockUnreliable
    } else if now >= sample.expires_at {
        TelemetrySampleStatus::Stale
    } else {
        TelemetrySampleStatus::Current
    };
    Ok(TelemetryView {
        sample: status,
        state: Some(sample.state),
        age_ms: (!clock_bad).then(|| (now - sample.received_at).num_milliseconds()),
        state_elapsed_ms: (!clock_bad).then(|| (now - sample.state_since).num_milliseconds()),
        attempt: Some(sample.attempt),
        active_tool_count: sample.active_tool_count,
        tool_name: sample.tool_name,
    })
}

fn canonical_projections_current(paths: &RunPaths) -> Result<bool, Error> {
    let Some(manifest) = crate::read_manifest_opt(paths)? else {
        return Ok(false);
    };
    let events = paths.checked_events()?;
    Ok(manifest.applied_seq == crate::recover_last_seq(&events)?)
}

fn validate_update_shape(update: &TelemetryUpdate) -> Result<(), TelemetryError> {
    if update.schema_version != TELEMETRY_SCHEMA_VERSION {
        return Err(TelemetryError::UnsupportedSchema {
            found: update.schema_version,
        });
    }
    if update.protocol_version != TELEMETRY_PROTOCOL_VERSION {
        return Err(TelemetryError::UnsupportedProtocol {
            found: update.protocol_version,
        });
    }
    validate_metadata(
        update.state,
        update.active_tool_count,
        update.tool_name.as_deref(),
    )
}

fn validate_metadata(
    state: TelemetryState,
    count: Option<u8>,
    name: Option<&str>,
) -> Result<(), TelemetryError> {
    if state != TelemetryState::ToolRunning && (count.is_some() || name.is_some()) {
        return Err(TelemetryError::InvalidMetadata(
            "tool metadata is allowed only for tool_running",
        ));
    }
    if let Some(count) = count {
        if !(1..=32).contains(&count) {
            return Err(TelemetryError::InvalidMetadata(
                "active_tool_count must be between 1 and 32",
            ));
        }
    }
    if let Some(name) = name {
        if count != Some(1) {
            return Err(TelemetryError::InvalidMetadata(
                "tool_name requires active_tool_count=1",
            ));
        }
        if !valid_tool_name(name) {
            return Err(TelemetryError::InvalidMetadata(
                "tool_name must match ^[A-Za-z0-9_.:-]{1,64}$",
            ));
        }
    }
    Ok(())
}

fn valid_tool_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b':' | b'-'))
}

fn valid_stored_shape(sample: &StoredTelemetrySample, paths: &RunPaths, node_id: &NodeId) -> bool {
    sample.schema_version == TELEMETRY_SCHEMA_VERSION
        && sample.protocol_version == TELEMETRY_PROTOCOL_VERSION
        && sample.run_id == paths.run_id
        && sample.node_id == *node_id
        && sample
            .received_at
            .checked_add_signed(Duration::seconds(TELEMETRY_FRESHNESS_SECS))
            .is_some_and(|expected| sample.expires_at == expected)
        && validate_metadata(
            sample.state,
            sample.active_tool_count,
            sample.tool_name.as_deref(),
        )
        .is_ok()
}

fn ensure_size(what: &'static str, bytes: usize) -> Result<(), TelemetryError> {
    if bytes <= TELEMETRY_MAX_BYTES {
        Ok(())
    } else {
        Err(TelemetryError::TooLarge { what, bytes })
    }
}

fn projected_node_ids(paths: &RunPaths) -> Result<Vec<NodeId>, Error> {
    paths.guard_root()?;
    let dir = paths.nodes_dir();
    reject_symlink(&dir, || Error::SymlinkSubdir {
        name: "nodes",
        path: dir.clone(),
    })?;
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(Error::io(&dir, error)),
    };
    let mut ids = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| Error::io(&dir, error))?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
            continue;
        };
        if let Ok(node_id) = NodeId::parse_str(stem) {
            ids.push(node_id);
        }
    }
    ids.sort_by(|left, right| left.as_str().cmp(right.as_str()));
    Ok(ids)
}

fn checked_telemetry_file(paths: &RunPaths, node_id: &NodeId) -> Result<PathBuf, Error> {
    paths.guard_root()?;
    let dir = paths.root.join("telemetry");
    reject_symlink(&dir, || Error::SymlinkSubdir {
        name: "telemetry",
        path: dir.clone(),
    })?;
    let path = dir.join(format!("{}.json", node_id.as_str()));
    reject_symlink(&path, || Error::SymlinkStateFile {
        name: "telemetry",
        path: path.clone(),
    })?;
    Ok(path)
}

enum StoredRead {
    Absent,
    Corrupt,
    Valid(StoredTelemetrySample),
}

fn read_stored(path: &Path) -> Result<StoredRead, Error> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    nofollow(&mut options);
    let mut file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(StoredRead::Absent)
        }
        Err(error) => return Err(Error::io(path, error)),
    };
    let mut bytes = Vec::new();
    file.by_ref()
        .take((TELEMETRY_MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| Error::io(path, error))?;
    if bytes.len() > TELEMETRY_MAX_BYTES {
        return Ok(StoredRead::Corrupt);
    }
    Ok(match serde_json::from_slice(&bytes) {
        Ok(sample) => StoredRead::Valid(sample),
        Err(_) => StoredRead::Corrupt,
    })
}
