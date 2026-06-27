//! Event append primitive + `seq` recovery (design.md §1.4, §4).

use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::atomic::open_events_append;
use crate::error::{Error, Result};
use crate::lock::RunLock;
use crate::paths::RunPaths;
use crate::reducer::{apply_event, validate_event};
use crate::schema::{Event, NodeId};

/// Parse the optional envelope `node_id` (`Option<&str>` from a caller) into the
/// typed `Option<NodeId>` an [`Event`] now carries. Callers are expected to pass
/// an already-validated id; a malformed one is rejected here ([`Error::InvalidNodeId`])
/// so it can never be written into `events.jsonl` as an unvalidated string.
fn parse_envelope_node_id(node_id: Option<&str>) -> Result<Option<NodeId>> {
    node_id
        .map(|s| {
            NodeId::parse_str(s).map_err(|e| Error::InvalidNodeId {
                node_id: s.to_string(),
                reason: e.to_string(),
            })
        })
        .transpose()
}

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

/// Truncate a torn (newline-less) final line off `events.jsonl` so the next
/// append never concatenates onto a partial record.
///
/// `recover_last_seq` only *ignores* a torn tail for seq purposes — it never
/// removes the bytes. Without this, an append after a crash-truncated write
/// would write its `\n`-terminated line directly onto the partial bytes,
/// producing one malformed `…torn…{"seq":…}` line that every later reader
/// (now sharing a strict torn-tail policy) hard-errors on. Cutting back to
/// the last complete record here guarantees the file is always empty or
/// `\n`-terminated before we append.
///
/// Caller must hold the run's [`RunLock`]. No-op when the file is absent,
/// empty, or already `\n`-terminated (the common, clean case — one `stat` +
/// one-byte read, no rewrite).
fn truncate_torn_tail(events_path: &Path) -> Result<()> {
    let mut f = match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(events_path)
    {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(Error::io(events_path, e)),
    };
    let len = f.metadata().map_err(|e| Error::io(events_path, e))?.len();
    if len == 0 {
        return Ok(());
    }
    let mut tail = [0u8; 1];
    f.seek(SeekFrom::End(-1))
        .map_err(|e| Error::io(events_path, e))?;
    f.read_exact(&mut tail)
        .map_err(|e| Error::io(events_path, e))?;
    if tail[0] == b'\n' {
        return Ok(());
    }
    // Torn final line: cut back to just past the last complete record's
    // trailing newline, or to empty when no complete record exists.
    let keep = match find_prev_newline(&mut f, len, events_path)? {
        Some(nl) => nl + 1,
        None => 0,
    };
    f.set_len(keep).map_err(|e| Error::io(events_path, e))?;
    f.sync_all().map_err(|e| Error::io(events_path, e))?;
    Ok(())
}

/// Append one event with a caller-supplied `seq`. The caller **must** hold
/// the run's [`RunLock`] for the duration of this call and is responsible
/// for ensuring `seq` is monotonic. Misuse can corrupt the event log.
///
/// Test-only (`#[cfg(test)]`): a raw, no-reducer, caller-managed-`seq`
/// primitive used by the crate's fixtures and the flock stress test to craft
/// event logs with explicit seqs. Production mutation goes through
/// [`append_and_apply_event`]; projection rebuild (future) replays via
/// [`crate::reducer`], so neither needs this.
#[cfg(test)]
pub(crate) fn append_event_with_seq(
    paths: &RunPaths,
    seq: u64,
    kind: &str,
    node_id: Option<&str>,
    idempotency_key: Option<&str>,
    data: Value,
) -> Result<()> {
    write_event_line(paths, seq, kind, node_id, idempotency_key, data)
}

#[cfg(test)]
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
        node_id: parse_envelope_node_id(node_id)?,
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

/// Outcome of an [`append_and_apply_event`] call.
///
/// `seq` is the value a caller surfaces to a user: the freshly appended
/// event's `seq`, or — on an idempotent replay — the `seq` of the
/// pre-existing matching event. A reducer no-op (e.g. an event dropped by
/// the terminal-state guard) is still a success at this layer: `seq` names
/// the appended event regardless of whether the reducer changed anything.
///
/// There is intentionally no `derived_event_ids` field. This API mutates
/// exactly one event; the supervisor's report consumption, which emits a
/// *batch* of derived discussion/spinoff events under one held lock, uses
/// [`append_and_apply_unlocked`] instead (the sanctioned lock-held
/// composition path) and tracks its own emitted ids.
#[derive(Debug, Serialize)]
pub struct AppendResult {
    /// `seq` of the appended event, or of the prior event on an idempotent
    /// replay.
    pub seq: u64,
    /// True when `idempotency_key` matched a prior event so nothing new was
    /// appended or applied; `seq`/`prior` then describe that prior event.
    pub idempotent_replay: bool,
    /// On an idempotent replay, the prior event's recorded `node_id` and
    /// `data`, so a caller can reject a key reused with a conflicting
    /// request (Stripe-style). `None` on a fresh append.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prior: Option<PriorEvent>,
}

/// The one canonical mutation entry point: append a single event to
/// `events.jsonl` *and* fold it into the projection files via the reducer,
/// all under the run's `flock`, with idempotency-key dedup.
///
/// Every `events.jsonl` line is immediately reflected in `manifest.json` /
/// `nodes/*.json` / `discussions/*.json` / `spinoffs/*.json` under one lock,
/// so a read CLI run a millisecond later never sees a stale projection.
///
/// The append is transactional against reducer validation: the event is
/// first run through [`validate_event`](crate::reducer) under the lock, and
/// only a validating event is appended (and fsynced) and then folded by the
/// reducer. A reducer-rejected event (a `CorruptEventLog` for a malformed
/// payload) errors *before* any bytes are written, so the log never gains a
/// poison line that a future replay / `rebuild_projections` would choke on.
///
/// When `idempotency_key` is `Some` and a prior event with the same `kind` +
/// key already exists ([`find_prior_with_key`]), nothing is appended or
/// applied: the result carries the prior event's `seq`, `idempotent_replay:
/// true`, and `prior: Some(..)` so the caller can detect a key reused with a
/// conflicting payload. With `idempotency_key: None` no scan runs.
///
/// Callers that must compose several writes — or a read-modify-write
/// transaction (read a projection, decide, then append) — under one lock
/// window hold the lock themselves and use [`append_and_apply_unlocked`],
/// the sanctioned lock-held composition path. Re-entering this function
/// while already holding the lock would deadlock: `flock` blocks when a
/// second open of the lock file from the same process tries `LOCK_EX`.
pub fn append_and_apply_event(
    paths: &RunPaths,
    kind: &str,
    node_id: Option<&str>,
    idempotency_key: Option<&str>,
    data: Value,
) -> Result<AppendResult> {
    RunLock::with_lock(&paths.lock(), || {
        // Idempotency lookup + append share this one lock window so a
        // concurrent retry can't see "no prior event" and double-append.
        if let Some(key) = idempotency_key {
            if let Some(prior) = find_prior_with_key(paths, kind, key)? {
                return Ok(AppendResult {
                    seq: prior.seq,
                    idempotent_replay: true,
                    prior: Some(prior),
                });
            }
        }
        let seq = append_and_apply_unlocked(paths, kind, node_id, idempotency_key, data)?;
        Ok(AppendResult {
            seq,
            idempotent_replay: false,
            prior: None,
        })
    })
}

/// Append one event and fold it into projections, assuming the caller
/// already holds the run's [`RunLock`]. The **sanctioned lock-held
/// composition path**: use it to fold extra logic (an idempotency-key
/// lookup, a status precondition) or several writes (the supervisor's
/// derived discussion/spinoff batch) into one locked critical section.
/// Calling [`append_and_apply_event`] from within a held lock would
/// deadlock because `flock` blocks when a second open of the lock file from
/// the same process tries to acquire `LOCK_EX`.
pub fn append_and_apply_unlocked(
    paths: &RunPaths,
    kind: &str,
    node_id: Option<&str>,
    idempotency_key: Option<&str>,
    data: Value,
) -> Result<u64> {
    let events_path = paths.events();
    // Remove any crash-torn final line BEFORE recovering the seq or
    // appending, so the new record is never concatenated onto a partial one
    // and `seq` is recovered from a clean, `\n`-terminated file.
    truncate_torn_tail(&events_path)?;
    let last = recover_last_seq(&events_path)?;
    let seq = last + 1;
    let ev = Event {
        ts: Utc::now(),
        seq,
        kind: kind.to_string(),
        run_id: paths.run_id.clone(),
        node_id: parse_envelope_node_id(node_id)?,
        idempotency_key: idempotency_key.map(str::to_string),
        data,
    };
    // Transactional gate: validate against current projection state BEFORE
    // the durable append. A reducer-rejected event errors here and is never
    // written, so a later replay / rebuild can't trip on a poison line. On
    // success, `apply_event` below sees the same locked state and folds it
    // without error.
    validate_event(paths, &ev)?;
    let mut line = serde_json::to_vec(&ev).map_err(|e| Error::json(events_path.clone(), e))?;
    line.push(b'\n');
    let mut f = open_events_append(&events_path)?;
    f.write_all(&line)
        .map_err(|e| Error::io(events_path.clone(), e))?;
    f.sync_all().map_err(|e| Error::io(events_path, e))?;
    apply_event(paths, &ev)?;
    Ok(seq)
}

/// One physical line surfaced by [`PhysicalLineReader`]: its content with
/// any trailing terminator stripped, plus enough framing for the torn-tail
/// policy (whether it was newline-terminated) and for error context (byte
/// offset + 1-based line number).
struct PhysicalLine<'a> {
    /// Line content with a single trailing terminator (`\n`, optionally
    /// preceded by `\r`) removed. Interior/leading bytes are untouched.
    content: &'a [u8],
    /// `false` only for a final line lacking a trailing `\n` — a torn,
    /// in-flight append. `true` for every newline-terminated line. Because a
    /// non-terminated line can only be the last bytes in the file, this is
    /// `false` for at most one line, and only ever the last one.
    complete: bool,
    /// 1-based line number, for `CorruptEventLog` context.
    lineno: u64,
}

/// The single physical-line reader behind both [`read_all_events`] and
/// [`find_prior_with_key`], so the read paths can never disagree about the
/// torn-tail policy (design.md §1.4; torn-line-policy-consistency).
///
/// Bytes are read with [`BufRead::read_until`] (not `read_line`/`lines()`)
/// for two reasons: it keeps the trailing `\n` so a torn final line is
/// distinguishable from a newline-terminated interior one, and it reads raw
/// bytes so a torn tail that cuts a multi-byte UTF-8 sequence is tolerated as
/// a partial write rather than surfacing as an I/O error. A *newline-
/// terminated* line with invalid UTF-8 still reaches the caller's parse,
/// which classifies it as `CorruptEventLog`.
///
/// `next_line` lends a slice into an internal buffer, so a caller holds at
/// most one line at a time — the streaming (lending-iterator) pattern, which
/// keeps the per-line allocation cost to a single reused buffer.
struct PhysicalLineReader<R: BufRead> {
    reader: R,
    buf: Vec<u8>,
    lineno: u64,
    done: bool,
}

impl<R: BufRead> PhysicalLineReader<R> {
    fn new(reader: R) -> Self {
        Self {
            reader,
            buf: Vec::new(),
            lineno: 0,
            done: false,
        }
    }

    /// Yield the next physical line, or `None` at end of file. I/O errors are
    /// surfaced raw so the caller can attach the log path.
    fn next_line(&mut self) -> std::io::Result<Option<PhysicalLine<'_>>> {
        if self.done {
            return Ok(None);
        }
        self.buf.clear();
        let n = self.reader.read_until(b'\n', &mut self.buf)?;
        if n == 0 {
            self.done = true;
            return Ok(None);
        }
        self.lineno += 1;
        let complete = self.buf.last() == Some(&b'\n');
        // A non-terminated line is necessarily the final bytes of the file;
        // stop after handing it back so the torn-tail policy only ever sees
        // it last.
        if !complete {
            self.done = true;
        }
        let len = trim_line_end(&self.buf).len();
        Ok(Some(PhysicalLine {
            content: &self.buf[..len],
            complete,
            lineno: self.lineno,
        }))
    }
}

/// Read every event from `events.jsonl`. Used by tests and reducer replays.
///
/// # Torn-line policy
///
/// Built on the shared [`PhysicalLineReader`] so it matches
/// [`find_prior_with_key`] and [`recover_last_seq`] exactly: a torn final
/// line lacking a trailing `\n` is an in-flight partial write, dropped
/// *without* parsing even if its bytes happen to be valid JSON. Any
/// newline-terminated line that fails to parse is interior corruption and
/// surfaces as [`Error::CorruptEventLog`] — not a transient JSON fault — so a
/// replay rejects a poisoned log loudly instead of silently dropping a line.
pub fn read_all_events(events_path: &Path) -> Result<Vec<Event>> {
    let f = match std::fs::File::open(events_path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(Error::io(events_path, e)),
    };
    let mut reader = PhysicalLineReader::new(BufReader::new(f));
    let mut out = Vec::new();
    while let Some(line) = reader
        .next_line()
        .map_err(|e| Error::io(events_path, e))?
    {
        // Torn final line (no trailing newline): uncommitted partial write,
        // discarded without parsing — mirrors `recover_last_seq`.
        if !line.complete {
            break;
        }
        if line.content.is_empty() {
            continue;
        }
        let ev: Event =
            serde_json::from_slice(line.content).map_err(|e| Error::CorruptEventLog {
                path: events_path.to_path_buf(),
                reason: format!(
                    "line {} is not a valid event: {} [{e}]",
                    line.lineno,
                    excerpt(line.content)
                ),
            })?;
        out.push(ev);
    }
    Ok(out)
}

/// A prior event located by [`find_prior_with_key`]. Carries enough to let
/// an idempotent-retry caller both return the recorded `seq` and verify the
/// retry payload matches what was originally written.
#[derive(Debug, Clone, PartialEq, Serialize)]
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
/// append-only file. `pub(crate)`: this lock-sensitive primitive is no longer
/// a public API — [`append_and_apply_event`] folds the scan and the append
/// into one lock window so callers can't run the scan-then-append race.
pub(crate) fn find_prior_with_key(
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
    let mut reader = PhysicalLineReader::new(BufReader::new(f));
    // `seq` of the last successfully-parsed line, for best-effort error
    // context pointing at where corruption begins.
    let mut last_good_seq: u64 = 0;
    while let Some(line) = reader
        .next_line()
        .map_err(|e| Error::io(&events_path, e))?
    {
        // Mirror `recover_last_seq`: a final line lacking a trailing newline
        // is an uncommitted partial write, discarded WITHOUT parsing — even
        // if its bytes form valid JSON. Parsing it could otherwise return a
        // "match" for an event recovery considers unwritten, double-counting
        // the seq or skipping a real append.
        if !line.complete {
            break;
        }
        if line.content.is_empty() {
            continue;
        }
        let probe: ProbeFields =
            serde_json::from_slice(line.content).map_err(|e| Error::CorruptEventLog {
                path: events_path.clone(),
                reason: format!(
                    "line {} is not a valid event envelope (last good seq {last_good_seq}): \
                 {} [{e}]",
                    line.lineno,
                    excerpt(line.content),
                ),
            })?;
        if let Some(seq) = probe.seq {
            last_good_seq = seq;
        }
        if probe.kind != kind || probe.idempotency_key.as_deref() != Some(idempotency_key) {
            continue;
        }
        let full: FullEventForReplay =
            serde_json::from_slice(line.content).map_err(|e| Error::CorruptEventLog {
                path: events_path.clone(),
                reason: format!(
                    "line {} matched idempotency key but is not a replayable event: {} [{e}]",
                    line.lineno,
                    excerpt(line.content),
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

        let r = append_and_apply_event(&paths, "run.status", None, None, serde_json::json!({}))
            .unwrap();
        assert_eq!(r.seq, 1);

        let events = read_all_events(&paths.events()).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].run_id.as_str(), run_id);
    }

    /// Build a fresh, empty run directory with a valid `RunPaths` whose
    /// `run_id` matches the envelope the reducer will fold.
    fn fresh_run(tmp: &TempDir) -> RunPaths {
        let run_id = "01jxsnap000000000000000000";
        let dir = tmp.path().join(run_id);
        std::fs::create_dir_all(&dir).unwrap();
        RunPaths::new(dir, run_id).unwrap()
    }

    /// Drive a run to a live node so reducer-affecting events have a target.
    fn bootstrap_live_node(paths: &RunPaths) {
        append_and_apply_event(
            paths,
            "run.created",
            None,
            None,
            serde_json::json!({ "kind": "spinoff", "lifecycle": "autonomous", "title": "fix" }),
        )
        .unwrap();
        append_and_apply_event(
            paths,
            "node.created",
            Some("n-0001"),
            None,
            serde_json::json!({ "kind": "spinoff" }),
        )
        .unwrap();
    }

    #[test]
    fn append_and_apply_event_success_path_appends_and_folds() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);

        let r = append_and_apply_event(
            &paths,
            "run.created",
            None,
            None,
            serde_json::json!({ "kind": "spinoff", "lifecycle": "autonomous", "title": "t" }),
        )
        .unwrap();
        assert_eq!(r.seq, 1);
        assert!(!r.idempotent_replay);
        assert!(r.prior.is_none());

        // The reducer ran under the same lock: the manifest projection exists.
        let m = crate::read_manifest(&paths).unwrap();
        assert_eq!(m.run_id.as_str(), paths.run_id.as_str());
    }

    #[test]
    fn append_and_apply_event_idempotent_replay_returns_prior_without_appending() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap_live_node(&paths);

        let data = serde_json::json!({ "status": "running" });
        let first = append_and_apply_event(
            &paths,
            "node.status",
            Some("n-0001"),
            Some("k1"),
            data.clone(),
        )
        .unwrap();
        assert!(!first.idempotent_replay);
        let before = read_all_events(&paths.events()).unwrap().len();

        // Same kind + key: a replay returns the prior event and appends nothing.
        let replay = append_and_apply_event(
            &paths,
            "node.status",
            Some("n-0001"),
            Some("k1"),
            data.clone(),
        )
        .unwrap();
        assert!(replay.idempotent_replay);
        assert_eq!(replay.seq, first.seq);
        let prior = replay.prior.expect("replay carries the prior event");
        assert_eq!(prior.node_id.as_deref(), Some("n-0001"));
        assert_eq!(prior.data, data);
        assert_eq!(
            read_all_events(&paths.events()).unwrap().len(),
            before,
            "replay must not append a new line"
        );
    }

    #[test]
    fn append_and_apply_event_reducer_noop_is_still_a_success() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap_live_node(&paths);

        // Settle the node terminal.
        append_and_apply_event(
            &paths,
            "node.report",
            Some("n-0001"),
            None,
            serde_json::json!({ "success": true }),
        )
        .unwrap();
        let nid = crate::schema::NodeId::parse_str("n-0001").unwrap();
        assert_eq!(
            crate::read_node(&paths, &nid).unwrap().status,
            crate::schema::Status::Done
        );

        // A later status event is dropped by the terminal-state guard, but the
        // append still happened: the result names the appended event's seq and
        // is not a replay. The node stays Done.
        let before = read_all_events(&paths.events()).unwrap().len();
        let r = append_and_apply_event(
            &paths,
            "node.status",
            Some("n-0001"),
            None,
            serde_json::json!({ "status": "running" }),
        )
        .unwrap();
        assert!(!r.idempotent_replay);
        assert_eq!(r.seq as usize, before + 1);
        assert_eq!(
            read_all_events(&paths.events()).unwrap().len(),
            before + 1,
            "the event is appended even when the reducer no-ops"
        );
        assert_eq!(
            crate::read_node(&paths, &nid).unwrap().status,
            crate::schema::Status::Done,
            "terminal status is frozen"
        );
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
