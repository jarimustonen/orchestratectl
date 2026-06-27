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
    // A newline-terminated complete line that won't parse is event-log
    // corruption, not a transient JSON fault — classify it the same way
    // `find_prior_with_key` does so both readers map to `CorruptEventLog`
    // (and the CLI's `corrupt-event-log` exit class) for the same condition.
    let v: Value = serde_json::from_slice(line).map_err(|e| Error::CorruptEventLog {
        path: events_path.to_path_buf(),
        reason: format!(
            "last complete line is not valid JSON: {} [{e}]",
            excerpt(line)
        ),
    })?;
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
#[derive(Debug, Clone, PartialEq)]
pub struct PriorEvent {
    /// The recorded `seq` of the matching event.
    pub seq: u64,
    /// The event's top-level `node_id`, if any.
    pub node_id: Option<String>,
    /// The event's `data` payload.
    pub data: Value,
}

/// Fields skimmed from every line to test for a match without ever
/// allocating the (potentially large) `data` payload. `seq` is optional and
/// used only for best-effort error context — it is never a match key, so a
/// line missing it must not change whether a `kind` + `idempotency_key`
/// match is found.
#[derive(Deserialize)]
struct ProbeFields {
    #[serde(default)]
    seq: Option<u64>,
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

/// Maximum number of bytes from a malformed line to surface (escaped) in an
/// [`Error::CorruptEventLog`] reason.
const CORRUPT_LINE_EXCERPT_BYTES: usize = 100;

/// Stream-scan `events.jsonl` for the first event with matching `kind` and
/// `idempotency_key`, returning a typed [`PriorEvent`] (or `None` when the
/// log is missing or holds no such event).
///
/// The skim parses each line's envelope (`kind` / `idempotency_key` / `seq`)
/// but never materializes `data` for non-matching lines; the full payload
/// (`node_id` plus `data`) is deserialized only for the one matching line.
/// JSON parsing still scans every byte of every line, so the scan is linear
/// in total log bytes under the lock — there is no payload-skipping shortcut.
///
/// # Torn-line policy
///
/// [`recover_last_seq`] tolerates a crash-truncated *final* line that lacks
/// a trailing newline and discards it regardless of whether its bytes
/// happen to form valid JSON. This scanner mirrors that exactly: a final
/// line with no trailing `\n` is treated as an in-flight partial write and
/// ignored — *before* any parse attempt — so the read (dedup) and write
/// (recovery) paths never disagree about whether that tail is committed.
///
/// Any *interior* line that fails to parse (it is newline-terminated, so a
/// later line follows) is a data-integrity fault, so it returns
/// [`Error::CorruptEventLog`] rather than silently skipping a line that
/// might carry the very key being looked up, which would let the caller
/// double-append. This is strictly *more* conservative than
/// `recover_last_seq` (which only inspects the last complete line) — a
/// deliberate choice for the dedup read.
///
/// Bytes are read with [`std::io::BufRead::read_until`] rather than
/// `read_line` so a torn tail that cuts a multi-byte UTF-8 sequence is
/// tolerated as a partial write (matching `recover_last_seq`) instead of
/// surfacing as an I/O error; a *newline-terminated* line containing
/// invalid UTF-8 is reported as `CorruptEventLog`, not I/O.
///
/// Caller must hold the run's [`RunLock`]; the scan is read-only over an
/// append-only file. The contract is documentation-only — a future
/// higher-level mutation API (tracked as `core-append-and-apply-api`) is
/// expected to fold this into a lock-holding type.
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
    let mut buf: Vec<u8> = Vec::new();
    let mut lineno: u64 = 0;
    // `seq` of the last successfully-parsed line, for best-effort error
    // context pointing at where corruption begins.
    let mut last_good_seq: u64 = 0;
    loop {
        buf.clear();
        // `read_until(b'\n')` keeps the trailing `\n` (which `lines()`
        // strips) — that byte distinguishes a torn final line from a
        // newline-terminated interior line — and reads raw bytes so a torn
        // tail with partial UTF-8 doesn't become an I/O error.
        let n = reader
            .read_until(b'\n', &mut buf)
            .map_err(|e| Error::io(&events_path, e))?;
        if n == 0 {
            break;
        }
        lineno += 1;
        let had_newline = buf.last() == Some(&b'\n');
        let line = trim_line_end(&buf);
        if line.is_empty() {
            continue;
        }
        // Mirror `recover_last_seq`: a final line lacking a trailing newline
        // is an uncommitted partial write, discarded WITHOUT parsing — even
        // if its bytes form valid JSON. Parsing it could otherwise return a
        // "match" for an event recovery considers unwritten, double-counting
        // the seq or skipping a real append.
        if !had_newline {
            break;
        }
        let probe: ProbeFields =
            serde_json::from_slice(line).map_err(|e| Error::CorruptEventLog {
                path: events_path.clone(),
                reason: format!(
                    "line {lineno} is not a valid event envelope (last good seq {last_good_seq}): \
                 {} [{e}]",
                    excerpt(line),
                ),
            })?;
        if let Some(seq) = probe.seq {
            last_good_seq = seq;
        }
        if probe.kind != kind || probe.idempotency_key.as_deref() != Some(idempotency_key) {
            continue;
        }
        let full: FullEventForReplay =
            serde_json::from_slice(line).map_err(|e| Error::CorruptEventLog {
                path: events_path.clone(),
                reason: format!(
                    "line {lineno} matched idempotency key but is not a replayable event: {} [{e}]",
                    excerpt(line),
                ),
            })?;
        return Ok(Some(PriorEvent {
            seq: full.seq,
            node_id: full.node_id,
            data: full.data,
        }));
    }
    Ok(None)
}

/// Strip a single trailing line terminator (`\n`, optionally preceded by
/// `\r`) from a raw line. Unlike `trim_end_matches`, this removes exactly
/// one terminator so interior/leading bytes are never altered.
fn trim_line_end(buf: &[u8]) -> &[u8] {
    let mut end = buf.len();
    if end > 0 && buf[end - 1] == b'\n' {
        end -= 1;
        if end > 0 && buf[end - 1] == b'\r' {
            end -= 1;
        }
    }
    &buf[..end]
}

/// Render a bounded, escaped prefix of a malformed log line for inclusion
/// in an error message. Bytes are lossily decoded (a torn multi-byte tail
/// becomes the replacement char) and control characters are escaped so an
/// excerpt can't inject newlines or ANSI sequences into CLI output.
fn excerpt(line: &[u8]) -> String {
    let shown = &line[..line.len().min(CORRUPT_LINE_EXCERPT_BYTES)];
    let mut out: String = String::from_utf8_lossy(shown).escape_debug().to_string();
    if line.len() > CORRUPT_LINE_EXCERPT_BYTES {
        out.push('…');
    }
    out
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
    fn find_prior_with_key_ignores_valid_json_final_line_without_newline() {
        // The dangerous case: a crash landed a COMPLETE, valid-JSON event
        // but the trailing newline never flushed. `recover_last_seq`
        // discards any newline-less tail, so it considers this event
        // unwritten (returns 0). The dedup scan MUST agree and return None
        // — otherwise it would report "already appended", the caller skips
        // the append, and the event is lost / the seq double-counts.
        let tmp = TempDir::new().unwrap();
        let line =
            br#"{"seq":1,"kind":"node.report","idempotency_key":"k1","node_id":"n-1","data":{}}"#;
        let paths = paths_with_events(&tmp, line);
        assert_eq!(recover_last_seq(&paths.events()).unwrap(), 0);
        assert!(
            find_prior_with_key(&paths, "node.report", "k1")
                .unwrap()
                .is_none(),
            "torn tail must be ignored even when it parses as valid JSON"
        );
    }

    #[test]
    fn find_prior_with_key_skips_nonmatching_line_missing_seq() {
        // `seq` is not a match key, so a NON-matching envelope that happens
        // to lack `seq` must be skimmed past, not treated as corruption that
        // aborts the scan before a later match. (The pre-lift scanner's
        // probe didn't require `seq`; making it required would have been a
        // regression that hid a real key behind an unrelated seq-less line.)
        let tmp = TempDir::new().unwrap();
        let log = concat!(
            r#"{"kind":"node.status","idempotency_key":"other","node_id":"n-1","data":{}}"#,
            "\n",
            r#"{"seq":2,"kind":"node.report","idempotency_key":"k1","node_id":"n-1","data":{"ok":true}}"#,
            "\n",
        );
        let paths = paths_with_events(&tmp, log.as_bytes());
        let got = find_prior_with_key(&paths, "node.report", "k1")
            .unwrap()
            .expect("match after a seq-less non-matching line");
        assert_eq!(got.seq, 2);
        assert_eq!(got.node_id.as_deref(), Some("n-1"));
    }

    #[test]
    fn find_prior_with_key_matched_line_bad_payload_is_corrupt_log() {
        // A line that skims fine (kind + key match) but whose full payload
        // is malformed (`node_id` is a number, not a string) is event-log
        // corruption — it must surface as CorruptEventLog (exit 1), not a
        // generic JSON/io error (exit 2).
        let tmp = TempDir::new().unwrap();
        let log = concat!(
            r#"{"seq":1,"kind":"node.report","idempotency_key":"k1","node_id":42,"data":{}}"#,
            "\n",
        );
        let paths = paths_with_events(&tmp, log.as_bytes());
        let err = find_prior_with_key(&paths, "node.report", "k1").unwrap_err();
        assert!(
            matches!(err, Error::CorruptEventLog { .. }),
            "expected CorruptEventLog, got {err:?}"
        );
    }

    #[test]
    fn find_prior_with_key_handles_crlf_line_endings() {
        let tmp = TempDir::new().unwrap();
        let log = concat!(
            r#"{"seq":1,"kind":"node.report","idempotency_key":"k1","node_id":"n-1","data":{}}"#,
            "\r\n",
        );
        let paths = paths_with_events(&tmp, log.as_bytes());
        let got = find_prior_with_key(&paths, "node.report", "k1")
            .unwrap()
            .expect("CRLF-terminated match");
        assert_eq!(got.seq, 1);
    }

    #[test]
    fn find_prior_with_key_tolerates_partial_utf8_torn_tail() {
        // A crash can cut a multi-byte UTF-8 sequence mid-character. With
        // byte-oriented reading this torn (newline-less) tail is tolerated
        // like any other partial write, not surfaced as an I/O error.
        let tmp = TempDir::new().unwrap();
        let mut log = Vec::new();
        log.extend_from_slice(
            br#"{"seq":1,"kind":"node.report","idempotency_key":"k1","node_id":"n-1","data":{}}"#,
        );
        log.push(b'\n');
        log.extend_from_slice(&[0xF0, 0x9F]); // start of a 4-byte char, truncated
        let paths = paths_with_events(&tmp, &log);
        let got = find_prior_with_key(&paths, "node.report", "k1")
            .unwrap()
            .expect("match before the partial-UTF8 tail");
        assert_eq!(got.seq, 1);
    }

    #[test]
    fn recover_last_seq_newline_terminated_garbage_is_corrupt_log() {
        // Consistency guard with find_prior_with_key: a newline-terminated
        // final line that isn't valid JSON is CorruptEventLog from BOTH
        // readers, so the CLI maps both to the same corrupt-event-log exit.
        let tmp = TempDir::new().unwrap();
        let paths = paths_with_events(&tmp, b"{not json at all\n");
        let err = recover_last_seq(&paths.events()).unwrap_err();
        assert!(
            matches!(err, Error::CorruptEventLog { .. }),
            "expected CorruptEventLog, got {err:?}"
        );
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
