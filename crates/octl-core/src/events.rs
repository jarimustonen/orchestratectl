//! Event append primitive + `seq` recovery (design.md §1.4, §4).

use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;

use chrono::Utc;
use serde::Deserialize;
use serde_json::Value;

use crate::atomic::open_events_append;
use crate::error::{Error, Result};
use crate::lock::RunLock;
use crate::paths::RunPaths;
use crate::reducer::apply_event;
use crate::schema::Event;

/// Backward-scan chunk size when looking for the previous newline.
const SCAN_CHUNK: u64 = 64 * 1024;

/// Read the last `seq` from `events.jsonl`, or `0` if empty/missing.
///
/// Tolerates:
/// - lines larger than any fixed buffer (`node.report` payloads can be 10s of KB
///   per `design.md` §1.4) — we scan backwards in chunks for the previous `\n`.
/// - a crash-truncated final line lacking a trailing `\n` — that partial tail
///   is discarded and recovery uses the last complete record.
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

    // Require a newline-terminated final line; otherwise treat the last
    // partial chunk as torn and recover from the previous complete line.
    let mut tail_byte = [0u8; 1];
    f.seek(SeekFrom::End(-1))
        .map_err(|e| Error::io(events_path, e))?;
    f.read_exact(&mut tail_byte)
        .map_err(|e| Error::io(events_path, e))?;
    let mut end = if tail_byte[0] == b'\n' {
        len - 1
    } else {
        match find_prev_newline(&mut f, len, events_path)? {
            Some(p) => p,
            None => return Ok(0),
        }
    };

    // `end` is the byte index of the trailing `\n` of the last complete
    // record (exclusive of the newline). Find the previous newline to
    // bracket the line.
    let line_start = match find_prev_newline(&mut f, end, events_path)? {
        Some(p) => p + 1,
        None => 0,
    };
    let line_len = end - line_start;
    f.seek(SeekFrom::Start(line_start))
        .map_err(|e| Error::io(events_path, e))?;
    let mut line = vec![0u8; line_len as usize];
    f.read_exact(&mut line)
        .map_err(|e| Error::io(events_path, e))?;
    // Strip trailing CR if present (defensive).
    if line.last() == Some(&b'\r') {
        line.pop();
    }
    if line.is_empty() {
        // Two newlines in a row (or trailing-only) — treat as "no event".
        // Recurse downward by recovering from the next earlier line.
        if line_start == 0 {
            return Ok(0);
        }
        end = line_start - 1;
        let prev = match find_prev_newline(&mut f, end, events_path)? {
            Some(p) => p + 1,
            None => 0,
        };
        let plen = end - prev;
        f.seek(SeekFrom::Start(prev))
            .map_err(|e| Error::io(events_path, e))?;
        let mut prev_line = vec![0u8; plen as usize];
        f.read_exact(&mut prev_line)
            .map_err(|e| Error::io(events_path, e))?;
        return parse_seq(&prev_line, events_path);
    }
    parse_seq(&line, events_path)
}

fn parse_seq(line: &[u8], events_path: &Path) -> Result<u64> {
    let v: Value = serde_json::from_slice(line).map_err(|e| Error::json(events_path, e))?;
    v.get("seq")
        .and_then(Value::as_u64)
        .ok_or_else(|| Error::CorruptEventLog {
            path: events_path.to_path_buf(),
            reason: "last complete line missing integer `seq` field".into(),
        })
}

/// Find the byte offset of the last `\n` strictly before `before`. Returns
/// `None` if no newline exists in `[0, before)`.
fn find_prev_newline(
    f: &mut std::fs::File,
    before: u64,
    events_path: &Path,
) -> Result<Option<u64>> {
    if before == 0 {
        return Ok(None);
    }
    let mut pos = before;
    loop {
        let start = pos.saturating_sub(SCAN_CHUNK);
        let len = pos - start;
        f.seek(SeekFrom::Start(start))
            .map_err(|e| Error::io(events_path, e))?;
        let mut buf = vec![0u8; len as usize];
        f.read_exact(&mut buf)
            .map_err(|e| Error::io(events_path, e))?;
        if let Some(i) = buf.iter().rposition(|b| *b == b'\n') {
            return Ok(Some(start + i as u64));
        }
        if start == 0 {
            return Ok(None);
        }
        pos = start;
    }
}

/// Append one event under the run's `flock`. Returns the assigned `seq`.
///
/// `seq` is recovered from the last line of `events.jsonl` on every call.
/// Long-lived supervisors that hold their own cached counter should use
/// [`append_event_with_seq`] under their own `flock`-held scope instead.
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
        write_event_line(paths, seq, kind, node_id, idempotency_key, data)?;
        Ok(seq)
    })
}

/// Append one event with a caller-supplied `seq`. The caller **must** hold
/// the run's [`RunLock`] for the duration of this call and is responsible
/// for ensuring `seq` is monotonic. Misuse can corrupt the event log.
///
/// Long-lived supervisors that cache `next_seq` in memory use this path;
/// short-lived callers should prefer [`append_event`].
pub fn append_event_with_seq(
    paths: &RunPaths,
    seq: u64,
    kind: &str,
    node_id: Option<&str>,
    idempotency_key: Option<&str>,
    data: Value,
) -> Result<()> {
    write_event_line(paths, seq, kind, node_id, idempotency_key, data)
}

fn write_event_line(
    paths: &RunPaths,
    seq: u64,
    kind: &str,
    node_id: Option<&str>,
    idempotency_key: Option<&str>,
    data: Value,
) -> Result<()> {
    let ev = Event {
        ts: Utc::now(),
        seq,
        kind: kind.to_string(),
        run_id: paths.run_id.clone(),
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
}

/// Append one event under the run's `flock` *and* fold it into the
/// projection files via [`apply_event`]. Returns the assigned `seq`.
///
/// This is the canonical mutate path for short-lived CLI callers: every
/// `events.jsonl` line is immediately reflected in `manifest.json` /
/// `nodes/*.json` / `discussions/*.json` / `spinoffs/*.json` under one
/// lock, so a read CLI run a millisecond later never sees a stale
/// projection.
pub fn append_and_apply(
    paths: &RunPaths,
    kind: &str,
    node_id: Option<&str>,
    idempotency_key: Option<&str>,
    data: Value,
) -> Result<u64> {
    RunLock::with_lock(&paths.lock(), || {
        append_and_apply_unlocked(paths, kind, node_id, idempotency_key, data)
    })
}

/// Same as [`append_and_apply`] but assumes the caller already holds the
/// run's [`RunLock`]. Use when you need to fold extra logic (e.g. an
/// idempotency-key lookup) into the same locked critical section —
/// calling [`append_and_apply`] recursively would deadlock because
/// `flock` blocks when a second open of the lock file from the same
/// process tries to acquire `LOCK_EX`.
pub fn append_and_apply_unlocked(
    paths: &RunPaths,
    kind: &str,
    node_id: Option<&str>,
    idempotency_key: Option<&str>,
    data: Value,
) -> Result<u64> {
    let last = recover_last_seq(&paths.events())?;
    let seq = last + 1;
    let ev = Event {
        ts: Utc::now(),
        seq,
        kind: kind.to_string(),
        run_id: paths.run_id.clone(),
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
    apply_event(paths, &ev)?;
    Ok(seq)
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

/// A prior event located by [`find_prior_with_key`]. Carries enough to let
/// an idempotent-retry caller both return the recorded `seq` and verify the
/// retry payload matches what was originally written.
#[derive(Debug, Clone)]
pub struct PriorEvent {
    /// The recorded `seq` of the matching event.
    pub seq: u64,
    /// The event's top-level `node_id`, if any.
    pub node_id: Option<String>,
    /// The event's `data` payload.
    pub data: Value,
}

/// Fields skimmed from every line to test for a match without ever
/// deserializing the (potentially large) `data` payload.
#[derive(Deserialize)]
struct ProbeFields {
    seq: u64,
    kind: String,
    idempotency_key: Option<String>,
}

/// Fields pulled from the one matching line, including the full payload.
#[derive(Deserialize)]
struct FullEventForReplay {
    seq: u64,
    node_id: Option<String>,
    data: Value,
}

/// Maximum number of characters from a malformed line to surface in a
/// [`Error::CorruptEventLog`] reason.
const CORRUPT_LINE_EXCERPT: usize = 100;

/// Stream-scan `events.jsonl` for the first event with matching `kind` and
/// `idempotency_key`, returning a typed [`PriorEvent`] (or `None` when the
/// log is missing or holds no such event).
///
/// Only `kind` / `idempotency_key` / `seq` are deserialized while skimming,
/// so the cost stays bounded in log size rather than log size × payload
/// size; the full `node_id` + `data` payload is parsed only for the one
/// matching line.
///
/// # Torn-line policy
///
/// [`recover_last_seq`] tolerates a crash-truncated *final* line that lacks
/// a trailing newline. This scanner mirrors exactly that contract: a final
/// line with no trailing `\n` that fails to parse is treated as an
/// in-flight partial write and ignored. Any *interior* line that fails to
/// parse (it is newline-terminated, so a later line follows) is a
/// data-integrity fault — by the time this runs, `recover_last_seq` has
/// already accepted the same file under the same lock — so it returns
/// [`Error::CorruptEventLog`] rather than silently skipping a line that
/// might carry the very key being looked up, which would let the caller
/// double-append.
///
/// Caller must hold the run's [`RunLock`]; the scan is read-only over an
/// append-only file.
pub fn find_prior_with_key(
    paths: &RunPaths,
    kind: &str,
    idempotency_key: &str,
) -> Result<Option<PriorEvent>> {
    let events_path = paths.events();
    let f = match std::fs::File::open(&events_path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(Error::io(&events_path, e)),
    };
    let mut reader = BufReader::new(f);
    let mut buf = String::new();
    let mut lineno: u64 = 0;
    // `seq` of the last successfully-parsed line, for best-effort error
    // context pointing at where corruption begins.
    let mut last_good_seq: u64 = 0;
    loop {
        buf.clear();
        // `read_line` preserves the trailing `\n`, which `lines()` strips —
        // that byte is exactly what distinguishes a torn final line from a
        // newline-terminated interior line.
        let n = reader
            .read_line(&mut buf)
            .map_err(|e| Error::io(&events_path, e))?;
        if n == 0 {
            break;
        }
        lineno += 1;
        let had_newline = buf.ends_with('\n');
        let line = buf.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            continue;
        }
        let probe: ProbeFields = match serde_json::from_str(line) {
            Ok(p) => p,
            // Torn final line (no trailing newline): a crash mid-append.
            // EOF follows on the next `read_line`, so just stop.
            Err(_) if !had_newline => break,
            // Newline-terminated line that won't parse: interior corruption.
            Err(e) => {
                return Err(Error::CorruptEventLog {
                    path: events_path.clone(),
                    reason: format!(
                        "line {lineno} is not valid JSON (last good seq {last_good_seq}): \
                         {} [{e}]",
                        excerpt(line),
                    ),
                });
            }
        };
        last_good_seq = probe.seq;
        if probe.kind != kind || probe.idempotency_key.as_deref() != Some(idempotency_key) {
            continue;
        }
        let full: FullEventForReplay =
            serde_json::from_str(line).map_err(|e| Error::json(&events_path, e))?;
        return Ok(Some(PriorEvent {
            seq: full.seq,
            node_id: full.node_id,
            data: full.data,
        }));
    }
    Ok(None)
}

/// Truncate a log line to a bounded, char-boundary-safe prefix for
/// inclusion in an error message.
fn excerpt(line: &str) -> String {
    if line.len() <= CORRUPT_LINE_EXCERPT {
        return line.to_string();
    }
    let mut end = CORRUPT_LINE_EXCERPT;
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &line[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RunPaths;
    use tempfile::TempDir;

    #[test]
    fn envelope_run_id_comes_from_paths_not_directory_basename() {
        // The whole point of storing run_id: even when the on-disk directory
        // name disagrees with the run id (symlinked/non-canonical root, the
        // original `root.file_name()` bug), the envelope must carry the stored
        // run_id verbatim — never the basename.
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("not-a-ulid-basename");
        std::fs::create_dir_all(&dir).unwrap();
        let run_id = "01jxsnap000000000000000000";
        let paths = RunPaths::new(dir, run_id).unwrap();

        let seq = append_event(&paths, "run.status", None, None, serde_json::json!({})).unwrap();
        assert_eq!(seq, 1);

        let events = read_all_events(&paths.events()).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].run_id, run_id);
    }

    /// Build a `RunPaths` over a fresh tempdir and write `bytes` verbatim to
    /// `events.jsonl` — verbatim so a test can craft torn-line boundaries
    /// (a missing trailing `\n`) that the append path never produces.
    fn paths_with_events(tmp: &TempDir, bytes: &[u8]) -> RunPaths {
        let dir = tmp.path().join("run");
        std::fs::create_dir_all(&dir).unwrap();
        let paths = RunPaths::new(dir, "01jxsnap000000000000000000").unwrap();
        std::fs::write(paths.events(), bytes).unwrap();
        paths
    }

    #[test]
    fn find_prior_with_key_missing_log_is_none() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("run");
        std::fs::create_dir_all(&dir).unwrap();
        let paths = RunPaths::new(dir, "01jxsnap000000000000000000").unwrap();
        // No events.jsonl written at all.
        let got = find_prior_with_key(&paths, "node.report", "k1").unwrap();
        assert!(got.is_none());
    }

    #[test]
    fn find_prior_with_key_finds_the_matching_line() {
        let tmp = TempDir::new().unwrap();
        let log = concat!(
            r#"{"seq":1,"kind":"node.status","idempotency_key":"k0","node_id":"n-1","data":{}}"#,
            "\n",
            r#"{"seq":2,"kind":"node.report","idempotency_key":"k1","node_id":"n-1","data":{"ok":true}}"#,
            "\n",
        );
        let paths = paths_with_events(&tmp, log.as_bytes());
        let got = find_prior_with_key(&paths, "node.report", "k1")
            .unwrap()
            .expect("match");
        assert_eq!(got.seq, 2);
        assert_eq!(got.node_id.as_deref(), Some("n-1"));
        assert_eq!(got.data, serde_json::json!({"ok": true}));
    }

    #[test]
    fn find_prior_with_key_no_match_is_none() {
        let tmp = TempDir::new().unwrap();
        let log = concat!(
            r#"{"seq":1,"kind":"node.report","idempotency_key":"other","node_id":"n-1","data":{}}"#,
            "\n",
        );
        let paths = paths_with_events(&tmp, log.as_bytes());
        assert!(find_prior_with_key(&paths, "node.report", "k1")
            .unwrap()
            .is_none());
    }

    #[test]
    fn find_prior_with_key_tolerates_torn_final_line() {
        // A complete record, then a crash-truncated final line with NO
        // trailing newline — exactly what `recover_last_seq` tolerates.
        // The scan must still return the earlier match and never error.
        let tmp = TempDir::new().unwrap();
        let mut log = String::new();
        log.push_str(
            r#"{"seq":1,"kind":"node.report","idempotency_key":"k1","node_id":"n-1","data":{"ok":true}}"#,
        );
        log.push('\n');
        log.push_str(r#"{"seq":2,"kind":"node.rep"#); // torn mid-write, no newline
        let paths = paths_with_events(&tmp, log.as_bytes());

        let got = find_prior_with_key(&paths, "node.report", "k1")
            .unwrap()
            .expect("match before the torn tail");
        assert_eq!(got.seq, 1);

        // A torn final line with no matching key ahead of it returns None,
        // not an error.
        let tmp2 = TempDir::new().unwrap();
        let paths2 = paths_with_events(&tmp2, br#"{"seq":1,"kind":"node.rep"#);
        assert!(find_prior_with_key(&paths2, "node.report", "k1")
            .unwrap()
            .is_none());
    }

    #[test]
    fn find_prior_with_key_rejects_torn_middle_line() {
        // A newline-terminated garbage line FOLLOWED by another line: this
        // is interior corruption, not an in-flight tail. It must be a hard
        // error, never a silent skip — a skipped line could carry the very
        // key being looked up and let the caller double-append.
        let tmp = TempDir::new().unwrap();
        let log = concat!(
            r#"{"seq":1,"kind":"node.report","idempotency_key":"k0","node_id":"n-1","data":{}}"#,
            "\n",
            "{not valid json at all\n",
            r#"{"seq":3,"kind":"node.report","idempotency_key":"k1","node_id":"n-1","data":{}}"#,
            "\n",
        );
        let paths = paths_with_events(&tmp, log.as_bytes());
        let err = find_prior_with_key(&paths, "node.report", "k1").unwrap_err();
        match err {
            Error::CorruptEventLog { reason, .. } => {
                assert!(reason.contains("line 2"), "reason was: {reason}");
                assert!(reason.contains("last good seq 1"), "reason was: {reason}");
            }
            other => panic!("expected CorruptEventLog, got {other:?}"),
        }
    }
}
