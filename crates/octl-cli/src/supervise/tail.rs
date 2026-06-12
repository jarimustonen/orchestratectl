//! Tail-follow primitive over a run's `events.jsonl`.
//!
//! Polling-based (no inotify in MVP). Each tick re-reads from the
//! recorded byte offset; returns all parsed events whose `seq` is
//! greater than the supplied `since_seq` cursor.
//!
//! Designed to be cheap to call repeatedly: keeps a byte offset so we
//! never re-parse already-seen lines. The first call seeks to find the
//! offset corresponding to `since_seq`; subsequent calls start at the
//! cached `pos`.

use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use octl_core::Event;

use crate::error::CliError;

pub struct EventTail {
    path: PathBuf,
    /// Byte offset of the start of the next unread line.
    pos: u64,
    /// Highest `seq` returned so far. Newly-parsed events with
    /// `seq <= last_seq` are skipped — defends against a torn read or
    /// a replayed `--from-seq` cursor.
    last_seq: u64,
}

impl EventTail {
    /// Build a tail starting just past `since_seq`. The first `poll()`
    /// call will skip over already-seen lines.
    pub fn new(path: impl Into<PathBuf>, since_seq: u64) -> Self {
        Self {
            path: path.into(),
            pos: 0,
            last_seq: since_seq,
        }
    }

    /// Read all events appended since the last poll. Returns an empty
    /// vec when nothing new is available. Tolerates the file not yet
    /// existing (a child run's `events.jsonl` may lag the
    /// `child.spawned` event by a few ms).
    pub fn poll(&mut self) -> Result<Vec<Event>, CliError> {
        let mut f = match File::open(&self.path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => {
                return Err(CliError::system(
                    "io_error",
                    format!("open {}: {}", self.path.display(), e),
                ));
            }
        };
        let len = f
            .metadata()
            .map_err(|e| {
                CliError::system("io_error", format!("metadata {}: {}", self.path.display(), e))
            })?
            .len();
        if len < self.pos {
            // File truncated under us (unlikely in production, but
            // tests/dev may rm+re-create). Restart from the beginning;
            // duplicate-seq guard below will skip already-seen events.
            self.pos = 0;
        }
        if len == self.pos {
            return Ok(Vec::new());
        }
        f.seek(SeekFrom::Start(self.pos)).map_err(|e| {
            CliError::system("io_error", format!("seek {}: {}", self.path.display(), e))
        })?;
        let mut reader = BufReader::new(f);
        let mut out = Vec::new();
        let mut consumed: u64 = 0;
        loop {
            let mut line = String::new();
            let n = reader.read_line(&mut line).map_err(|e| {
                CliError::system(
                    "io_error",
                    format!("read {}: {}", self.path.display(), e),
                )
            })?;
            if n == 0 {
                break;
            }
            if !line.ends_with('\n') {
                // Partial line (mid-append). Don't consume its bytes —
                // the next poll will retry from this offset.
                break;
            }
            consumed += n as u64;
            let trimmed = line.trim_end_matches(['\n', '\r']);
            if trimmed.is_empty() {
                continue;
            }
            let ev: Event = match serde_json::from_str(trimmed) {
                Ok(v) => v,
                Err(e) => {
                    return Err(CliError::system(
                        "corrupt_event_log",
                        format!(
                            "parse {}: {} (line: {})",
                            self.path.display(),
                            e,
                            trimmed.chars().take(120).collect::<String>()
                        ),
                    ));
                }
            };
            if ev.seq <= self.last_seq {
                continue;
            }
            self.last_seq = ev.seq;
            out.push(ev);
        }
        self.pos += consumed;
        Ok(out)
    }

    #[allow(dead_code)]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[allow(dead_code)]
    pub fn last_seq(&self) -> u64 {
        self.last_seq
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_line(path: &Path, seq: u64, kind: &str) {
        let line = format!(
            "{{\"ts\":\"2026-06-12T00:00:00Z\",\"seq\":{seq},\"kind\":\"{kind}\",\"run_id\":\"r\",\"data\":{{}}}}\n"
        );
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        f.write_all(line.as_bytes()).unwrap();
    }

    #[test]
    fn missing_file_yields_empty() {
        let dir = TempDir::new().unwrap();
        let mut t = EventTail::new(dir.path().join("missing.jsonl"), 0);
        assert!(t.poll().unwrap().is_empty());
    }

    #[test]
    fn reads_new_events_only() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("events.jsonl");
        write_line(&p, 1, "node.created");
        write_line(&p, 2, "node.report");

        let mut t = EventTail::new(&p, 0);
        let first = t.poll().unwrap();
        assert_eq!(first.len(), 2);
        assert_eq!(first[0].seq, 1);
        assert_eq!(first[1].seq, 2);

        // Second poll: nothing new.
        assert!(t.poll().unwrap().is_empty());

        // Append more.
        write_line(&p, 3, "child.spawned");
        let next = t.poll().unwrap();
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].seq, 3);
    }

    #[test]
    fn skips_already_consumed_seqs() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("events.jsonl");
        write_line(&p, 1, "a");
        write_line(&p, 2, "b");
        write_line(&p, 3, "c");
        let mut t = EventTail::new(&p, 2);
        let evs = t.poll().unwrap();
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].seq, 3);
    }

    #[test]
    fn ignores_partial_trailing_line() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("events.jsonl");
        write_line(&p, 1, "a");
        // Append a partial line (no newline).
        {
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&p)
                .unwrap();
            f.write_all(b"{\"ts\":\"2026").unwrap();
        }
        let mut t = EventTail::new(&p, 0);
        let evs = t.poll().unwrap();
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].seq, 1);
        // Finish the line and re-poll.
        {
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&p)
                .unwrap();
            f.write_all(
                b"-06-12T00:00:00Z\",\"seq\":2,\"kind\":\"b\",\"run_id\":\"r\",\"data\":{}}\n",
            )
            .unwrap();
        }
        let evs = t.poll().unwrap();
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].seq, 2);
    }
}
