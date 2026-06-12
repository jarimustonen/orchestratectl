//! Event append primitive + `seq` recovery (design.md §1.4, §4).

use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;

use chrono::Utc;
use serde_json::Value;

use crate::atomic::open_events_append;
use crate::error::{Error, Result};
use crate::lock::RunLock;
use crate::paths::RunPaths;
use crate::schema::Event;

/// Maximum bytes to scan back from EOF to recover the last `seq`.
const SEQ_RECOVERY_TAIL_BYTES: u64 = 64 * 1024;

/// Read the last `seq` from `events.jsonl`, or `0` if empty/missing.
///
/// Caller must already hold the run's [`RunLock`] for correctness against
/// concurrent appenders.
pub fn recover_last_seq(events_path: &Path) -> Result<u64> {
    let mut f = match std::fs::File::open(events_path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(Error::io(events_path, e)),
    };
    let len = f.metadata().map_err(|e| Error::io(events_path, e))?.len();
    if len == 0 {
        return Ok(0);
    }
    let read_from = len.saturating_sub(SEQ_RECOVERY_TAIL_BYTES);
    f.seek(SeekFrom::Start(read_from))
        .map_err(|e| Error::io(events_path, e))?;
    let mut buf = Vec::with_capacity((len - read_from) as usize);
    f.read_to_end(&mut buf)
        .map_err(|e| Error::io(events_path, e))?;
    let last_line = buf
        .split(|b| *b == b'\n')
        .rfind(|s| !s.is_empty())
        .ok_or_else(|| Error::CorruptEventLog {
            path: events_path.to_path_buf(),
            reason: "no newline-terminated lines in tail window".into(),
        })?;
    let v: Value = serde_json::from_slice(last_line).map_err(|e| Error::json(events_path, e))?;
    let seq = v
        .get("seq")
        .and_then(Value::as_u64)
        .ok_or_else(|| Error::CorruptEventLog {
            path: events_path.to_path_buf(),
            reason: "last line missing integer `seq` field".into(),
        })?;
    Ok(seq)
}

/// Append one event under the run's `flock`. Returns the assigned `seq`.
///
/// `seq` is recovered from the last line of `events.jsonl` on every call.
/// Long-lived supervisors that hold their own cached counter should bypass
/// this and use [`append_event_with_seq`] instead.
pub fn append_event(
    paths: &RunPaths,
    kind: &str,
    node_id: Option<&str>,
    idempotency_key: Option<&str>,
    data: Value,
) -> Result<u64> {
    RunLock::with_lock(&paths.lock(), || {
        let last = recover_last_seq(&paths.events())?;
        let seq = last + 1;
        append_event_with_seq(
            paths,
            seq,
            kind,
            node_id,
            idempotency_key,
            data,
            /* relock = */ false,
        )?;
        Ok(seq)
    })
}

/// Append one event with a caller-supplied `seq`. Acquires the run lock
/// unless `relock` is `false` (caller already holds it).
pub fn append_event_with_seq(
    paths: &RunPaths,
    seq: u64,
    kind: &str,
    node_id: Option<&str>,
    idempotency_key: Option<&str>,
    data: Value,
    relock: bool,
) -> Result<()> {
    let do_append = || -> Result<()> {
        let run_id = paths
            .root
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let ev = Event {
            ts: Utc::now(),
            seq,
            kind: kind.to_string(),
            run_id,
            node_id: node_id.map(str::to_string),
            idempotency_key: idempotency_key.map(str::to_string),
            data,
        };
        let events_path = paths.events();
        let mut line = serde_json::to_vec(&ev).map_err(|e| Error::json(events_path.clone(), e))?;
        line.push(b'\n');
        let mut f = open_events_append(&events_path)?;
        f.write_all(&line)
            .map_err(|e| Error::io(events_path.clone(), e))?;
        f.sync_all().map_err(|e| Error::io(events_path, e))?;
        Ok(())
    };
    if relock {
        RunLock::with_lock(&paths.lock(), do_append)
    } else {
        do_append()
    }
}

/// Read every event from `events.jsonl`. Used by tests and reducer replays.
pub fn read_all_events(events_path: &Path) -> Result<Vec<Event>> {
    let f = match std::fs::File::open(events_path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(Error::io(events_path, e)),
    };
    let r = BufReader::new(f);
    let mut out = Vec::new();
    for line in r.lines() {
        let line = line.map_err(|e| Error::io(events_path, e))?;
        if line.is_empty() {
            continue;
        }
        let ev: Event = serde_json::from_str(&line).map_err(|e| Error::json(events_path, e))?;
        out.push(ev);
    }
    Ok(out)
}
