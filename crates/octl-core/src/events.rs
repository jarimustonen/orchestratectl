//! Event append primitive + `seq` recovery (design.md §1.4, §4).

use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::atomic::{open_events_append, write_atomic};
use crate::error::{Error, Result};
use crate::lock::RunLock;
use crate::paths::RunPaths;
use crate::reducer::{commit_ops, reduce_event_to_ops};
use crate::schema::{Event, NodeId};

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
    // record. Walk backward over complete lines, skipping any that are empty
    // — consecutive newlines (e.g. from external editing) are tolerated by
    // the forward reader, so seq recovery must tolerate them too — and recover
    // the seq from the last non-empty record.
    loop {
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
        if !line.is_empty() {
            return parse_seq(&line, events_path);
        }
        // Empty line: no record here. Step to the newline before it and keep
        // scanning; reaching the start means the log holds no event.
        if line_start == 0 {
            return Ok(0);
        }
        end = line_start - 1;
    }
}

/// The envelope fields recovered from the last complete line. Required fields
/// mirror [`Event`]'s required shape, so `recover_last_seq` accepts a last line
/// iff [`read_all_events`] would — the two readers agree on what the last
/// record is. `data` / `idempotency_key` are skipped (serde ignores unknown
/// fields) so a multi-KB `node.report` payload isn't re-materialized on the
/// hot append path just to read `seq`.
#[derive(Deserialize)]
#[allow(dead_code)] // fields exist to force serde validation, not to be read
struct SeqLine {
    seq: u64,
    ts: chrono::DateTime<chrono::Utc>,
    kind: String,
    run_id: crate::schema::RunId,
    #[serde(default)]
    node_id: Option<NodeId>,
}

fn parse_seq(line: &[u8], events_path: &Path) -> Result<u64> {
    // The last complete line must be a full, valid event envelope — the same
    // bar `read_all_events` applies to every line — so a `\n`-terminated line
    // that parses as JSON but isn't a valid event (e.g. `{"seq":1}` missing
    // `ts`/`run_id`) is event-log corruption, not a usable seq source. This
    // keeps the three readers aligned on the last record.
    let hdr: SeqLine = serde_json::from_slice(line).map_err(|e| Error::CorruptEventLog {
        path: events_path.to_path_buf(),
        reason: format!(
            "last complete line is not a valid event: {} [{e}]",
            excerpt(line)
        ),
    })?;
    Ok(hdr.seq)
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
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true).write(true);
    // `O_NOFOLLOW`: refuse to rewrite the tail through a symlinked event log.
    crate::paths::nofollow(&mut opts);
    let mut f = match opts.open(events_path) {
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
    node_id: Option<&NodeId>,
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
    node_id: Option<&NodeId>,
    idempotency_key: Option<&str>,
    data: Value,
) -> Result<()> {
    let ev = Event {
        ts: Utc::now(),
        seq,
        kind: kind.to_string(),
        run_id: paths.run_id.clone(),
        node_id: node_id.cloned(),
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
/// On success, every `events.jsonl` line is folded into `manifest.json` /
/// `nodes/*.json` / `discussions/*.json` / `spinoffs/*.json` before the lock
/// is released, so a read CLI run a millisecond later never sees a stale
/// projection. This is *not* a crash-atomic transaction: the event is fsynced
/// before the reducer runs, so a crash (or an I/O error from `apply_event`)
/// after the append but before the projection write leaves the log ahead of
/// the projections — recoverable only by a future `rebuild_projections`. The
/// log is the source of truth; projections are a derived cache.
///
/// The append is transactional against reducer *validation*: the event is
/// first reduced through [`reduce_event_to_ops`](crate::reducer) under the
/// lock — the single plan-then-commit path that both validates and computes
/// the projection writes — and only a validating event is appended (and
/// fsynced) and then committed by the reducer. A reducer-rejected event (a
/// `CorruptEventLog` for a malformed payload) errors *before* any bytes are
/// written, so the log never gains a poison line that a future replay /
/// `rebuild_projections` would choke on.
/// (A pre-existing torn tail may still be truncated before validation runs —
/// those bytes are uncommitted by definition; see [`recover_last_seq`].)
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
    node_id: Option<&NodeId>,
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
    node_id: Option<&NodeId>,
    idempotency_key: Option<&str>,
    data: Value,
) -> Result<u64> {
    // Symlink containment runs once here, before truncate/recover/open all
    // reuse this path — guarding the run root and the event log itself so a
    // swapped `events.jsonl` can't redirect the run's source-of-truth write
    // outside the run tree.
    let events_path = paths.checked_events()?;
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
        node_id: node_id.cloned(),
        idempotency_key: idempotency_key.map(str::to_string),
        data,
    };
    // Transactional gate, plan-then-commit: reduce the event against current
    // projection state BEFORE the durable append. `reduce_event_to_ops` both
    // validates and computes the exact projection writes to make; a reducer-
    // rejected event errors here and is never written, so a later replay /
    // rebuild can't trip on a poison line. The planned ops are then committed
    // *after* the fsynced append — nothing mutates the projections between the
    // plan and the commit (the append only touches `events.jsonl`), so the
    // planned writes are still valid. One reduce pass serves both the gate and
    // the apply, so there is no validate/apply branch pair to drift apart.
    let ops = reduce_event_to_ops(paths, &ev)?;
    let mut line = serde_json::to_vec(&ev).map_err(|e| Error::json(events_path.clone(), e))?;
    line.push(b'\n');
    let mut f = open_events_append(&events_path)?;
    f.write_all(&line)
        .map_err(|e| Error::io(events_path.clone(), e))?;
    f.sync_all().map_err(|e| Error::io(events_path, e))?;
    commit_ops(paths, ops)?;
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
    while let Some(line) = reader.next_line().map_err(|e| Error::io(events_path, e))? {
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

/// Outcome of a [`quarantine_corrupt_lines`] call that removed at least one
/// poison line. `backup_path` is the renamed copy of the original log (kept
/// verbatim for operator forensics / hand-repair); `removed_byte_offsets`
/// are the start offsets, in that original, of every newline-terminated line
/// that failed to parse as an [`Event`] and was excised from the recovered
/// `events.jsonl`.
#[derive(Debug, Clone, Serialize)]
pub struct Quarantine {
    /// Path to the timestamped `.bak` holding the original poisoned log.
    pub backup_path: PathBuf,
    /// Byte offsets (in the original log) of every excised corrupt line.
    pub removed_byte_offsets: Vec<u64>,
}

/// Heal a poisoned `events.jsonl` by excising its corrupt physical lines.
///
/// P2 made the supervisor *skip* a corrupt JSONL line in memory and keep
/// tailing, but the bytes stayed on disk forever — so every fresh strict
/// reader ([`read_all_events`] / a future `rebuild_projections`) still
/// hard-errors on them, and the skip diagnostic is unreachable to a strict
/// replay (the corrupt line aborts the read before it). This is the durable
/// repair: under the run's [`RunLock`], the original log is renamed to
/// `events.jsonl.corrupt-<ts>.bak` and a recovered `events.jsonl` is written
/// in its place containing every line *except* the corrupt ones.
///
/// "Corrupt" means exactly what the strict readers reject: a
/// newline-terminated, non-empty line that does not parse as a full [`Event`]
/// envelope. Empty lines and a torn (newline-less) final line are retained
/// verbatim — the readers already tolerate both, so excising them would be a
/// behavior change, not a repair.
///
/// Returns `Ok(None)` when the log is missing or already clean (no rename, no
/// rewrite — the common case is cheap: one read, no corrupt line found).
/// Returns `Ok(Some(_))` with the backup path and removed offsets when at
/// least one line was excised. Caller is expected to surface the outcome
/// (e.g. a `supervisor.event_log_quarantined` diagnostic) and, for a live
/// tail, restart its read cursor at offset 0 since every byte offset shifts.
///
/// `backup_ts` is supplied by the caller (kept out of core so the rename is
/// deterministic in tests); a filename-safe basic-ISO stamp like
/// `20260628T120000Z` is the intended form.
///
/// # Operator recovery
///
/// The excised bytes are never destroyed — they survive verbatim in the
/// `events.jsonl.corrupt-<ts>.bak` sibling (named by the emitted
/// `supervisor.event_log_quarantined { backup_path }` diagnostic). To recover
/// a line the automated repair dropped: open the `.bak`, inspect the line(s)
/// at the reported `removed_byte_offsets`, hand-fix any salvageable JSON, and —
/// if you want the record back — stop the run's supervisor, append the
/// corrected line to the live `events.jsonl` (or replace the file wholesale
/// from a fixed copy of the backup), then restart the supervisor. The healed
/// log is the source of truth; projections rebuild from it.
pub fn quarantine_corrupt_lines(paths: &RunPaths, backup_ts: &str) -> Result<Option<Quarantine>> {
    RunLock::with_lock(&paths.lock(), || {
        quarantine_corrupt_lines_unlocked(paths, backup_ts)
    })
}

/// As [`quarantine_corrupt_lines`] but assumes the caller already holds the
/// run's [`RunLock`] — the sanctioned lock-held composition path, mirroring
/// [`append_and_apply_unlocked`]. Re-entering [`quarantine_corrupt_lines`]
/// under a held lock would deadlock on the second `flock` open.
pub fn quarantine_corrupt_lines_unlocked(
    paths: &RunPaths,
    backup_ts: &str,
) -> Result<Option<Quarantine>> {
    // Guard the run root + event log against symlink redirection before the
    // rename/rewrite, exactly as the append path does.
    let events_path = paths.checked_events()?;
    let raw = match std::fs::read(&events_path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(Error::io(&events_path, e)),
    };

    // Walk physical lines, keeping the raw bytes (terminator included) of every
    // retained line so the recovered file is byte-identical save for the
    // excised corruption. A line is corrupt iff it is newline-terminated,
    // non-empty, and fails the same strict `Event` parse `read_all_events`
    // applies — so the recovered log is guaranteed to pass a strict replay.
    let mut recovered: Vec<u8> = Vec::with_capacity(raw.len());
    let mut removed_byte_offsets: Vec<u64> = Vec::new();
    let mut offset: u64 = 0;
    let mut i = 0usize;
    while i < raw.len() {
        let (line_end, complete) = match raw[i..].iter().position(|b| *b == b'\n') {
            Some(p) => (i + p + 1, true), // include the trailing '\n'
            None => (raw.len(), false),   // torn final line, no '\n'
        };
        let raw_line = &raw[i..line_end];
        let content = trim_line_end(raw_line);
        let corrupt =
            complete && !content.is_empty() && serde_json::from_slice::<Event>(content).is_err();
        if corrupt {
            removed_byte_offsets.push(offset);
        } else {
            recovered.extend_from_slice(raw_line);
        }
        offset += raw_line.len() as u64;
        i = line_end;
    }

    if removed_byte_offsets.is_empty() {
        return Ok(None);
    }

    // Rename the poisoned log aside (forensics), then atomically drop the
    // recovered log in its place. Order matters: the rename frees the path for
    // `write_atomic`'s tempfile+rename and preserves the original even if the
    // rewrite then fails.
    let backup_path = backup_path_for(&events_path, backup_ts);
    std::fs::rename(&events_path, &backup_path).map_err(|e| Error::io(&backup_path, e))?;
    write_atomic(&events_path, &recovered)?;
    Ok(Some(Quarantine {
        backup_path,
        removed_byte_offsets,
    }))
}

/// Build the `events.jsonl.corrupt-<ts>.bak` sibling path for a quarantine
/// backup, preserving the original file name as a prefix.
fn backup_path_for(events_path: &Path, ts: &str) -> PathBuf {
    let mut name = events_path
        .file_name()
        .map(std::ffi::OsStr::to_os_string)
        .unwrap_or_default();
    name.push(format!(".corrupt-{ts}.bak"));
    events_path.with_file_name(name)
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
    // Guard the run root + event log before reading: the idempotency scan
    // opens `events.jsonl` ahead of the append, so it must refuse a symlinked
    // log too rather than read through it.
    let events_path = paths.checked_events()?;
    let f = match std::fs::File::open(&events_path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(Error::io(&events_path, e)),
    };
    let mut reader = PhysicalLineReader::new(BufReader::new(f));
    // `seq` of the last successfully-parsed line, for best-effort error
    // context pointing at where corruption begins.
    let mut last_good_seq: u64 = 0;
    while let Some(line) = reader.next_line().map_err(|e| Error::io(&events_path, e))? {
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
    use serde_json::json;
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

    #[cfg(unix)]
    #[test]
    fn append_rejects_a_symlinked_event_log() {
        // `events.jsonl` is the run's source of truth and highest-leverage
        // write — a symlinked log must be refused, not appended through.
        use crate::Error;
        use std::os::unix::fs::symlink;
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        let target = tmp.path().join("evil-events.jsonl");
        symlink(&target, paths.events()).unwrap();
        let err = append_and_apply_event(&paths, "run.status", None, None, json!({})).unwrap_err();
        assert!(
            matches!(err, Error::SymlinkStateFile { name: "events", .. }),
            "got {err:?}"
        );
        // The forged append never reached the symlink target.
        assert!(!target.exists());
    }

    /// Build a fresh, empty run directory with a valid `RunPaths` whose
    /// `run_id` matches the envelope the reducer will fold.
    fn fresh_run(tmp: &TempDir) -> RunPaths {
        let run_id = "01jxsnap000000000000000000";
        let dir = tmp.path().join(run_id);
        std::fs::create_dir_all(&dir).unwrap();
        RunPaths::new(dir, run_id).unwrap()
    }

    /// Parse a `NodeId` for a test append call (the typed envelope id).
    fn nid(s: &str) -> NodeId {
        NodeId::parse_str(s).unwrap()
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
            Some(&nid("n-0001")),
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
            Some(&nid("n-0001")),
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
            Some(&nid("n-0001")),
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
        let n0001 = nid("n-0001");
        append_and_apply_event(
            &paths,
            "node.report",
            Some(&n0001),
            None,
            serde_json::json!({ "success": true }),
        )
        .unwrap();
        assert_eq!(
            crate::read_node(&paths, &n0001).unwrap().status,
            crate::schema::Status::Done
        );

        // A later status event is dropped by the terminal-state guard, but the
        // append still happened: the result names the appended event's seq and
        // is not a replay. The node stays Done.
        let before = read_all_events(&paths.events()).unwrap().len();
        let r = append_and_apply_event(
            &paths,
            "node.status",
            Some(&n0001),
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
            crate::read_node(&paths, &n0001).unwrap().status,
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
    fn rejected_event_is_not_appended() {
        // The transactional fix: a reducer-rejected event must error BEFORE
        // any durable write, so events.jsonl never gains a poison line.
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap_live_node(&paths);
        let before = read_all_events(&paths.events()).unwrap().len();

        // `node.report` with neither success nor cancelled → reducer rejects.
        let err =
            append_and_apply_event(&paths, "node.report", Some(&nid("n-0001")), None, json!({}))
                .unwrap_err();
        assert!(matches!(err, Error::CorruptEventLog { .. }), "got {err:?}");

        assert_eq!(
            read_all_events(&paths.events()).unwrap().len(),
            before,
            "a rejected event must not be appended"
        );
        // The log is still clean and re-readable (no poison line stranded it).
        assert!(recover_last_seq(&paths.events()).is_ok());
        let next = append_and_apply_event(
            &paths,
            "node.report",
            Some(&nid("n-0001")),
            None,
            json!({ "success": true }),
        )
        .unwrap();
        assert_eq!(
            next.seq as usize,
            before + 1,
            "the next valid append reuses the seq the rejected event never consumed"
        );
    }

    #[test]
    fn validate_event_agrees_with_apply_event() {
        // Drift guard: `validate_event` (the pre-append gate) must return Err
        // in EXACTLY the cases `apply_event` would, for the same state — else
        // it would refuse a harmless no-op or let a poison line through.
        use crate::reducer::{apply_event, validate_event};

        fn ev(paths: &RunPaths, kind: &str, node_id: Option<&str>, data: Value) -> Event {
            Event {
                ts: Utc::now(),
                seq: 999,
                kind: kind.to_string(),
                run_id: paths.run_id.clone(),
                node_id: node_id.map(|s| crate::schema::NodeId::parse_str(s).unwrap()),
                idempotency_key: None,
                data,
            }
        }
        // validate is read-only, so running it first leaves apply's pre-state
        // intact; we compare the two verdicts on the same fresh run.
        fn agree(paths: &RunPaths, e: &Event, label: &str) {
            let v = validate_event(paths, e).is_err();
            let a = apply_event(paths, e).is_err();
            assert_eq!(v, a, "{label}: validate_err={v} apply_err={a}");
        }

        // Live node: bad report rejected; good report accepted; missing
        // node_id rejected; bad status rejected.
        {
            let tmp = TempDir::new().unwrap();
            let paths = fresh_run(&tmp);
            bootstrap_live_node(&paths);
            agree(
                &paths,
                &ev(&paths, "node.report", Some("n-0001"), json!({})),
                "report-bare",
            );
        }
        {
            let tmp = TempDir::new().unwrap();
            let paths = fresh_run(&tmp);
            bootstrap_live_node(&paths);
            agree(
                &paths,
                &ev(
                    &paths,
                    "node.report",
                    Some("n-0001"),
                    json!({ "success": true }),
                ),
                "report-good",
            );
        }
        {
            let tmp = TempDir::new().unwrap();
            let paths = fresh_run(&tmp);
            bootstrap_live_node(&paths);
            agree(
                &paths,
                &ev(&paths, "node.report", None, json!({})),
                "report-no-node-id",
            );
        }
        {
            let tmp = TempDir::new().unwrap();
            let paths = fresh_run(&tmp);
            bootstrap_live_node(&paths);
            agree(
                &paths,
                &ev(&paths, "node.status", Some("n-0001"), json!({})),
                "status-missing",
            );
        }
        // Terminal node: a malformed report is a clean no-op (guard before
        // validate) — both must accept it.
        {
            let tmp = TempDir::new().unwrap();
            let paths = fresh_run(&tmp);
            bootstrap_live_node(&paths);
            append_and_apply_event(
                &paths,
                "node.report",
                Some(&nid("n-0001")),
                None,
                json!({ "success": true }),
            )
            .unwrap();
            agree(
                &paths,
                &ev(&paths, "node.report", Some("n-0001"), json!({})),
                "report-bare-on-terminal",
            );
        }
        // Missing node: a status with no `status` field is a no-op.
        {
            let tmp = TempDir::new().unwrap();
            let paths = fresh_run(&tmp);
            agree(
                &paths,
                &ev(&paths, "node.status", Some("n-0001"), json!({})),
                "status-missing-node",
            );
        }
        // Existing manifest: a run.status with no `status` is rejected.
        {
            let tmp = TempDir::new().unwrap();
            let paths = fresh_run(&tmp);
            bootstrap_live_node(&paths);
            agree(
                &paths,
                &ev(&paths, "run.status", None, json!({})),
                "run-status-missing",
            );
        }
        // Open discussion: a resolve without `resolution` is rejected.
        {
            let tmp = TempDir::new().unwrap();
            let paths = fresh_run(&tmp);
            bootstrap_live_node(&paths);
            append_and_apply_event(
                &paths,
                "discussion.opened",
                Some(&nid("n-0001")),
                None,
                json!({ "discussion_id": "d-abcdefghij", "topic": "t", "node_id": "n-0001" }),
            )
            .unwrap();
            agree(
                &paths,
                &ev(
                    &paths,
                    "discussion.resolved",
                    None,
                    json!({ "discussion_id": "d-abcdefghij" }),
                ),
                "resolve-missing-resolution",
            );
        }
        // node.created: new node missing `kind` rejected; replay over an
        // existing node with bad payload is a no-op (existence short-circuit).
        {
            let tmp = TempDir::new().unwrap();
            let paths = fresh_run(&tmp);
            agree(
                &paths,
                &ev(&paths, "node.created", Some("n-0002"), json!({})),
                "node-created-missing-kind",
            );
        }
        {
            let tmp = TempDir::new().unwrap();
            let paths = fresh_run(&tmp);
            bootstrap_live_node(&paths);
            agree(
                &paths,
                &ev(&paths, "node.created", Some("n-0001"), json!({})),
                "node-created-replay-bad-payload",
            );
        }
        // discussion.opened missing `topic`.
        {
            let tmp = TempDir::new().unwrap();
            let paths = fresh_run(&tmp);
            bootstrap_live_node(&paths);
            agree(
                &paths,
                &ev(
                    &paths,
                    "discussion.opened",
                    Some("n-0001"),
                    json!({ "discussion_id": "d-abcdefghij", "node_id": "n-0001" }),
                ),
                "discussion-opened-missing-topic",
            );
        }
        // spinoff.proposed missing `proposed_title`; spinoff.{approved,rejected}
        // with an unparseable proposal id.
        {
            let tmp = TempDir::new().unwrap();
            let paths = fresh_run(&tmp);
            bootstrap_live_node(&paths);
            agree(
                &paths,
                &ev(
                    &paths,
                    "spinoff.proposed",
                    Some("n-0001"),
                    json!({ "proposal_id": "p-abcdefghij", "proposed_kind": "spinoff", "node_id": "n-0001" }),
                ),
                "spinoff-proposed-missing-title",
            );
        }
        {
            let tmp = TempDir::new().unwrap();
            let paths = fresh_run(&tmp);
            agree(
                &paths,
                &ev(
                    &paths,
                    "spinoff.approved",
                    None,
                    json!({ "proposal_id": "not a valid id" }),
                ),
                "spinoff-approved-bad-id",
            );
            agree(
                &paths,
                &ev(
                    &paths,
                    "spinoff.rejected",
                    None,
                    json!({ "proposal_id": "not a valid id" }),
                ),
                "spinoff-rejected-bad-id",
            );
        }
        // child.spawned: missing/invalid child_run_id.
        {
            let tmp = TempDir::new().unwrap();
            let paths = fresh_run(&tmp);
            agree(
                &paths,
                &ev(&paths, "child.spawned", Some("n-0001"), json!({})),
                "child-spawned-missing-child-run-id",
            );
            agree(
                &paths,
                &ev(
                    &paths,
                    "child.spawned",
                    Some("n-0001"),
                    json!({ "child_run_id": "bad" }),
                ),
                "child-spawned-bad-child-run-id",
            );
        }
        // Cross-run envelope and unknown kind.
        {
            let tmp = TempDir::new().unwrap();
            let paths = fresh_run(&tmp);
            let mut foreign = ev(&paths, "run.status", None, json!({ "status": "running" }));
            foreign.run_id = crate::schema::RunId::parse_str("02jxsnap000000000000000000").unwrap();
            agree(&paths, &foreign, "cross-run");
            agree(
                &paths,
                &ev(&paths, "totally.unknown", None, json!({})),
                "unknown-kind",
            );
        }
    }

    #[test]
    fn read_all_events_drops_torn_final_line() {
        // The bug this fixes: `read_all_events` used to silently ACCEPT a
        // valid-JSON final line lacking a trailing newline — a line
        // `recover_last_seq` discards as an uncommitted partial write. Now it
        // shares the torn-tail policy: the torn final line is dropped without
        // error, and the reader agrees with `recover_last_seq`.
        let tmp = TempDir::new().unwrap();
        let mut log = String::new();
        log.push_str(
            r#"{"ts":"2026-06-12T00:00:00Z","seq":1,"kind":"marker","run_id":"01jxsnap000000000000000000","data":{}}"#,
        );
        log.push('\n');
        // A COMPLETE, valid-JSON event whose trailing newline never flushed.
        log.push_str(
            r#"{"ts":"2026-06-12T00:00:00Z","seq":2,"kind":"marker","run_id":"01jxsnap000000000000000000","data":{}}"#,
        );
        let paths = paths_with_events(&tmp, log.as_bytes());

        let events = read_all_events(&paths.events()).unwrap();
        assert_eq!(
            events.iter().map(|e| e.seq).collect::<Vec<_>>(),
            vec![1],
            "torn final line must be dropped, not parsed"
        );
        // And it agrees with the recovery path.
        assert_eq!(recover_last_seq(&paths.events()).unwrap(), 1);
    }

    #[test]
    fn recover_last_seq_rejects_seq_only_last_line() {
        // A `\n`-terminated last line that is valid JSON with a `seq` but is
        // NOT a valid event envelope (missing ts/kind/run_id) must be rejected
        // by recover_last_seq, matching read_all_events — otherwise an append
        // would continue past a line replay can never fold.
        let tmp = TempDir::new().unwrap();
        let paths = paths_with_events(&tmp, b"{\"seq\":99}\n");
        let err = recover_last_seq(&paths.events()).unwrap_err();
        assert!(matches!(err, Error::CorruptEventLog { .. }), "got {err:?}");
        // And the forward reader agrees.
        assert!(matches!(
            read_all_events(&paths.events()).unwrap_err(),
            Error::CorruptEventLog { .. }
        ));
    }

    #[test]
    fn recover_last_seq_skips_multiple_trailing_blank_lines() {
        // External editing can leave several trailing blank lines. The forward
        // reader skips them; seq recovery must walk back over all of them to
        // the last real record (not just one), so the two readers agree.
        let tmp = TempDir::new().unwrap();
        let mut log = String::new();
        log.push_str(
            r#"{"ts":"2026-06-12T00:00:00Z","seq":7,"kind":"marker","run_id":"01jxsnap000000000000000000","data":{}}"#,
        );
        log.push_str("\n\n\n\n");
        let paths = paths_with_events(&tmp, log.as_bytes());
        assert_eq!(recover_last_seq(&paths.events()).unwrap(), 7);
        let events = read_all_events(&paths.events()).unwrap();
        assert_eq!(events.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![7]);
    }

    #[test]
    fn read_all_events_rejects_corrupt_middle_line() {
        // A newline-terminated garbage line FOLLOWED by another line is
        // interior corruption — a hard `CorruptEventLog`, never a silent skip.
        let tmp = TempDir::new().unwrap();
        let log = concat!(
            r#"{"ts":"2026-06-12T00:00:00Z","seq":1,"kind":"marker","run_id":"01jxsnap000000000000000000","data":{}}"#,
            "\n",
            "{not valid json at all\n",
            r#"{"ts":"2026-06-12T00:00:00Z","seq":3,"kind":"marker","run_id":"01jxsnap000000000000000000","data":{}}"#,
            "\n",
        );
        let paths = paths_with_events(&tmp, log.as_bytes());
        let err = read_all_events(&paths.events()).unwrap_err();
        match err {
            Error::CorruptEventLog { reason, .. } => {
                assert!(reason.contains("line 2"), "reason was: {reason}");
            }
            other => panic!("expected CorruptEventLog, got {other:?}"),
        }
    }

    #[test]
    fn append_truncates_torn_tail_before_writing() {
        // A crash left a valid record then a torn (newline-less) partial
        // write. The next append must truncate the torn bytes BEFORE writing,
        // so the log never gains a `…torn…{"seq":N}` malformed line.
        let tmp = TempDir::new().unwrap();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(
            br#"{"ts":"2026-06-12T00:00:00Z","seq":1,"kind":"marker","run_id":"01jxsnap000000000000000000","data":{}}"#,
        );
        bytes.push(b'\n');
        bytes.extend_from_slice(br#"{"seq":2,"kind":"TORN_PARTIAL_NEVER_FLUSHED"#); // no newline
        let paths = paths_with_events(&tmp, &bytes);

        // The torn tail is ignored for seq recovery (last complete seq = 1).
        assert_eq!(recover_last_seq(&paths.events()).unwrap(), 1);

        // `marker` is an unknown kind → reducer no-op, so the append succeeds
        // without any projection prerequisites.
        let r = append_and_apply_event(&paths, "marker", None, None, serde_json::json!({"x": 1}))
            .unwrap();
        assert_eq!(r.seq, 2, "seq continues from the last complete record");

        let raw = std::fs::read(paths.events()).unwrap();
        assert!(
            raw.ends_with(b"\n"),
            "log must be newline-terminated after a clean append"
        );
        assert!(
            !String::from_utf8_lossy(&raw).contains("TORN_PARTIAL_NEVER_FLUSHED"),
            "the torn tail must be truncated away before the append"
        );
        let events = read_all_events(&paths.events()).unwrap();
        assert_eq!(events.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![1, 2]);
    }

    #[test]
    fn append_truncates_all_torn_file_to_empty_then_writes_seq_1() {
        // The whole file is one torn (newline-less) partial write — no complete
        // record exists. truncate_torn_tail must cut it to empty, and the next
        // append starts a fresh seq 1.
        let tmp = TempDir::new().unwrap();
        let paths = paths_with_events(&tmp, br#"{"seq":1,"kind":"marker"#);
        assert_eq!(recover_last_seq(&paths.events()).unwrap(), 0);

        let r = append_and_apply_event(&paths, "marker", None, None, json!({})).unwrap();
        assert_eq!(r.seq, 1);
        let events = read_all_events(&paths.events()).unwrap();
        assert_eq!(events.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![1]);
    }

    #[test]
    fn quarantine_excises_corrupt_middle_line_and_recovers() {
        // A valid record, a newline-terminated garbage line, then another
        // valid record. Quarantine must rename the original aside, write a
        // recovered log holding only the two valid lines, and report the bad
        // line's byte offset.
        let tmp = TempDir::new().unwrap();
        let good1 = r#"{"ts":"2026-06-12T00:00:00Z","seq":1,"kind":"marker","run_id":"01jxsnap000000000000000000","data":{}}"#;
        let bad = "{not valid json at all";
        let good3 = r#"{"ts":"2026-06-12T00:00:00Z","seq":3,"kind":"marker","run_id":"01jxsnap000000000000000000","data":{}}"#;
        let log = format!("{good1}\n{bad}\n{good3}\n");
        let paths = paths_with_events(&tmp, log.as_bytes());

        // Strict replay chokes on the poison line beforehand.
        assert!(matches!(
            read_all_events(&paths.events()).unwrap_err(),
            Error::CorruptEventLog { .. }
        ));

        let q = quarantine_corrupt_lines(&paths, "20260612T000000Z")
            .unwrap()
            .expect("a corrupt line was excised");
        // The bad line started at the byte after `good1\n`.
        assert_eq!(q.removed_byte_offsets, vec![(good1.len() + 1) as u64]);
        assert_eq!(
            q.backup_path.file_name().unwrap().to_str().unwrap(),
            "events.jsonl.corrupt-20260612T000000Z.bak"
        );

        // The backup is the verbatim original; the recovered log now replays
        // strictly with only the two valid records.
        assert_eq!(std::fs::read(&q.backup_path).unwrap(), log.as_bytes());
        let events = read_all_events(&paths.events()).unwrap();
        assert_eq!(events.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![1, 3]);
    }

    #[test]
    fn quarantine_clean_log_is_noop() {
        // A log with no corruption must not be renamed or rewritten.
        let tmp = TempDir::new().unwrap();
        let log = concat!(
            r#"{"ts":"2026-06-12T00:00:00Z","seq":1,"kind":"marker","run_id":"01jxsnap000000000000000000","data":{}}"#,
            "\n",
        );
        let paths = paths_with_events(&tmp, log.as_bytes());
        assert!(quarantine_corrupt_lines(&paths, "20260612T000000Z")
            .unwrap()
            .is_none());
        // No backup created; original untouched.
        assert_eq!(std::fs::read(paths.events()).unwrap(), log.as_bytes());
        let bak = paths
            .events()
            .with_file_name("events.jsonl.corrupt-20260612T000000Z.bak");
        assert!(!bak.exists());
    }

    #[test]
    fn quarantine_missing_log_is_none() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("run");
        std::fs::create_dir_all(&dir).unwrap();
        let paths = RunPaths::new(dir, "01jxsnap000000000000000000").unwrap();
        assert!(quarantine_corrupt_lines(&paths, "20260612T000000Z")
            .unwrap()
            .is_none());
    }

    #[test]
    fn quarantine_preserves_torn_tail_and_excises_only_corruption() {
        // A valid record, a corrupt newline-terminated line, then a torn
        // (newline-less) final line. Only the corrupt middle line is excised;
        // the torn tail is retained verbatim (the readers tolerate it as an
        // in-flight partial write — excising it would change behavior).
        let tmp = TempDir::new().unwrap();
        let good = r#"{"ts":"2026-06-12T00:00:00Z","seq":1,"kind":"marker","run_id":"01jxsnap000000000000000000","data":{}}"#;
        let bad = "{garbage";
        let torn = r#"{"seq":2,"kind":"node.rep"#; // mid-write, no newline
        let mut log = Vec::new();
        log.extend_from_slice(format!("{good}\n{bad}\n{torn}").as_bytes());
        let paths = paths_with_events(&tmp, &log);

        let q = quarantine_corrupt_lines(&paths, "20260612T000000Z")
            .unwrap()
            .expect("the corrupt middle line was excised");
        assert_eq!(q.removed_byte_offsets, vec![(good.len() + 1) as u64]);

        let recovered = std::fs::read(paths.events()).unwrap();
        assert_eq!(recovered, format!("{good}\n{torn}").as_bytes());
        // The torn tail still recovers the last complete seq as 1.
        assert_eq!(recover_last_seq(&paths.events()).unwrap(), 1);
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
