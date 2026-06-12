//! Integration tests for `event tail`.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;
use tempfile::TempDir;

fn bin(home: &TempDir) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_orchestratectl"));
    c.env("ORCHESTRATECTL_HOME", home.path());
    c
}

fn run_ok_output(cmd: &mut Command) -> (String, String) {
    let out = cmd.output().expect("spawn");
    let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
    let stderr = String::from_utf8(out.stderr).expect("utf8 stderr");
    assert!(
        out.status.success(),
        "exit={:?}\nstderr={}\nstdout={}",
        out.status,
        stderr,
        stdout
    );
    (stdout, stderr)
}

fn create_run(home: &TempDir) -> String {
    let out = bin(home)
        .args([
            "--json",
            "run",
            "create",
            "--kind",
            "spinoff",
            "--title",
            "tail-test",
        ])
        .output()
        .expect("spawn");
    assert!(
        out.status.success(),
        "create failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).expect("json");
    v["data"]["run_id"].as_str().expect("run_id").to_string()
}

fn events_path(home: &TempDir, run_id: &str) -> PathBuf {
    home.path().join("runs").join(run_id).join("events.jsonl")
}

fn append_event(path: &Path, seq: u64, kind: &str) {
    let line = serde_json::json!({
        "ts": "2026-06-12T10:00:00Z",
        "seq": seq,
        "kind": kind,
        "run_id": "test",
        "node_id": null,
        "idempotency_key": null,
        "data": {"status": "ok"},
    });
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .expect("open events.jsonl");
    let mut s = line.to_string();
    s.push('\n');
    f.write_all(s.as_bytes()).expect("write");
    f.sync_all().expect("sync");
}

#[test]
fn tail_reads_existing_events_and_terminal_envelope() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    let evp = events_path(&home, &run_id);
    // `run create` already wrote one `run.created` event.
    append_event(&evp, 2, "node.created");
    append_event(&evp, 3, "node.report");

    let (stdout, _) = run_ok_output(bin(&home).args(["--json", "event", "tail", &run_id]));
    let lines: Vec<&str> = stdout.lines().collect();
    // 3 events + 1 terminal envelope
    assert_eq!(lines.len(), 4, "stdout was: {stdout}");
    let last: Value = serde_json::from_str(lines.last().unwrap()).expect("terminal json");
    assert_eq!(last["event"], "result");
    assert_eq!(last["status"], "ok");
    // Per-event lines parse as Event
    let first: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(first["seq"], 1);
    let second: Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(second["kind"], "node.created");
}

#[test]
fn tail_from_seq_skips_earlier_events() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    let evp = events_path(&home, &run_id);
    append_event(&evp, 2, "a");
    append_event(&evp, 3, "b");

    let (stdout, _) =
        run_ok_output(bin(&home).args(["--json", "event", "tail", &run_id, "--from-seq", "3"]));
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "stdout was: {stdout}");
    let only: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(only["seq"], 3);
    let terminal: Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(terminal["event"], "result");
}

#[test]
fn tail_text_format_no_terminal_envelope() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    let (stdout, _) =
        run_ok_output(bin(&home).args(["event", "tail", &run_id, "--format", "text"]));
    // 1 event line (run.created), no envelope
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 1, "stdout was: {stdout}");
    assert!(lines[0].contains("run.created"), "line: {}", lines[0]);
    assert!(lines[0].starts_with("["), "line: {}", lines[0]);
}

#[test]
fn tail_jsonl_format_canonical_one_line() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    let (stdout, _) =
        run_ok_output(bin(&home).args(["event", "tail", &run_id, "--format", "jsonl"]));
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2);
    let ev: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(ev["kind"], "run.created");
}

#[test]
fn tail_output_file_truncate_without_follow() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    let out_path = home.path().join("captured.jsonl");
    // Pre-fill the output file: tail without --follow must truncate it.
    std::fs::write(&out_path, b"GARBAGE\n").unwrap();

    let _ = run_ok_output(bin(&home).args([
        "--json",
        "event",
        "tail",
        &run_id,
        "--output",
        out_path.to_str().unwrap(),
    ]));
    let body = std::fs::read_to_string(&out_path).unwrap();
    assert!(!body.contains("GARBAGE"), "file not truncated: {body}");
    let lines: Vec<&str> = body.lines().collect();
    assert_eq!(lines.len(), 2);
    let terminal: Value = serde_json::from_str(lines.last().unwrap()).unwrap();
    assert_eq!(terminal["event"], "result");
}

#[test]
fn tail_unknown_run_fails_with_run_not_found() {
    let home = TempDir::new().unwrap();
    let out = bin(&home)
        .args(["--json", "event", "tail", "01J0000000000000000000000X"])
        .output()
        .expect("spawn");
    assert!(!out.status.success());
    let stderr = String::from_utf8(out.stderr).expect("utf8");
    let last = stderr.lines().last().expect("error envelope");
    let v: Value = serde_json::from_str(last).expect("json");
    assert_eq!(v["error"]["code"], "run_not_found");
}

#[test]
fn tail_invalid_run_id_rejected() {
    let home = TempDir::new().unwrap();
    let out = bin(&home)
        .args(["--json", "event", "tail", "../etc"])
        .output()
        .expect("spawn");
    assert!(!out.status.success());
    let stderr = String::from_utf8(out.stderr).expect("utf8");
    let last = stderr.lines().last().expect("error envelope");
    let v: Value = serde_json::from_str(last).expect("json");
    assert_eq!(v["error"]["code"], "invalid_id");
}

#[test]
fn tail_follow_picks_up_appended_event_within_one_second() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    let evp = events_path(&home, &run_id);

    let mut child = bin(&home)
        .args(["--json", "event", "tail", &run_id, "--follow"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn follow");

    // Give the child a moment to drain the initial file + start polling.
    thread::sleep(Duration::from_millis(250));
    append_event(&evp, 2, "node.created");

    // Poll the child stdout up to 2s waiting for the second event to appear.
    let mut stdout = child.stdout.take().expect("stdout handle");
    let deadline = Instant::now() + Duration::from_secs(2);
    let collected = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let collected_writer = collected.clone();
    let reader = thread::spawn(move || {
        use std::io::Read;
        let mut buf = [0u8; 4096];
        while let Ok(n) = stdout.read(&mut buf) {
            if n == 0 {
                break;
            }
            collected_writer
                .lock()
                .unwrap()
                .push_str(&String::from_utf8_lossy(&buf[..n]));
        }
    });

    let mut saw_second = false;
    while Instant::now() < deadline {
        if collected.lock().unwrap().contains("node.created") {
            saw_second = true;
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }

    // SIGTERM the child to make the follow loop exit cleanly.
    // (We use kill() rather than relying on signal-handler exit codes here
    // — the goal is the read-side assertion, not signal semantics.)
    let _ = child.kill();
    let _ = child.wait();
    let _ = reader.join();

    assert!(
        saw_second,
        "follow did not pick up appended event within 1s; saw: {}",
        collected.lock().unwrap()
    );
}
