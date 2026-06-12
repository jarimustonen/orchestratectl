//! Integration tests for `event tail`.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
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

fn append_event(path: &Path, run_id: &str, seq: u64, kind: &str) {
    let line = serde_json::json!({
        "ts": "2026-06-12T10:00:00Z",
        "seq": seq,
        "kind": kind,
        "run_id": run_id,
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
    append_event(&evp, &run_id, 2, "node.created");
    append_event(&evp, &run_id, 3, "node.report");

    let (stdout, _) = run_ok_output(bin(&home).args(["--json", "event", "tail", &run_id]));
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 4, "stdout was: {stdout}");
    let last: Value = serde_json::from_str(lines.last().unwrap()).expect("terminal json");
    assert_eq!(last["event"], "result");
    assert_eq!(last["status"], "ok");
    assert_eq!(last["schema_version"], 1);
    assert_eq!(last["last_seq"], 3);
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
    append_event(&evp, &run_id, 2, "a");
    append_event(&evp, &run_id, 3, "b");

    let (stdout, _) =
        run_ok_output(bin(&home).args(["--json", "event", "tail", &run_id, "--from-seq", "3"]));
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "stdout was: {stdout}");
    let only: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(only["seq"], 3);
    let terminal: Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(terminal["event"], "result");
    // Only seq=3 was emitted, but last_seq reflects every event we
    // saw (seq=1, 2, 3) so the dedup state is correct on follow.
    assert_eq!(terminal["last_seq"], 3);
}

#[test]
fn tail_from_seq_zero_includes_seq_one() {
    // Regression: a previous implementation used `saturating_sub(1)` and
    // would have skipped seq=0 events. We don't currently emit seq=0,
    // but verify the default --from-seq=0 includes the first event.
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    let (stdout, _) =
        run_ok_output(bin(&home).args(["--json", "event", "tail", &run_id, "--from-seq", "0"]));
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "stdout was: {stdout}");
    let first: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(first["seq"], 1);
}

#[test]
fn tail_text_format_no_terminal_envelope() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    let (stdout, _) =
        run_ok_output(bin(&home).args(["event", "tail", &run_id, "--format", "text"]));
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
fn tail_json_and_format_text_conflict_is_rejected() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    let out = bin(&home)
        .args(["--json", "event", "tail", &run_id, "--format", "text"])
        .output()
        .expect("spawn");
    assert!(!out.status.success());
    let stderr = String::from_utf8(out.stderr).expect("utf8");
    let last = stderr.lines().last().expect("error envelope");
    let v: Value = serde_json::from_str(last).expect("json");
    assert_eq!(v["error"]["code"], "conflicting_arguments");
}

#[test]
fn tail_output_file_truncate_without_follow() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    let out_path = home.path().join("captured.jsonl");
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
fn tail_output_aliasing_events_file_is_rejected() {
    // Without the guard, --output <events.jsonl> would truncate the
    // canonical event log (no-follow) — silent data loss.
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    let evp = events_path(&home, &run_id);

    let pre = std::fs::read_to_string(&evp).expect("events file exists");
    assert!(pre.contains("run.created"));

    let out = bin(&home)
        .args([
            "--json",
            "event",
            "tail",
            &run_id,
            "--output",
            evp.to_str().unwrap(),
        ])
        .output()
        .expect("spawn");
    assert!(!out.status.success());
    let stderr = String::from_utf8(out.stderr).expect("utf8");
    let last = stderr.lines().last().expect("error envelope");
    let v: Value = serde_json::from_str(last).expect("json");
    assert_eq!(v["error"]["code"], "invalid_output");

    // The canonical log must still contain the original event.
    let post = std::fs::read_to_string(&evp).expect("events file still readable");
    assert_eq!(pre, post, "events.jsonl was modified despite alias guard");
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
fn tail_partial_line_is_held_until_newline_arrives() {
    // Append the first half of a line (no \n), start tail with --follow,
    // verify it doesn't crash or emit garbage, then complete the line
    // and verify the event is emitted exactly once.
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    let evp = events_path(&home, &run_id);

    // Write a partial JSON line.
    let partial = format!(
        "{{\"ts\":\"2026-06-12T10:00:00Z\",\"seq\":2,\"kind\":\"partial.test\",\"run_id\":\"{}\",\"node_id\":null,\"idempotency_key\":null,\"data\":",
        run_id
    );
    let mut f = OpenOptions::new().append(true).open(&evp).expect("open");
    f.write_all(partial.as_bytes()).unwrap();
    f.sync_all().unwrap();
    drop(f);

    let mut child = bin(&home)
        .args(["--json", "event", "tail", &run_id, "--follow"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn follow");

    let collected = read_stdout_in_background(&mut child);

    // Wait one poll cycle and a bit — the partial must NOT have been emitted.
    thread::sleep(Duration::from_millis(800));

    // Complete the line.
    let completion = "{\"status\":\"ok\"}}\n";
    let mut f = OpenOptions::new().append(true).open(&evp).expect("open");
    f.write_all(completion.as_bytes()).unwrap();
    f.sync_all().unwrap();
    drop(f);

    let saw = wait_for_substring(&collected, "partial.test", Duration::from_secs(2));
    let _ = send_sigint(&child);
    let _ = child.wait();

    let body = collected.lock().unwrap().clone();
    assert!(saw, "partial.test never emitted; stdout was: {body}");
    // Make sure only one occurrence of the completed event.
    assert_eq!(
        body.matches("\"partial.test\"").count(),
        1,
        "stdout: {body}"
    );
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

    let collected = read_stdout_in_background(&mut child);

    // Wait until the initial drain has emitted run.created so we know
    // the child is in the poll loop (vs racing startup).
    assert!(
        wait_for_substring(&collected, "run.created", Duration::from_secs(2)),
        "initial drain didn't emit"
    );
    append_event(&evp, &run_id, 2, "node.created");
    let saw = wait_for_substring(&collected, "node.created", Duration::from_millis(1500));

    let _ = send_sigint(&child);
    let _ = child.wait();
    assert!(
        saw,
        "follow did not pick up appended event in time; stdout: {}",
        collected.lock().unwrap()
    );
}

#[cfg(unix)]
#[test]
fn tail_follow_emits_cancelled_envelope_on_sigint() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);

    let mut child = bin(&home)
        .args(["--json", "event", "tail", &run_id, "--follow"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn follow");

    let collected = read_stdout_in_background(&mut child);
    assert!(wait_for_substring(
        &collected,
        "run.created",
        Duration::from_secs(2)
    ));

    send_sigint(&child).expect("send SIGINT");
    let status = child.wait().expect("wait");
    assert_eq!(
        status.code(),
        Some(130),
        "expected exit 130, got {:?}; stdout: {}",
        status,
        collected.lock().unwrap()
    );

    let body = collected.lock().unwrap().clone();
    let last_line = body
        .lines()
        .rfind(|l| !l.is_empty())
        .expect("at least one stdout line");
    let v: Value = serde_json::from_str(last_line).expect("terminal json");
    assert_eq!(v["event"], "cancelled");
    assert_eq!(v["schema_version"], 1);
}

#[cfg(unix)]
#[test]
fn tail_follow_detects_log_truncation() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    let evp = events_path(&home, &run_id);
    append_event(&evp, &run_id, 2, "x");

    let mut child = bin(&home)
        .args(["--json", "event", "tail", &run_id, "--follow"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn follow");

    let collected = read_stdout_in_background(&mut child);
    assert!(wait_for_substring(
        &collected,
        "\"seq\":2",
        Duration::from_secs(2)
    ));

    // Truncate the file we share with the child's fd. `set_len(0)` on
    // a separately-opened handle still shrinks the inode the child is
    // reading from.
    let f = OpenOptions::new().write(true).open(&evp).expect("open rw");
    f.set_len(0).expect("truncate");
    drop(f);

    let stderr = child.stderr.take().expect("stderr");
    let stderr_str = read_to_string(stderr);
    let status = child.wait().expect("wait");
    assert!(!status.success(), "expected failure exit");
    let last = stderr_str
        .lines()
        .rfind(|l| !l.is_empty())
        .expect("error envelope");
    let v: Value = serde_json::from_str(last).expect("error envelope json");
    assert_eq!(v["error"]["code"], "events_log_truncated");
}

// -- helpers ---------------------------------------------------------------

fn read_stdout_in_background(child: &mut Child) -> std::sync::Arc<std::sync::Mutex<String>> {
    let stdout = child.stdout.take().expect("stdout");
    let collected = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let writer = collected.clone();
    thread::spawn(move || {
        use std::io::Read;
        let mut buf = [0u8; 4096];
        let mut s = stdout;
        while let Ok(n) = s.read(&mut buf) {
            if n == 0 {
                break;
            }
            writer
                .lock()
                .unwrap()
                .push_str(&String::from_utf8_lossy(&buf[..n]));
        }
    });
    collected
}

fn read_to_string<R: std::io::Read + Send + 'static>(mut r: R) -> String {
    let mut s = String::new();
    let _ = r.read_to_string(&mut s);
    s
}

fn wait_for_substring(
    collected: &std::sync::Arc<std::sync::Mutex<String>>,
    needle: &str,
    timeout: Duration,
) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if collected.lock().unwrap().contains(needle) {
            return true;
        }
        thread::sleep(Duration::from_millis(50));
    }
    false
}

#[cfg(unix)]
fn send_sigint(child: &Child) -> std::io::Result<()> {
    let pid = child.id() as i32;
    let rc = unsafe { libc::kill(pid, libc::SIGINT) };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(unix))]
fn send_sigint(child: &Child) -> std::io::Result<()> {
    // On non-Unix, fall back to kill() (SIGKILL-equivalent). The
    // cancelled-envelope assertion will not hold, so the signal test is
    // gated to `cfg(unix)` above.
    let mut c = unsafe { std::ptr::read(child as *const Child) };
    c.kill()
}
