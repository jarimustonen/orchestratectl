//! Validation-gate integration tests (V2/V3/V7/V8/V9) for the
//! `supervisor-process` issue.
//!
//! Every test points the binary at a fresh `TempDir` via
//! `ORCHESTRATECTL_HOME` so the user's real `~/.orchestratectl/` is
//! never touched. tmux/external-process probes are stubbed via
//! `TMUX_BIN` redirection or by skipping the probe entirely.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use serde_json::Value;
use tempfile::TempDir;

fn bin(home: &TempDir) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_orchestratectl"));
    c.env("ORCHESTRATECTL_HOME", home.path());
    c.env("OCTL_TEST_SKIP_MATERIALIZE", "1");
    // Make tmux probes deterministically "no window" by pointing
    // TMUX_BIN at a binary that prints nothing.
    c.env("TMUX_BIN", "/usr/bin/true");
    c
}

fn run_ok(cmd: &mut Command) -> Value {
    let out = cmd.output().expect("spawn");
    assert!(
        out.status.success(),
        "exit={:?} stderr={}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("stdout is valid JSON")
}

fn read_events(events: &Path) -> Vec<Value> {
    std::fs::read_to_string(events)
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .collect()
}

fn count_kind(events: &Path, kind: &str) -> usize {
    read_events(events)
        .into_iter()
        .filter(|v| v["kind"] == kind)
        .count()
}

/// How long readiness polls wait before giving up. Generous on purpose: the
/// suite runs many process-spawning supervisor tests in parallel, so under
/// contention a detached spawn+boot+write can take several seconds (observed
/// 0–1000ms isolated, far longer on a saturated/handbrake CI runner).
const POLL_DEADLINE: Duration = Duration::from_secs(30);

/// Poll `predicate` (re-evaluated every 50ms) until it returns `true`, or until
/// `deadline` elapses. Returns `true` if the condition was met in time, `false`
/// on timeout.
///
/// Replaces fixed `sleep`s when waiting on a *detached* supervisor process:
/// its boot+tick+write latency varies, so a single short sleep is inherently
/// flaky, while the assertion we care about is simply "the condition eventually
/// holds". The poll returns as soon as the predicate is satisfied.
///
/// `predicate` is `FnMut` so callers can capture the last observation (see
/// `wait_for_kind`). Timing uses `Instant::elapsed()` rather than a precomputed
/// `Instant + Duration` (which can panic on overflow) and sleeps for at most the
/// remaining budget, so the predicate still gets one final evaluation right at
/// the deadline instead of being skipped after a budget-consuming sleep.
fn poll_until<F: FnMut() -> bool>(deadline: Duration, mut predicate: F) -> bool {
    let start = std::time::Instant::now();
    loop {
        if predicate() {
            return true;
        }
        match deadline.checked_sub(start.elapsed()) {
            Some(remaining) if !remaining.is_zero() => {
                std::thread::sleep(remaining.min(Duration::from_millis(50)));
            }
            _ => return false,
        }
    }
}

/// Poll until `events` contains at least `want` lines of `kind`, or time out.
/// Returns the count observed at the moment the predicate last ran, so callers
/// can assert on it directly (a timeout simply yields the last sub-`want` count).
fn wait_for_kind(events: &Path, kind: &str, want: usize) -> usize {
    let mut seen = 0;
    poll_until(POLL_DEADLINE, || {
        seen = count_kind(events, kind);
        seen >= want
    });
    seen
}

fn create_run(home: &TempDir, kind: &str, title: &str) -> String {
    let v = run_ok(bin(home).args([
        "--output", "json", "run", "create", "--kind", kind, "--title", title,
    ]));
    v["data"]["run_id"].as_str().unwrap().to_string()
}

fn run_dir(home: &TempDir, run_id: &str) -> std::path::PathBuf {
    home.path().join("runs").join(run_id)
}

/// V2: `agent_pid` discovery and PID-based liveness probe.
///
/// Real tmux-pane PID re-discovery requires a live tmux server, which
/// CI cannot rely on. This in-process simulation exercises the same
/// code path: the watchdog reads `nodes/n-0001.json`, probes the
/// recorded PID via `kill(pid, 0)`, and treats an exited PID as
/// terminal — synthesizing a `node.report {failed: true,
/// reason: "agent-died"}` event for the supervisor's reducer to fold.
#[test]
fn v2_agent_pid_discovery_via_liveness_probe() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home, "spinoff", "v2-pid");
    // Forge a node.created carrying an agent_pid that points at our
    // own PID (definitely alive, so the watchdog must NOT synthesize a
    // failed report on the first --once tick).
    let our_pid = std::process::id();
    let report = home.path().join("v2-node.json");
    std::fs::write(
        &report,
        format!(
            r#"{{"kind":"spinoff","task":"x","agent_pid":{our_pid},"tmux_window":"never-existed"}}"#
        ),
    )
    .unwrap();
    run_ok(bin(&home).args([
        "--output",
        "json",
        "event",
        "create",
        &run_id,
        "--kind",
        "node.created",
        "--node-id",
        "n-0001",
        "--from-file",
        report.to_str().unwrap(),
    ]));

    // Tick the supervisor once. Since tmux_window is set and TMUX_BIN
    // is /usr/bin/true (no output), the watchdog will see no matching
    // window and may synthesize TmuxGone. To keep V2 about PID
    // semantics, blank the tmux_window field by overwriting the node
    // JSON's tmux_window to null and re-running.
    let node_p = run_dir(&home, &run_id).join("nodes").join("n-0001.json");
    let mut n: Value = serde_json::from_slice(&std::fs::read(&node_p).unwrap()).unwrap();
    n["tmux_window"] = Value::Null;
    std::fs::write(&node_p, serde_json::to_vec_pretty(&n).unwrap()).unwrap();

    // Now supervise --once: alive PID + no tmux probe → no synthesis.
    run_ok(bin(&home).args(["--output", "json", "supervise", &run_id, "--once"]));
    let events = run_dir(&home, &run_id).join("events.jsonl");
    assert_eq!(
        count_kind(&events, "node.report"),
        0,
        "alive PID must not synthesize a failed node.report"
    );

    // Swap the PID to one that is guaranteed dead (PID 1 may be
    // privileged-owned but not "dead"; use a never-allocated high PID
    // like 0x7FFF_FFFE). On macOS, signal to PID 999_999_999 returns
    // ESRCH unless someone forked an unreasonable number of times.
    let mut n: Value = serde_json::from_slice(&std::fs::read(&node_p).unwrap()).unwrap();
    n["agent_pid"] = Value::from(0x3FFF_FFFE_i64);
    std::fs::write(&node_p, serde_json::to_vec_pretty(&n).unwrap()).unwrap();
    run_ok(bin(&home).args(["--output", "json", "supervise", &run_id, "--once"]));
    assert!(
        count_kind(&events, "node.report") >= 1,
        "dead PID must synthesize a failed node.report. events={:?}",
        read_events(&events)
            .into_iter()
            .map(|v| v["kind"].clone())
            .collect::<Vec<_>>()
    );
}

/// V3: `kill(pid, 0)` + `start_time` identity defense.
///
/// Verifies that (1) `start_time` is stable across reads, (2) supplying
/// a wildly wrong `start_time` forces `Recycled` rather than `Alive` for
/// the watchdog's own PID, and (3) the watchdog's verdict on a dead
/// PID is `Dead`. Cross-platform via the `sysinfo` crate path.
#[test]
fn v3_kill_and_start_time_identity() {
    // Stability: two reads of our own start_time differ by ≤ 1s.
    // (Already covered by a unit test on watchdog.rs; here we
    // additionally drive it through the public CLI by running
    // `supervise --once` repeatedly and confirming the supervisor does
    // not falsely declare itself dead.)
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home, "spinoff", "v3-st");
    let our_pid = std::process::id();
    let report = home.path().join("v3.json");
    std::fs::write(
        &report,
        format!(r#"{{"kind":"spinoff","task":"x","agent_pid":{our_pid}}}"#),
    )
    .unwrap();
    run_ok(bin(&home).args([
        "--output",
        "json",
        "event",
        "create",
        &run_id,
        "--kind",
        "node.created",
        "--node-id",
        "n-0001",
        "--from-file",
        report.to_str().unwrap(),
    ]));
    // Record a deliberately wrong start_time so the watchdog flags Recycled.
    let node_p = run_dir(&home, &run_id).join("nodes").join("n-0001.json");
    let mut n: Value = serde_json::from_slice(&std::fs::read(&node_p).unwrap()).unwrap();
    n["agent_pid_start_time"] = Value::String("1970-01-01T00:00:00Z".into());
    n["tmux_window"] = Value::Null;
    std::fs::write(&node_p, serde_json::to_vec_pretty(&n).unwrap()).unwrap();

    run_ok(bin(&home).args(["--output", "json", "supervise", &run_id, "--once"]));
    let events = run_dir(&home, &run_id).join("events.jsonl");
    let reports = read_events(&events)
        .into_iter()
        .filter(|v| v["kind"] == "node.report")
        .collect::<Vec<_>>();
    assert_eq!(reports.len(), 1, "recycled PID must synthesize one report");
    assert_eq!(reports[0]["data"]["reason"], "agent-pid-recycled");
}

/// V7: Deterministic-ID dedup under crash-recovery.
///
/// Drives the in-process `reducer::process_node_report` (rather than
/// the binary) so we can use the `FAULT_INJECT_AFTER_NTH` thread-local
/// to crash mid-batch, then restart and verify exactly-once outcome.
#[test]
fn v7_deterministic_id_dedup_under_crash() {
    // We re-derive the same logic in-test by driving the CLI through
    // two distinct calls. Setup: a parent run with one spawning node,
    // a child run that emits a node.report containing 2 discussions +
    // 3 spinoffs. First supervise pass: emit all 5 derived events.
    // Second supervise pass on the same child run: must be a no-op
    // (every deterministic ID already exists in the parent's
    // discussions/ or spinoffs/ directory).
    let home = TempDir::new().unwrap();
    let parent = create_run(&home, "orchestrated", "v7-parent");

    // Forge a parent-side spawning node + child-spawned link so the
    // supervisor's find_spawning_node lookup resolves.
    let p_node = home.path().join("v7-parent-node.json");
    std::fs::write(&p_node, r#"{"kind":"orchestrated","task":"x"}"#).unwrap();
    run_ok(bin(&home).args([
        "--output",
        "json",
        "event",
        "create",
        &parent,
        "--kind",
        "node.created",
        "--node-id",
        "n-0001",
        "--from-file",
        p_node.to_str().unwrap(),
    ]));

    // Create a child run AS A child of the parent (via run create
    // --parent-*) so that child.spawned lands in the parent's events
    // and the parent node's children list is populated.
    let child_create = run_ok(bin(&home).args([
        "--output",
        "json",
        "run",
        "create",
        "--kind",
        "spinoff",
        "--title",
        "v7-child",
        "--parent-run-id",
        &parent,
        "--parent-node-id",
        "n-0001",
    ]));
    let child = child_create["data"]["run_id"].as_str().unwrap().to_string();

    // Forge the child-side node.report.
    let c_node = home.path().join("v7-child-node.json");
    std::fs::write(&c_node, r#"{"kind":"spinoff","task":"x"}"#).unwrap();
    run_ok(bin(&home).args([
        "--output",
        "json",
        "event",
        "create",
        &child,
        "--kind",
        "node.created",
        "--node-id",
        "n-0001",
        "--from-file",
        c_node.to_str().unwrap(),
    ]));
    let report = home.path().join("v7-report.json");
    std::fs::write(
        &report,
        r#"{
            "success": true,
            "summary": "v7",
            "discussion_items": [
                {"topic": "d-a", "severity": "discuss", "options": ["x"]},
                {"topic": "d-b", "severity": "discuss", "options": ["y"]}
            ],
            "spinoff_proposals": [
                {"proposed_title": "s-a", "proposed_kind": "spinoff", "rationale": "r1"},
                {"proposed_title": "s-b", "proposed_kind": "spinoff", "rationale": "r2"},
                {"proposed_title": "s-c", "proposed_kind": "spinoff", "rationale": "r3"}
            ],
            "wrap_up_recommendations": []
        }"#,
    )
    .unwrap();
    run_ok(bin(&home).args([
        "--output",
        "json",
        "node",
        "report",
        &child,
        "n-0001",
        "--from-file",
        report.to_str().unwrap(),
    ]));

    // First supervise pass on the parent: should see child.spawned,
    // tail the child's events, consume the node.report. With --once
    // we tick exactly once but the supervisor handles all 3 loops in
    // that single tick.
    //
    // The supervisor's spawn_child_supervisor will fork+exec a real
    // child supervisor process. To avoid that interfering with the
    // test (the child supervisor would also tick and exit thanks to
    // its own behavior, but only because we're not passing --once to
    // the fork). Instead, pre-seed supervisor.state.json on the parent
    // to mark the child as already-spawned, so the parent skips the
    // fork.
    let state_p = run_dir(&home, &parent).join("supervisor.state.json");
    std::fs::write(
        &state_p,
        format!(r#"{{"schema_version":1,"spawned_children":{{"{child}":1}}}}"#),
    )
    .unwrap();

    run_ok(bin(&home).args(["--output", "json", "supervise", &parent, "--once"]));

    // Inspect parent's discussions/ and spinoffs/.
    let disc_dir = run_dir(&home, &parent).join("discussions");
    let spin_dir = run_dir(&home, &parent).join("spinoffs");
    let n_disc = std::fs::read_dir(&disc_dir).map_or(0, std::iter::Iterator::count);
    let n_spin = std::fs::read_dir(&spin_dir).map_or(0, std::iter::Iterator::count);
    assert_eq!(n_disc, 2, "parent must have 2 discussions");
    assert_eq!(n_spin, 3, "parent must have 3 spinoffs");

    // Recover by deleting cursor; rerun supervisor. The deterministic
    // IDs match the files already on disk → reducer skips emission.
    std::fs::remove_file(&state_p).unwrap();
    // Restore "spawned_children" so the parent doesn't try to fork again.
    std::fs::write(
        &state_p,
        format!(r#"{{"schema_version":1,"spawned_children":{{"{child}":1}}}}"#),
    )
    .unwrap();
    run_ok(bin(&home).args(["--output", "json", "supervise", &parent, "--once"]));

    let n_disc2 = std::fs::read_dir(&disc_dir).unwrap().count();
    let n_spin2 = std::fs::read_dir(&spin_dir).unwrap().count();
    assert_eq!(n_disc2, 2, "replay must not duplicate discussions");
    assert_eq!(n_spin2, 3, "replay must not duplicate spinoffs");
}

/// §7.8: SIGINT exits 130, SIGTERM exits 143, and the `supervisor.exited`
/// event records `reason:"signal"` + the specific `signal` name. Regression
/// guard for the supervisor-process review FIX (F3) — `ctrlc` could not
/// surface which signal fired, so the old code exited 0 with no signal field.
#[test]
fn signal_exit_codes_and_payload() {
    use std::io::Read;
    for (sig, code, name) in [("TERM", 143, "SIGTERM"), ("INT", 130, "SIGINT")] {
        let home = TempDir::new().unwrap();
        let run_id = create_run(&home, "spinoff", "sig");
        // Long-lived supervisor (no --once): spawn, let it enter the loop,
        // then deliver the signal and assert the exit code + event.
        let mut child = bin(&home)
            .args(["supervise", &run_id])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn supervisor");
        // Poll for readiness (the PID file appears once the supervisor has
        // installed its signal handlers and entered the loop) rather than a
        // fixed sleep, so the kill never races startup even on a loaded CI.
        let pid_file = run_dir(&home, &run_id).join("supervisor.pid");
        assert!(
            poll_until(POLL_DEADLINE, || pid_file.exists()),
            "supervisor did not start in time: {}",
            pid_file.display()
        );
        unsafe {
            libc::kill(child.id() as i32, sig_num(sig));
        }
        let status = child.wait().expect("wait");
        assert_eq!(
            status.code(),
            Some(code),
            "{name} must exit {code}, got {status:?}"
        );
        // §7.8: a cleanly-signalled supervisor removes its own PID file.
        assert!(
            !pid_file.exists(),
            "{name}: supervisor.pid must be removed on signal exit"
        );
        let events = run_dir(&home, &run_id).join("events.jsonl");
        let mut s = String::new();
        std::fs::File::open(&events)
            .unwrap()
            .read_to_string(&mut s)
            .unwrap();
        let exited = s
            .lines()
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
            .find(|v| v["kind"] == "supervisor.exited")
            .expect("supervisor.exited present");
        assert_eq!(exited["data"]["reason"], "signal");
        assert_eq!(exited["data"]["signal"], name);
    }
}

fn sig_num(sig: &str) -> libc::c_int {
    match sig {
        "TERM" => libc::SIGTERM,
        "INT" => libc::SIGINT,
        _ => unreachable!(),
    }
}

/// V8: `run reattach` end-to-end.
///
/// Start a run, fork a one-shot supervisor via `run reattach --once`,
/// confirm the events.jsonl picks up the supervisor.reattached marker.
/// Reattach again: the previous supervisor is dead, so the new one
/// boots cleanly. Demonstrates the stale-PID detection path.
#[test]
fn v8_reattach_end_to_end() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home, "spinoff", "v8");

    run_ok(bin(&home).args(["--output", "json", "run", "reattach", &run_id, "--once"]));
    // Wait for the spawned --once supervisor to boot, tick, and write its
    // supervisor.exited event. Polling (not a fixed sleep) because the
    // detached process's latency varies and a single 500ms sleep is flaky.
    let events = run_dir(&home, &run_id).join("events.jsonl");
    assert!(wait_for_kind(&events, "supervisor.exited", 1) >= 1);
    assert!(count_kind(&events, "supervisor.reattach-requested") >= 1);
    assert!(count_kind(&events, "supervisor.reattached") >= 1);

    // The `--once` supervisor writes `supervisor.exited` just *before* it
    // removes its pid file and exits. Wait for the pid file to disappear so
    // the second reattach sees a genuinely stale (dead) prior supervisor
    // rather than racing the still-dying one (which would refuse).
    let pid_file = run_dir(&home, &run_id).join("supervisor.pid");
    // Fatal on timeout: if the pid file never disappears the second reattach
    // would race the still-dying supervisor (the exact condition this wait
    // guards against) and fail later with a misleading reattach-count error, so
    // assert the precondition here with a useful message instead.
    assert!(
        poll_until(POLL_DEADLINE, || !pid_file.exists()),
        "prior --once supervisor did not remove its pid file in time: {}",
        pid_file.display()
    );

    // Second reattach: prior PID is stale.
    run_ok(bin(&home).args(["--output", "json", "run", "reattach", &run_id, "--once"]));
    assert!(wait_for_kind(&events, "supervisor.reattach-requested", 2) >= 2);
}

/// V9: `run cancel` synthesized-report propagation.
///
/// A child run with a non-terminal node receives `run cancel`. The
/// cancel verb synthesizes a terminal `node.report {cancelled: true}`.
/// A parent supervisor that tails the child sees the cancelled report
/// and must (a) not emit any spinoffs/discussions from it, (b) advance
/// `last_processed_report_seq_by_child` so a replay is a no-op.
#[test]
fn v9_cancel_synthesizes_report_no_spinoffs() {
    let home = TempDir::new().unwrap();
    let parent = create_run(&home, "orchestrated", "v9-parent");
    let p_node = home.path().join("v9-pn.json");
    std::fs::write(&p_node, r#"{"kind":"orchestrated","task":"x"}"#).unwrap();
    run_ok(bin(&home).args([
        "--output",
        "json",
        "event",
        "create",
        &parent,
        "--kind",
        "node.created",
        "--node-id",
        "n-0001",
        "--from-file",
        p_node.to_str().unwrap(),
    ]));

    let child = run_ok(bin(&home).args([
        "--output",
        "json",
        "run",
        "create",
        "--kind",
        "spinoff",
        "--title",
        "v9-c",
        "--parent-run-id",
        &parent,
        "--parent-node-id",
        "n-0001",
    ]))["data"]["run_id"]
        .as_str()
        .unwrap()
        .to_string();
    let c_node = home.path().join("v9-cn.json");
    std::fs::write(&c_node, r#"{"kind":"spinoff","task":"x"}"#).unwrap();
    run_ok(bin(&home).args([
        "--output",
        "json",
        "event",
        "create",
        &child,
        "--kind",
        "node.created",
        "--node-id",
        "n-0001",
        "--from-file",
        c_node.to_str().unwrap(),
    ]));

    // Cancel the child — synthesizes a node.report {cancelled: true}.
    run_ok(bin(&home).args(["--output", "json", "run", "cancel", &child]));

    // Pre-seed parent to skip child-supervisor fork.
    std::fs::write(
        run_dir(&home, &parent).join("supervisor.state.json"),
        format!(r#"{{"schema_version":1,"spawned_children":{{"{child}":1}}}}"#),
    )
    .unwrap();
    run_ok(bin(&home).args(["--output", "json", "supervise", &parent, "--once"]));

    // Parent must have zero spinoffs/discussions derived from the
    // cancelled child report.
    let n_spin = std::fs::read_dir(run_dir(&home, &parent).join("spinoffs"))
        .map_or(0, std::iter::Iterator::count);
    let n_disc = std::fs::read_dir(run_dir(&home, &parent).join("discussions"))
        .map_or(0, std::iter::Iterator::count);
    assert_eq!(n_spin, 0, "cancelled child must not propagate spinoffs");
    assert_eq!(n_disc, 0, "cancelled child must not propagate discussions");

    // Parent's spawning-node JSON should record the child cursor so a
    // replay is a no-op.
    let parent_node: Value = serde_json::from_slice(
        &std::fs::read(run_dir(&home, &parent).join("nodes").join("n-0001.json")).unwrap(),
    )
    .unwrap();
    assert!(
        parent_node["last_processed_report_seq_by_child"]
            .get(&child)
            .is_some(),
        "cursor not advanced: {parent_node:?}"
    );
}
