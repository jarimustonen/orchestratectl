//! `event tail` — stream a run's `events.jsonl` to stdout (or `--output`).
//!
//! Per design.md §2.3 and AGENTS-AI-FIRST-CLI §10/§12:
//! - Without `--follow`: read events with `seq >= from_seq` to EOF, emit
//!   a terminal `{"event":"result","status":"ok",...}` envelope in JSONL
//!   mode, exit 0.
//! - With `--follow`: same initial read, then poll the file every 500ms.
//!   SIGINT exits 130; SIGTERM also exits 130 (known §12 divergence —
//!   the task spec explicitly authorises the conservative fallback
//!   because `ctrlc::set_handler` does not surface the signal value).
//!   On signal, emit `{"event":"cancelled",...}` and flush. The signal
//!   is checked between events in the initial drain too, so a large
//!   backlog can still be cancelled responsively.
//!
//! Limitations (documented contract):
//! - `events.jsonl` is append-only by design.md §1.4; the follow loop
//!   does NOT handle log rotation / inode replacement. Such a change is
//!   detected via fd-shrink as `events_log_truncated`, but a same-size
//!   replacement would silently desync. That is out of contract.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Seek, Write};
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use octl_core::{read_manifest_opt, Event, SCHEMA_VERSION};

use crate::error::CliError;
use crate::event::{resolve_format, FormatArg};
use crate::output::OutputSpec;
use crate::run::{from_core, run_paths};

const POLL_INTERVAL: Duration = Duration::from_millis(500);
/// Exit code returned for both SIGINT and SIGTERM. Strictly correct
/// per §12 is 130 / 143 respectively, but `ctrlc` does not surface the
/// signal value to the handler — task spec authorises this fallback.
const CANCELLED_EXIT_FALLBACK: i32 = 130;

pub struct Args<'a> {
    pub run_id: String,
    pub from_seq: u64,
    pub follow: bool,
    pub to_file: Option<PathBuf>,
    pub spec: &'a OutputSpec,
    pub warnings: &'a [String],
}

pub fn run(args: Args<'_>) -> Result<(), CliError> {
    let run_id = &args.run_id;
    let format = resolve_format(args.spec.format)?;
    let root = crate::home::root_dir()?;
    let paths = run_paths(&root, run_id)?;

    // Require the run to exist (consistent with `run show`); reading a
    // non-existent run id is a user error, not "stream of nothing".
    if read_manifest_opt(&paths).map_err(from_core)?.is_none() {
        return Err(
            CliError::user("run_not_found", format!("no run with id {run_id}"))
                .with_invalid_value(run_id.as_str()),
        );
    }

    let events_path = paths.events();

    // Refuse to write our render back into the canonical event log.
    // Without this guard, `--output <…>/events.jsonl` truncates (no-follow)
    // or appends (follow) the source file — silent data loss.
    if let Some(out) = &args.to_file {
        reject_output_alias(&events_path, out)?;
    }

    // Logging warnings to stderr. §10 wants them in the stdout payload
    // under `--json`, but a stream has no single payload. Streaming-side
    // delivery (embed in terminal envelope) is tracked as a follow-up;
    // for now stderr keeps them visible without polluting the JSONL stream.
    for w in args.warnings {
        eprintln!("warning: {w}");
    }

    // Install signal handler before any I/O so a SIGINT during the
    // initial drain is honoured. Best-effort: re-install on repeated
    // invocations (e.g. test harnesses) is benign — `ctrlc` returns
    // `HandlerAlreadyExists` which we ignore.
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let flag = cancel.clone();
        let _ = ctrlc::try_set_handler(move || {
            flag.store(true, Ordering::Release);
        });
    }

    // Open writer (stdout or --output file).
    let mut writer: Box<dyn Write> = match &args.to_file {
        None => Box::new(std::io::stdout().lock()),
        Some(p) => Box::new(open_output(p, args.follow)?),
    };

    // Open events file if present; if missing, `reader` is None until
    // the follow loop sees it appear. Never *create* the file — that's
    // the supervisor's job.
    let mut reader = try_open_events_reader(&events_path)?;
    let mut last_seen_seq: Option<u64> = None;
    let mut buf = String::new();

    if let Some(r) = reader.as_mut() {
        drain(
            r,
            &mut buf,
            &mut *writer,
            format,
            args.from_seq,
            &mut last_seen_seq,
            &cancel,
        )?;
    }

    // If a signal arrived during initial drain, honour it regardless of
    // --follow.
    if cancel.load(Ordering::Acquire) {
        emit_terminal(&mut *writer, format, TerminalKind::Cancelled, last_seen_seq)?;
        flush_and_exit(writer, CANCELLED_EXIT_FALLBACK);
    }

    if !args.follow {
        emit_terminal(&mut *writer, format, TerminalKind::Result, last_seen_seq)?;
        writer
            .flush()
            .map_err(|e| CliError::system("io_error", format!("flush: {e}")))?;
        return Ok(());
    }

    // Follow loop.
    loop {
        if cancel.load(Ordering::Acquire) {
            emit_terminal(&mut *writer, format, TerminalKind::Cancelled, last_seen_seq)?;
            flush_and_exit(writer, CANCELLED_EXIT_FALLBACK);
        }
        std::thread::sleep(POLL_INTERVAL);
        if cancel.load(Ordering::Acquire) {
            continue;
        }

        // First time the file appears, open it.
        if reader.is_none() {
            reader = try_open_events_reader(&events_path)?;
            if reader.is_none() {
                continue;
            }
        }

        let r = reader.as_mut().unwrap();

        // Detect truncation against the open fd (not the path — a rotated
        // file would otherwise be misreported as "shrunk").
        let pos = r
            .stream_position()
            .map_err(|e| CliError::system("io_error", format!("stream_position: {e}")))?;
        let len_now = r
            .get_ref()
            .metadata()
            .map_err(|e| CliError::system("io_error", format!("fstat: {e}")))?
            .len();
        if len_now < pos {
            return Err(CliError::system(
                "events_log_truncated",
                format!(
                    "{} shrank from {} to {} bytes — append-only contract violated",
                    events_path.display(),
                    pos,
                    len_now
                ),
            ));
        }

        drain(
            r,
            &mut buf,
            &mut *writer,
            format,
            args.from_seq,
            &mut last_seen_seq,
            &cancel,
        )?;
    }
}

/// Try to open `events.jsonl` for reading. `Ok(None)` means it doesn't
/// exist yet — caller can poll. Other IO errors propagate.
fn try_open_events_reader(events_path: &Path) -> Result<Option<BufReader<File>>, CliError> {
    match File::open(events_path) {
        Ok(f) => Ok(Some(BufReader::new(f))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(CliError::system(
            "io_error",
            format!("open {}: {}", events_path.display(), e),
        )),
    }
}

fn open_output(p: &Path, follow: bool) -> Result<File, CliError> {
    let mut opts = OpenOptions::new();
    opts.create(true).write(true);
    if follow {
        opts.append(true);
    } else {
        opts.truncate(true);
    }
    opts.open(p)
        .map_err(|e| CliError::system("io_error", format!("open {}: {}", p.display(), e)))
}

/// Refuse if `output` resolves to the same on-disk file as the source
/// `events.jsonl`. Without this, `--output <…>/events.jsonl` silently
/// destroys the canonical event log.
fn reject_output_alias(events_path: &Path, output: &Path) -> Result<(), CliError> {
    // Canonicalize when both exist (this is the common danger case —
    // events.jsonl is created by `run create`, so it almost always
    // exists by the time tail runs).
    if let (Ok(ev), Ok(out)) = (
        std::fs::canonicalize(events_path),
        std::fs::canonicalize(output),
    ) {
        if ev == out {
            return Err(CliError::user(
                "invalid_output",
                "--output must not point at the run's events.jsonl",
            )
            .with_invalid_value(output.display().to_string()));
        }
    }
    Ok(())
}

/// Read every complete line currently in the file, parse, and emit. The
/// `cancel` flag is polled per event so a backlog drain can be cancelled.
/// Partial trailing bytes (no `\n`) are left in `buf` for the next call
/// to append to — no seek-back-and-rebuffer dance.
fn drain(
    reader: &mut BufReader<File>,
    buf: &mut String,
    writer: &mut dyn Write,
    format: FormatArg,
    from_seq: u64,
    last_seen_seq: &mut Option<u64>,
    cancel: &AtomicBool,
) -> Result<(), CliError> {
    let mut emitted_any = false;
    loop {
        if cancel.load(Ordering::Acquire) {
            break;
        }
        // `read_line` APPENDS to `buf` and stops at the first `\n`. We
        // accumulate partial bytes across poll cycles until we see one;
        // the whole `buf` is then a single complete line.
        let n = reader
            .read_line(buf)
            .map_err(|e| CliError::system("io_error", format!("read_line: {e}")))?;
        if n == 0 {
            // True EOF (no bytes read this call). Any pending partial
            // bytes stay in `buf` for the next poll cycle to complete.
            break;
        }
        if !buf.ends_with('\n') {
            // Read some bytes but no terminator yet — partial line.
            // Leave the bytes in `buf` and bail; the next `read_line`
            // call will append the remainder.
            break;
        }
        // `buf` holds exactly one complete line.
        let line = buf.trim_end_matches(['\n', '\r']);
        if !line.is_empty() {
            let ev: Event = serde_json::from_str(line).map_err(|e| {
                CliError::system(
                    "events_log_corrupt",
                    format!("parse event line: {e}: {line}"),
                )
            })?;
            let dedup_ok = match *last_seen_seq {
                Some(last) => ev.seq > last,
                None => true,
            };
            if ev.seq >= from_seq && dedup_ok {
                emit_event(writer, format, &ev)?;
                emitted_any = true;
            }
            if dedup_ok {
                *last_seen_seq = Some(ev.seq);
            }
        }
        // Consumed this line — start fresh for the next one.
        buf.clear();
    }
    if emitted_any {
        // Flush so consumers (especially block-buffered piped stdout)
        // observe new events within one poll interval.
        writer
            .flush()
            .map_err(|e| CliError::system("io_error", format!("flush: {e}")))?;
    }
    Ok(())
}

fn emit_event(writer: &mut dyn Write, format: FormatArg, ev: &Event) -> Result<(), CliError> {
    match format {
        FormatArg::Jsonl => {
            let line = serde_json::to_string(ev)
                .map_err(|e| CliError::system("internal_serialize", e.to_string()))?;
            writeln!(writer, "{line}")
        }
        FormatArg::Text => writeln!(writer, "{}", text_summary(ev)),
    }
    .map_err(|e| CliError::system("io_error", format!("write event: {e}")))
}

fn text_summary(ev: &Event) -> String {
    let node = ev
        .node_id
        .as_deref()
        .map(|n| format!(" node={n}"))
        .unwrap_or_default();
    let detail = text_data_detail(&ev.data);
    let detail = if detail.is_empty() {
        String::new()
    } else {
        format!(" {detail}")
    };
    format!(
        "[{seq}] {ts} {kind}{node}{detail}",
        seq = ev.seq,
        ts = ev.ts.to_rfc3339(),
        kind = ev.kind,
    )
}

/// Surface the most useful inline field for human eyes. We avoid emitting
/// the full payload (some are tens of KB) and probe common keys. `kind`
/// is excluded because it duplicates the outer field.
fn text_data_detail(data: &Value) -> String {
    let obj = match data.as_object() {
        Some(o) => o,
        None => return String::new(),
    };
    for key in ["status", "title", "reason", "message"] {
        if let Some(v) = obj.get(key).and_then(Value::as_str) {
            return format!("{key}={v}");
        }
    }
    String::new()
}

#[derive(Debug, Clone, Copy)]
enum TerminalKind {
    Result,
    Cancelled,
}

#[derive(Serialize)]
struct TerminalEnvelope<'a> {
    schema_version: u32,
    event: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_seq: Option<u64>,
}

/// Emit the terminal `{"event":"result"|"cancelled",...}` envelope. Text
/// mode emits nothing (humans read the trailing event line — no need for
/// a marker).
fn emit_terminal(
    writer: &mut dyn Write,
    format: FormatArg,
    kind: TerminalKind,
    last_seen_seq: Option<u64>,
) -> Result<(), CliError> {
    if matches!(format, FormatArg::Text) {
        return Ok(());
    }
    let envelope = match kind {
        TerminalKind::Result => TerminalEnvelope {
            schema_version: SCHEMA_VERSION,
            event: "result",
            status: Some("ok"),
            last_seq: last_seen_seq,
        },
        TerminalKind::Cancelled => TerminalEnvelope {
            schema_version: SCHEMA_VERSION,
            event: "cancelled",
            status: None,
            last_seq: last_seen_seq,
        },
    };
    let line = serde_json::to_string(&envelope)
        .map_err(|e| CliError::system("internal_serialize", e.to_string()))?;
    writeln!(writer, "{line}")
        .map_err(|e| CliError::system("io_error", format!("write terminal: {e}")))
}

/// Flush the writer AND the global stdout buffer, drop the writer, then
/// `process::exit`. The global `Stdout` is block-buffered when piped (the
/// agent case); dropping a `StdoutLock` releases the lock but does NOT
/// flush that block buffer. Without the explicit flush below, agents
/// reading through a pipe lose the final `cancelled` envelope.
fn flush_and_exit(mut writer: Box<dyn Write>, code: i32) -> ! {
    let _ = writer.flush();
    drop(writer);
    let _ = std::io::stdout().lock().flush();
    process::exit(code);
}
