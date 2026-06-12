//! `event tail` — stream a run's `events.jsonl` to stdout (or `--output`).
//!
//! Per design.md §2.3 and AGENTS-AI-FIRST-CLI §12:
//! - Without `--follow`: read events with `seq >= from_seq` to EOF, emit
//!   a terminal `{"event":"result","status":"ok"}` envelope (JSON modes),
//!   exit 0.
//! - With `--follow`: same initial read, then poll every 500ms. SIGINT
//!   exits 130, SIGTERM exits 143 (best-effort: ctrlc does not surface
//!   the signal value so we use 130 as the conservative default — see
//!   note below). On signal, emit `{"event":"cancelled"}` and flush.
//!
//! Signal note: `ctrlc::set_handler` invokes the same callback for SIGINT
//! and SIGTERM (with the `termination` feature) without distinguishing
//! them. The task spec authorises the 130 fallback when the crate cannot
//! distinguish. If we later need a true 143 on SIGTERM, swap `ctrlc` for
//! `signal-hook`.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::process;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use octl_core::{read_manifest_opt, Event};

use crate::error::CliError;
use crate::event::FormatArg;
use crate::run::{from_core, require_safe_id, run_paths};

const POLL_INTERVAL: Duration = Duration::from_millis(500);
const SIGINT_EXIT: i32 = 130;

pub struct Args<'a> {
    pub run_id: String,
    pub from_seq: u64,
    pub follow: bool,
    pub format: Option<FormatArg>,
    pub output: Option<PathBuf>,
    pub json: bool,
    pub warnings: &'a [String],
}

/// Resolved output format. Without `--format`, `--json` selects `jsonl`
/// (canonical machine-readable stream), otherwise the default is `text`.
fn resolve_format(format: Option<FormatArg>, json: bool) -> FormatArg {
    match format {
        Some(f) => f,
        None if json => FormatArg::Jsonl,
        None => FormatArg::Text,
    }
}

pub fn run(args: Args<'_>) -> Result<(), CliError> {
    let run_id = require_safe_id(&args.run_id, "run-id")?;
    let root = crate::home::root_dir()?;
    let paths = run_paths(&root, &run_id);

    // Require run to exist (consistent with `run show`); reading a
    // non-existent run id is a user error, not "stream of nothing".
    if read_manifest_opt(&paths).map_err(from_core)?.is_none() {
        return Err(
            CliError::user("run_not_found", format!("no run with id {run_id}"))
                .with_invalid_value(&run_id),
        );
    }

    let format = resolve_format(args.format, args.json);
    let events_path = paths.events();

    // Logging warnings are surfaced to stderr up-front (the rest of the
    // run is a long-running stream, so we can't put them in a trailing
    // envelope the way `run show` does).
    for w in args.warnings {
        eprintln!("warning: {}", w);
    }

    // Install the signal handler before we start reading so a SIGINT
    // during the initial read also lands a `cancelled` envelope.
    let cancel = Arc::new(AtomicBool::new(false));
    if args.follow {
        let flag = cancel.clone();
        // try_set_handler avoids panicking under repeated installation
        // (e.g. from integration test harnesses).
        let _ = ctrlc::try_set_handler(move || {
            flag.store(true, Ordering::SeqCst);
        });
    }

    // Open writer (stdout or --output file).
    let mut writer: Box<dyn Write> = match &args.output {
        None => Box::new(std::io::stdout().lock()),
        Some(p) => {
            let mut opts = OpenOptions::new();
            opts.create(true).write(true);
            if args.follow {
                opts.append(true);
            } else {
                opts.truncate(true);
            }
            let f = opts.open(p).map_err(|e| {
                CliError::system("io_error", format!("open {}: {}", p.display(), e))
            })?;
            Box::new(f)
        }
    };

    // Open events file (may not yet exist for a freshly-created run).
    let mut reader = open_events_reader(&events_path)?;
    let mut last_seen_seq: u64 = args.from_seq.saturating_sub(1);
    let mut buf = String::new();

    // Initial drain of the file.
    last_seen_seq = drain(&mut reader, &mut buf, &mut *writer, format, last_seen_seq)?;

    if !args.follow {
        emit_terminal(&mut *writer, format, TerminalKind::Result)?;
        writer
            .flush()
            .map_err(|e| CliError::system("io_error", format!("flush: {e}")))?;
        return Ok(());
    }

    // Follow loop.
    loop {
        if cancel.load(Ordering::SeqCst) {
            emit_terminal(&mut *writer, format, TerminalKind::Cancelled)?;
            let _ = writer.flush();
            // Drop writer before exit to flush stdout buffers / close file.
            drop(writer);
            process::exit(SIGINT_EXIT);
        }
        std::thread::sleep(POLL_INTERVAL);
        if cancel.load(Ordering::SeqCst) {
            continue;
        }
        // Detect truncation: file shrank below our current read position.
        let pos = reader
            .stream_position()
            .map_err(|e| CliError::system("io_error", format!("stream_position: {e}")))?;
        let len_now = match std::fs::metadata(&events_path) {
            Ok(m) => m.len(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
            Err(e) => {
                return Err(CliError::system(
                    "io_error",
                    format!("stat {}: {}", events_path.display(), e),
                ));
            }
        };
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
        last_seen_seq = drain(&mut reader, &mut buf, &mut *writer, format, last_seen_seq)?;
    }
}

/// Open `events.jsonl` if present, else hand back a reader over a empty
/// file we created in-memory so the polling loop can keep going until the
/// supervisor appends the first line.
fn open_events_reader(events_path: &std::path::Path) -> Result<BufReader<File>, CliError> {
    match File::open(events_path) {
        Ok(f) => Ok(BufReader::new(f)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Touch the file so the BufReader has something to seek/read.
            // Best-effort; if creation races with the writer, we open it
            // on the next poll cycle.
            let _ = OpenOptions::new()
                .create(true)
                .append(true)
                .open(events_path);
            let f = File::open(events_path).map_err(|e| {
                CliError::system("io_error", format!("open {}: {}", events_path.display(), e))
            })?;
            Ok(BufReader::new(f))
        }
        Err(e) => Err(CliError::system(
            "io_error",
            format!("open {}: {}", events_path.display(), e),
        )),
    }
}

/// Read every complete line currently in the file, parse, and emit. A
/// partial trailing line (no `\n`) is left in the file for the next poll
/// — we seek back so we re-read it whole.
fn drain(
    reader: &mut BufReader<File>,
    buf: &mut String,
    writer: &mut dyn Write,
    format: FormatArg,
    mut last_seen_seq: u64,
) -> Result<u64, CliError> {
    loop {
        buf.clear();
        let pos_before = reader
            .stream_position()
            .map_err(|e| CliError::system("io_error", format!("stream_position: {e}")))?;
        let n = reader
            .read_line(buf)
            .map_err(|e| CliError::system("io_error", format!("read_line: {e}")))?;
        if n == 0 {
            return Ok(last_seen_seq);
        }
        if !buf.ends_with('\n') {
            // Partial line — caller is mid-write. Rewind and retry on next poll.
            reader
                .seek(SeekFrom::Start(pos_before))
                .map_err(|e| CliError::system("io_error", format!("seek: {e}")))?;
            return Ok(last_seen_seq);
        }
        let line = buf.trim_end_matches(['\n', '\r']);
        if line.is_empty() {
            continue;
        }
        let ev: Event = serde_json::from_str(line).map_err(|e| {
            CliError::system(
                "events_log_corrupt",
                format!("parse event line: {e}: {line}"),
            )
        })?;
        if ev.seq <= last_seen_seq {
            continue;
        }
        emit_event(writer, format, &ev)?;
        last_seen_seq = ev.seq;
    }
}

fn emit_event(writer: &mut dyn Write, format: FormatArg, ev: &Event) -> Result<(), CliError> {
    match format {
        FormatArg::Jsonl => {
            let line = serde_json::to_string(ev)
                .map_err(|e| CliError::system("internal_serialize", e.to_string()))?;
            writeln!(writer, "{}", line)
        }
        FormatArg::Json => {
            let pretty = serde_json::to_string_pretty(ev)
                .map_err(|e| CliError::system("internal_serialize", e.to_string()))?;
            writeln!(writer, "{}", pretty)
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
        "[{seq:>6}] {ts} {kind}{node}{detail}",
        seq = ev.seq,
        ts = ev.ts.to_rfc3339(),
        kind = ev.kind,
    )
}

/// Try to surface the most useful inline field for human eyes. We avoid
/// emitting the full payload (some are tens of KB) and instead probe a
/// few common keys.
fn text_data_detail(data: &Value) -> String {
    let obj = match data.as_object() {
        Some(o) => o,
        None => return String::new(),
    };
    for key in ["status", "title", "kind", "reason", "message"] {
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
    event: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<&'a str>,
}

/// Emit the terminal `{"event":"result"}` or `{"event":"cancelled"}`
/// envelope. Text mode emits nothing (humans read the trailing event and
/// don't need a marker line — the prompt returns).
fn emit_terminal(
    writer: &mut dyn Write,
    format: FormatArg,
    kind: TerminalKind,
) -> Result<(), CliError> {
    if matches!(format, FormatArg::Text) {
        return Ok(());
    }
    let envelope = match kind {
        TerminalKind::Result => TerminalEnvelope {
            event: "result",
            status: Some("ok"),
        },
        TerminalKind::Cancelled => TerminalEnvelope {
            event: "cancelled",
            status: None,
        },
    };
    let line = serde_json::to_string(&envelope)
        .map_err(|e| CliError::system("internal_serialize", e.to_string()))?;
    writeln!(writer, "{}", line)
        .map_err(|e| CliError::system("io_error", format!("write terminal: {e}")))
}
