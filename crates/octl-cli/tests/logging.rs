//! Smoke tests for the non-blocking JSONL log appender.
//!
//! `init_logging` hands the log file to a background writer thread
//! (`tracing_appender::non_blocking`). The `WorkerGuard` returned from it
//! is bound in `run()` so it outlives every subcommand; dropping it on a
//! clean exit flushes the channel and joins the worker. These tests run
//! the real binary against a throwaway `TASKFLEET_HOME` and assert
//! that the events emitted during a normal invocation actually reach
//! `logs/orchestratectl.log.jsonl` — i.e. the guard is held long enough
//! that nothing is dropped on the way out.

use std::path::PathBuf;
use std::process::Command;

use tempfile::TempDir;

fn bin(home: &TempDir) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_orchestratectl"));
    c.env("TASKFLEET_HOME", home.path());
    c
}

fn log_file(home: &TempDir) -> PathBuf {
    home.path().join("logs").join("orchestratectl.log.jsonl")
}

#[test]
fn clean_exit_flushes_jsonl_log_lines() {
    let home = TempDir::new().unwrap();

    // `version` runs the full dispatch path: it emits the
    // "command dispatched" info event before returning. A clean exit must
    // flush that event through the non-blocking worker to disk.
    let out = bin(&home).arg("version").output().expect("spawn");
    assert!(out.status.success(), "exit: {:?}", out.status);

    let path = log_file(&home);
    assert!(
        path.exists(),
        "log file was not created at {}",
        path.display()
    );

    let contents = std::fs::read_to_string(&path).expect("read log file");
    let lines: Vec<&str> = contents.lines().filter(|l| !l.trim().is_empty()).collect();
    assert!(
        !lines.is_empty(),
        "no log lines were flushed — guard likely dropped before exit"
    );

    // Every line must be a valid JSON object (the `.json()` formatter),
    // and at least one must be the dispatch event proving the writer ran
    // end-to-end.
    let mut saw_dispatch = false;
    for line in &lines {
        let v: serde_json::Value = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("non-JSON log line {line:?}: {e}"));
        assert!(v.is_object(), "log line is not a JSON object: {line:?}");
        if v["fields"]["message"] == "command dispatched" {
            saw_dispatch = true;
        }
    }
    assert!(
        saw_dispatch,
        "did not find the 'command dispatched' event in:\n{contents}"
    );
}

#[test]
fn repeated_invocations_append_without_truncation() {
    // The appender opens with O_APPEND; a second clean run must add lines
    // rather than clobber the first run's output. This guards against a
    // regression where the non-blocking writer is given a truncating
    // handle.
    let home = TempDir::new().unwrap();

    let out1 = bin(&home).arg("version").output().expect("spawn 1");
    assert!(out1.status.success(), "first run failed: {:?}", out1.status);
    let first = std::fs::read_to_string(log_file(&home)).expect("read after first run");
    let first_lines: Vec<String> = first
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_owned)
        .collect();
    assert!(!first_lines.is_empty(), "first run wrote no log lines");

    let out2 = bin(&home).arg("version").output().expect("spawn 2");
    assert!(
        out2.status.success(),
        "second run failed: {:?}",
        out2.status
    );
    let second = std::fs::read_to_string(log_file(&home)).expect("read after second run");
    let second_lines: Vec<String> = second
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_owned)
        .collect();

    // Append, not truncate: the first run's lines must survive verbatim as
    // a prefix of the file after the second run. A plain count check would
    // also pass against a writer that truncated and happened to re-emit at
    // least as many lines — comparing content rules that out.
    assert!(
        second_lines.len() > first_lines.len(),
        "second run did not append (first={}, second={})",
        first_lines.len(),
        second_lines.len()
    );
    assert_eq!(
        &second_lines[..first_lines.len()],
        first_lines.as_slice(),
        "first run's log lines were not preserved as a prefix — file was truncated/rewritten"
    );
}
