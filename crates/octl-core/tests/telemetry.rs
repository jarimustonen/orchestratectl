use std::fs;

use chrono::{DateTime, Duration, Utc};
use octl_core::{
    parse_telemetry_update, read_node, read_telemetry_with_clock, update_telemetry_with_clock,
    RunPaths, TelemetryClock, TelemetryError, TelemetrySampleStatus, TelemetryState,
    TelemetryUpdate, TELEMETRY_MAX_BYTES,
};
use serde_json::{json, Value};
use tempfile::TempDir;

const RUN: &str = "01jxsnap000000000000000000";
const NODE: &str = "n-0001";

#[derive(Clone, Copy)]
struct FixedClock(DateTime<Utc>);
impl TelemetryClock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

fn time(seconds: i64) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-08-24T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
        + Duration::seconds(seconds)
}

fn node_value(status: &str, attempt: u32) -> Value {
    json!({
        "schema_version": 1,
        "node_id": NODE,
        "run_id": RUN,
        "parent_node_id": null,
        "kind": "spinoff",
        "status": status,
        "task": "fixture",
        "worktree_path": "/tmp/work",
        "branch": "wt/fixture",
        "base_sha": null,
        "tmux_window": "fixture",
        "tmux_identity": null,
        "agent_pid": 123,
        "agent_pid_start_time": null,
        "supervisor_pid": 456,
        "children": [],
        "started_at": "2026-08-24T11:00:00Z",
        "updated_at": "2026-08-24T11:00:00Z",
        "last_report": null,
        "last_processed_report_seq_by_child": {},
        "retry_attempts": attempt,
        "worker_exit": null,
        "pending_merge": null,
        "first_death_at": null,
        "awaiting_input": null
    })
}

fn manifest_value(applied_seq: u64) -> Value {
    json!({
        "schema_version":1,"applied_seq":applied_seq,"run_id":RUN,"kind":"spinoff",
        "lifecycle":"autonomous","title":"fixture","status":"running",
        "created_at":"2026-08-24T11:00:00Z","updated_at":"2026-08-24T11:00:00Z",
        "source_repo":null,"source_branch":null,"worktree_root":null,
        "managed_tmux_session":null,"notify_cmd":null,"harness":"pi","node_count":1,
        "parent_run_id":null,"parent_node_id":null
    })
}

fn canonical_events() -> Vec<u8> {
    let events = [
        json!({"ts":"2026-08-24T11:00:00Z","seq":1,"kind":"run.created","run_id":RUN,
            "data":{"kind":"spinoff","lifecycle":"autonomous","title":"fixture"}}),
        json!({"ts":"2026-08-24T11:00:01Z","seq":2,"kind":"node.created","run_id":RUN,
            "node_id":NODE,"data":{"kind":"spinoff","task":"fixture"}}),
        json!({"ts":"2026-08-24T11:00:02Z","seq":3,"kind":"node.status","run_id":RUN,
            "node_id":NODE,"data":{"status":"running"}}),
        json!({"ts":"2026-08-24T11:00:03Z","seq":4,"kind":"run.status","run_id":RUN,
            "data":{"status":"running"}}),
    ];
    let mut bytes = Vec::new();
    for event in events {
        bytes.extend(serde_json::to_vec(&event).unwrap());
        bytes.push(b'\n');
    }
    bytes
}

fn setup(status: &str, attempt: u32) -> (TempDir, RunPaths) {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("run");
    fs::create_dir_all(root.join("nodes")).unwrap();
    fs::write(root.join(".lock"), []).unwrap();
    fs::write(
        root.join("nodes/n-0001.json"),
        serde_json::to_vec(&node_value(status, attempt)).unwrap(),
    )
    .unwrap();
    fs::write(
        root.join("manifest.json"),
        serde_json::to_vec(&manifest_value(4)).unwrap(),
    )
    .unwrap();
    fs::write(root.join("events.jsonl"), canonical_events()).unwrap();
    let paths = RunPaths::new(root, RUN).unwrap();
    (temp, paths)
}

fn update(attempt: u32, state: TelemetryState) -> TelemetryUpdate {
    TelemetryUpdate {
        schema_version: 1,
        protocol_version: 1,
        run_id: RUN.parse().unwrap(),
        node_id: NODE.parse().unwrap(),
        attempt,
        state,
        active_tool_count: None,
        tool_name: None,
    }
}

fn sample_path(paths: &RunPaths) -> std::path::PathBuf {
    paths.root.join("telemetry/n-0001.json")
}

fn stored(paths: &RunPaths) -> Value {
    serde_json::from_slice(&fs::read(sample_path(paths)).unwrap()).unwrap()
}

#[test]
fn strict_request_rejects_unknown_fields_versions_metadata_and_size() {
    let valid = json!({
        "schema_version": 1,
        "protocol_version": 1,
        "run_id": RUN,
        "node_id": NODE,
        "attempt": 0,
        "state": "tool_running",
        "active_tool_count": 1,
        "tool_name": "functions.bash"
    });
    assert!(parse_telemetry_update(&serde_json::to_vec(&valid).unwrap()).is_ok());

    let mut unknown = valid.clone();
    unknown["outcome"] = json!("done");
    assert!(matches!(
        parse_telemetry_update(&serde_json::to_vec(&unknown).unwrap()),
        Err(TelemetryError::InvalidRequest(_))
    ));

    for (field, value) in [("schema_version", 2), ("protocol_version", 2)] {
        let mut wrong = valid.clone();
        wrong[field] = json!(value);
        assert!(parse_telemetry_update(&serde_json::to_vec(&wrong).unwrap()).is_err());
    }

    for bad in [
        json!({"schema_version":1,"protocol_version":1,"run_id":RUN,"node_id":NODE,"attempt":0,"state":"settled","active_tool_count":1}),
        json!({"schema_version":1,"protocol_version":1,"run_id":RUN,"node_id":NODE,"attempt":0,"state":"tool_running","active_tool_count":0}),
        json!({"schema_version":1,"protocol_version":1,"run_id":RUN,"node_id":NODE,"attempt":0,"state":"tool_running","active_tool_count":2,"tool_name":"bash"}),
        json!({"schema_version":1,"protocol_version":1,"run_id":RUN,"node_id":NODE,"attempt":0,"state":"tool_running","active_tool_count":1,"tool_name":"bash /tmp/secret"}),
    ] {
        assert!(matches!(
            parse_telemetry_update(&serde_json::to_vec(&bad).unwrap()),
            Err(TelemetryError::InvalidMetadata(_))
        ));
    }

    let oversized = vec![b' '; TELEMETRY_MAX_BYTES + 1];
    assert!(matches!(
        parse_telemetry_update(&oversized),
        Err(TelemetryError::TooLarge {
            what: "request",
            ..
        })
    ));
}

#[test]
fn update_atomically_replaces_one_bounded_sample_and_preserves_state_since() {
    let (_temp, paths) = setup("running", 0);
    let first = update(0, TelemetryState::ToolRunning);
    update_telemetry_with_clock(&paths, &first, &FixedClock(time(0))).unwrap();
    let first_json = stored(&paths);
    assert!(fs::metadata(sample_path(&paths)).unwrap().len() <= TELEMETRY_MAX_BYTES as u64);
    assert_eq!(first_json["state_since"], "2026-08-24T12:00:00Z");
    assert_eq!(first_json["received_at"], "2026-08-24T12:00:00Z");
    assert_eq!(first_json["expires_at"], "2026-08-24T12:01:30Z");

    update_telemetry_with_clock(&paths, &first, &FixedClock(time(30))).unwrap();
    let refreshed = stored(&paths);
    assert_eq!(refreshed["state_since"], first_json["state_since"]);
    assert_eq!(refreshed["received_at"], "2026-08-24T12:00:30Z");

    let changed = update(0, TelemetryState::Settled);
    update_telemetry_with_clock(&paths, &changed, &FixedClock(time(40))).unwrap();
    assert_eq!(stored(&paths)["state_since"], "2026-08-24T12:00:40Z");

    let entries: Vec<_> = fs::read_dir(paths.root.join("telemetry"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(entries, [std::ffi::OsString::from("n-0001.json")]);
}

#[test]
fn missing_corrupt_partial_old_attempt_and_freshness_are_classified() {
    let (_temp, paths) = setup("running", 0);
    let node_id = NODE.parse().unwrap();
    assert_eq!(
        read_telemetry_with_clock(&paths, &node_id, &FixedClock(time(0)))
            .unwrap()
            .sample,
        TelemetrySampleStatus::Absent
    );

    update_telemetry_with_clock(
        &paths,
        &update(0, TelemetryState::AgentActive),
        &FixedClock(time(0)),
    )
    .unwrap();
    assert_eq!(
        read_telemetry_with_clock(&paths, &node_id, &FixedClock(time(89)))
            .unwrap()
            .sample,
        TelemetrySampleStatus::Current
    );
    assert_eq!(
        read_telemetry_with_clock(&paths, &node_id, &FixedClock(time(90)))
            .unwrap()
            .sample,
        TelemetrySampleStatus::Stale
    );
    let backward = read_telemetry_with_clock(&paths, &node_id, &FixedClock(time(-1))).unwrap();
    assert_eq!(backward.sample, TelemetrySampleStatus::ClockUnreliable);
    assert_eq!(backward.age_ms, None);

    fs::write(sample_path(&paths), b"{\"schema_version\":1").unwrap();
    assert_eq!(
        read_telemetry_with_clock(&paths, &node_id, &FixedClock(time(0)))
            .unwrap()
            .sample,
        TelemetrySampleStatus::Invalid
    );

    update_telemetry_with_clock(
        &paths,
        &update(0, TelemetryState::Settled),
        &FixedClock(time(10)),
    )
    .unwrap();
    fs::write(
        paths.node(&node_id),
        serde_json::to_vec(&node_value("running", 1)).unwrap(),
    )
    .unwrap();
    assert_eq!(
        read_telemetry_with_clock(&paths, &node_id, &FixedClock(time(10)))
            .unwrap()
            .sample,
        TelemetrySampleStatus::Absent
    );
}

#[test]
fn attempt_change_resets_state_since_even_when_state_is_unchanged() {
    let (_temp, paths) = setup("running", 0);
    let same_state = TelemetryState::ToolRunning;
    update_telemetry_with_clock(&paths, &update(0, same_state), &FixedClock(time(0))).unwrap();
    let node_id = NODE.parse().unwrap();
    fs::write(
        paths.node(&node_id),
        serde_json::to_vec(&node_value("running", 1)).unwrap(),
    )
    .unwrap();
    update_telemetry_with_clock(&paths, &update(1, same_state), &FixedClock(time(20))).unwrap();
    let value = stored(&paths);
    assert_eq!(value["attempt"], 1);
    assert_eq!(value["state_since"], "2026-08-24T12:00:20Z");
}

#[test]
fn unknown_terminal_and_wrong_attempt_rejections_do_not_mutate_sample() {
    let (_temp, paths) = setup("running", 0);
    update_telemetry_with_clock(
        &paths,
        &update(0, TelemetryState::AgentActive),
        &FixedClock(time(0)),
    )
    .unwrap();
    let before = fs::read(sample_path(&paths)).unwrap();

    let wrong = update(1, TelemetryState::Settled);
    assert!(matches!(
        update_telemetry_with_clock(&paths, &wrong, &FixedClock(time(1))),
        Err(TelemetryError::AttemptMismatch { .. })
    ));
    assert_eq!(fs::read(sample_path(&paths)).unwrap(), before);

    let node_id = NODE.parse().unwrap();
    fs::write(
        paths.node(&node_id),
        serde_json::to_vec(&node_value("done", 0)).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        update_telemetry_with_clock(
            &paths,
            &update(0, TelemetryState::Shutdown),
            &FixedClock(time(2))
        ),
        Err(TelemetryError::TerminalNode { .. })
    ));
    assert_eq!(fs::read(sample_path(&paths)).unwrap(), before);

    let mut missing = update(0, TelemetryState::Settled);
    missing.node_id = "n-9999".parse().unwrap();
    assert!(matches!(
        update_telemetry_with_clock(&paths, &missing, &FixedClock(time(3))),
        Err(TelemetryError::NodeNotFound { .. })
    ));
    assert!(!paths.root.join("telemetry/n-9999.json").exists());
}

#[test]
fn corrupt_prior_is_replaced_instead_of_poisoning_the_endpoint() {
    let (_temp, paths) = setup("running", 0);
    fs::create_dir_all(paths.root.join("telemetry")).unwrap();
    fs::write(sample_path(&paths), b"partial").unwrap();
    update_telemetry_with_clock(
        &paths,
        &update(0, TelemetryState::Settled),
        &FixedClock(time(5)),
    )
    .unwrap();
    assert_eq!(stored(&paths)["state"], "settled");
}

#[test]
fn telemetry_has_no_event_projection_outcome_wait_merge_retry_or_cleanup_effect() {
    let (_temp, paths) = setup("running", 0);
    let node_id = NODE.parse().unwrap();
    let node_before = fs::read(paths.node(&node_id)).unwrap();
    let events_before = fs::read(paths.events()).ok();
    let manifest_before = fs::read(paths.manifest()).ok();

    for (offset, state) in [
        (0, TelemetryState::AgentActive),
        (1, TelemetryState::ToolRunning),
        (2, TelemetryState::Settled),
        (3, TelemetryState::Shutdown),
    ] {
        update_telemetry_with_clock(&paths, &update(0, state), &FixedClock(time(offset))).unwrap();
    }
    let node_after = read_node(&paths, &node_id).unwrap();

    // Every canonical outcome/settlement input is byte-for-byte untouched.
    assert_eq!(fs::read(paths.node(&node_id)).unwrap(), node_before);
    assert_eq!(fs::read(paths.events()).ok(), events_before);
    assert_eq!(fs::read(paths.manifest()).ok(), manifest_before);
    assert_eq!(node_after.status, octl_core::Status::Running);
    assert_eq!(node_after.last_report, None);
    assert_eq!(node_after.retry_attempts, 0);
    assert_eq!(node_after.pending_merge, None);
    assert_eq!(node_after.worker_exit, None);
    assert_eq!(node_after.first_death_at, None);

    // The stored DTO cannot smuggle any outcome/control spelling.
    let object = stored(&paths).as_object().unwrap().clone();
    let keys: std::collections::BTreeSet<_> = object.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        std::collections::BTreeSet::from([
            "attempt",
            "expires_at",
            "node_id",
            "protocol_version",
            "received_at",
            "run_id",
            "schema_version",
            "state",
            "state_since",
        ])
    );
}

#[test]
fn unapplied_event_tail_fails_closed_without_advancing_watermark() {
    let (_temp, paths) = setup("running", 0);
    let manifest = manifest_value(0);
    fs::write(paths.manifest(), serde_json::to_vec(&manifest).unwrap()).unwrap();
    let tail = json!({
        "ts":"2026-08-24T12:00:00Z","seq":1,"kind":"audit.fixture",
        "run_id":RUN,"node_id":NODE,"data":{}
    });
    let mut line = serde_json::to_vec(&tail).unwrap();
    line.push(b'\n');
    fs::write(paths.events(), line).unwrap();
    let manifest_before = fs::read(paths.manifest()).unwrap();
    let node_before = fs::read(paths.node(&NODE.parse().unwrap())).unwrap();

    assert!(matches!(
        update_telemetry_with_clock(
            &paths,
            &update(0, TelemetryState::AgentActive),
            &FixedClock(time(0))
        ),
        Err(TelemetryError::RunStateNotCurrent)
    ));
    assert_eq!(fs::read(paths.manifest()).unwrap(), manifest_before);
    assert_eq!(
        fs::read(paths.node(&NODE.parse().unwrap())).unwrap(),
        node_before
    );
    assert!(!sample_path(&paths).exists());
    assert!(matches!(
        read_telemetry_with_clock(&paths, &NODE.parse().unwrap(), &FixedClock(time(0))),
        Err(TelemetryError::RunStateNotCurrent)
    ));
}

#[test]
fn direct_updates_revalidate_shape_identity_and_metadata_without_mutation() {
    let (_temp, paths) = setup("running", 0);
    let mut bad = update(0, TelemetryState::Settled);
    bad.active_tool_count = Some(1);
    assert!(matches!(
        update_telemetry_with_clock(&paths, &bad, &FixedClock(time(0))),
        Err(TelemetryError::InvalidMetadata(_))
    ));
    bad = update(0, TelemetryState::Settled);
    bad.run_id = "02jxsnap000000000000000000".parse().unwrap();
    assert!(matches!(
        update_telemetry_with_clock(&paths, &bad, &FixedClock(time(0))),
        Err(TelemetryError::RunMismatch { .. })
    ));
    assert!(!sample_path(&paths).exists());
}

#[test]
fn metadata_and_strict_json_boundaries_are_pinned() {
    for (count, accepted) in [(1, true), (32, true), (33, false)] {
        let mut candidate = update(0, TelemetryState::ToolRunning);
        candidate.active_tool_count = Some(count);
        assert_eq!(
            update_telemetry_with_clock(&setup("running", 0).1, &candidate, &FixedClock(time(0)))
                .is_ok(),
            accepted
        );
    }
    for (length, accepted) in [(64, true), (65, false)] {
        let (_temp, paths) = setup("running", 0);
        let mut candidate = update(0, TelemetryState::ToolRunning);
        candidate.active_tool_count = Some(1);
        candidate.tool_name = Some("a".repeat(length));
        assert_eq!(
            update_telemetry_with_clock(&paths, &candidate, &FixedClock(time(0))).is_ok(),
            accepted
        );
    }
    let duplicate = format!(
        "{{\"schema_version\":1,\"schema_version\":1,\"protocol_version\":1,\"run_id\":\"{RUN}\",\"node_id\":\"{NODE}\",\"attempt\":0,\"state\":\"settled\"}}"
    );
    assert!(matches!(
        parse_telemetry_update(duplicate.as_bytes()),
        Err(TelemetryError::InvalidRequest(_))
    ));
}

#[test]
fn missing_or_ahead_canonical_state_fails_closed() {
    let (_temp, paths) = setup("running", 0);
    fs::remove_file(paths.manifest()).unwrap();
    assert!(matches!(
        update_telemetry_with_clock(
            &paths,
            &update(0, TelemetryState::Settled),
            &FixedClock(time(0))
        ),
        Err(TelemetryError::RunStateNotCurrent)
    ));

    fs::write(
        paths.manifest(),
        serde_json::to_vec(&manifest_value(5)).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        read_telemetry_with_clock(&paths, &NODE.parse().unwrap(), &FixedClock(time(0))),
        Err(TelemetryError::RunStateNotCurrent)
    ));
    assert!(!sample_path(&paths).exists());
}

#[test]
fn update_of_nonexistent_run_does_not_create_run_or_lock() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("missing-run");
    let paths = RunPaths::new(&root, RUN).unwrap();
    assert!(update_telemetry_with_clock(
        &paths,
        &update(0, TelemetryState::Settled),
        &FixedClock(time(0))
    )
    .is_err());
    assert!(!root.exists());
}

#[test]
fn same_state_backward_clock_preserves_anchor_and_reports_unreliable_until_catchup() {
    let (_temp, paths) = setup("running", 0);
    let request = update(0, TelemetryState::ToolRunning);
    update_telemetry_with_clock(&paths, &request, &FixedClock(time(100))).unwrap();
    update_telemetry_with_clock(&paths, &request, &FixedClock(time(90))).unwrap();
    let value = stored(&paths);
    assert_eq!(value["state_since"], "2026-08-24T12:01:40Z");
    assert_eq!(value["received_at"], "2026-08-24T12:01:30Z");
    let node_id = NODE.parse().unwrap();
    let unreliable = read_telemetry_with_clock(&paths, &node_id, &FixedClock(time(99))).unwrap();
    assert_eq!(unreliable.sample, TelemetrySampleStatus::ClockUnreliable);
    assert_eq!(unreliable.age_ms, None);
    let caught_up = read_telemetry_with_clock(&paths, &node_id, &FixedClock(time(100))).unwrap();
    assert_eq!(caught_up.sample, TelemetrySampleStatus::Current);
    assert_eq!(caught_up.age_ms, Some(10_000));
    assert_eq!(caught_up.state_elapsed_ms, Some(0));
}

#[test]
fn clock_overflow_rejects_without_replacing_prior_sample() {
    let (_temp, paths) = setup("running", 0);
    update_telemetry_with_clock(
        &paths,
        &update(0, TelemetryState::AgentActive),
        &FixedClock(time(0)),
    )
    .unwrap();
    let before = fs::read(sample_path(&paths)).unwrap();
    assert!(matches!(
        update_telemetry_with_clock(
            &paths,
            &update(0, TelemetryState::Settled),
            &FixedClock(DateTime::<Utc>::MAX_UTC)
        ),
        Err(TelemetryError::ClockOverflow)
    ));
    assert_eq!(fs::read(sample_path(&paths)).unwrap(), before);
}

#[test]
fn semantic_corruption_is_invalid_and_operational_io_errors_propagate() {
    let (_temp, paths) = setup("running", 0);
    let node_id = NODE.parse().unwrap();
    update_telemetry_with_clock(
        &paths,
        &update(0, TelemetryState::Settled),
        &FixedClock(time(0)),
    )
    .unwrap();
    for (field, value) in [
        ("run_id", json!("02jxsnap000000000000000000")),
        ("node_id", json!("n-9999")),
        ("protocol_version", json!(2)),
        ("expires_at", json!("2026-08-24T12:01:29Z")),
    ] {
        let mut value_sample = stored(&paths);
        value_sample[field] = value;
        fs::write(
            sample_path(&paths),
            serde_json::to_vec(&value_sample).unwrap(),
        )
        .unwrap();
        assert_eq!(
            read_telemetry_with_clock(&paths, &node_id, &FixedClock(time(1)))
                .unwrap()
                .sample,
            TelemetrySampleStatus::Invalid,
            "field {field}"
        );
        update_telemetry_with_clock(
            &paths,
            &update(0, TelemetryState::Settled),
            &FixedClock(time(0)),
        )
        .unwrap();
    }

    fs::remove_file(sample_path(&paths)).unwrap();
    fs::create_dir(sample_path(&paths)).unwrap();
    assert!(matches!(
        read_telemetry_with_clock(&paths, &node_id, &FixedClock(time(0))),
        Err(TelemetryError::Core(_))
    ));
    assert!(matches!(
        update_telemetry_with_clock(
            &paths,
            &update(0, TelemetryState::Settled),
            &FixedClock(time(0))
        ),
        Err(TelemetryError::Core(_))
    ));
}

#[cfg(unix)]
#[test]
fn telemetry_symlinks_are_rejected_without_touching_targets() {
    use std::os::unix::fs::symlink;
    let (temp, paths) = setup("running", 0);
    let outside = temp.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    symlink(&outside, paths.root.join("telemetry")).unwrap();
    assert!(matches!(
        update_telemetry_with_clock(
            &paths,
            &update(0, TelemetryState::Settled),
            &FixedClock(time(0))
        ),
        Err(TelemetryError::Core(octl_core::Error::SymlinkSubdir {
            name: "telemetry",
            ..
        }))
    ));
    assert!(fs::read_dir(&outside).unwrap().next().is_none());

    fs::remove_file(paths.root.join("telemetry")).unwrap();
    fs::create_dir(paths.root.join("telemetry")).unwrap();
    let target = temp.path().join("target.json");
    fs::write(&target, b"unchanged").unwrap();
    symlink(&target, sample_path(&paths)).unwrap();
    assert!(update_telemetry_with_clock(
        &paths,
        &update(0, TelemetryState::Settled),
        &FixedClock(time(0))
    )
    .is_err());
    assert_eq!(fs::read(target).unwrap(), b"unchanged");
}

#[test]
fn concurrent_replacement_never_exposes_partial_or_mixed_samples() {
    use std::sync::{Arc, Barrier};
    use std::thread;

    let (_temp, paths) = setup("running", 0);
    update_telemetry_with_clock(
        &paths,
        &update(0, TelemetryState::AgentActive),
        &FixedClock(time(0)),
    )
    .unwrap();
    let root = paths.root.clone();
    let barrier = Arc::new(Barrier::new(3));
    let writer_barrier = Arc::clone(&barrier);
    let writer_root = root.clone();
    let writer = thread::spawn(move || {
        let writer_paths = RunPaths::new(writer_root, RUN).unwrap();
        writer_barrier.wait();
        for index in 1..=100 {
            let state = if index % 2 == 0 {
                TelemetryState::AgentActive
            } else {
                TelemetryState::Settled
            };
            update_telemetry_with_clock(&writer_paths, &update(0, state), &FixedClock(time(index)))
                .unwrap();
        }
    });
    let mut readers = Vec::new();
    for _ in 0..2 {
        let reader_barrier = Arc::clone(&barrier);
        let reader_root = root.clone();
        readers.push(thread::spawn(move || {
            let reader_paths = RunPaths::new(reader_root, RUN).unwrap();
            let node_id = NODE.parse().unwrap();
            reader_barrier.wait();
            for _ in 0..200 {
                let view =
                    read_telemetry_with_clock(&reader_paths, &node_id, &FixedClock(time(100)))
                        .unwrap();
                assert_ne!(view.sample, TelemetrySampleStatus::Invalid);
                assert!(matches!(
                    view.state,
                    Some(TelemetryState::AgentActive | TelemetryState::Settled)
                ));
            }
        }));
    }
    writer.join().unwrap();
    for reader in readers {
        reader.join().unwrap();
    }
}

#[test]
fn terminal_and_retry_races_serialize_with_telemetry_validation() {
    use std::sync::{Arc, Barrier};
    use std::thread;

    // Terminal race.
    let (_temp, paths) = setup("running", 0);
    let root = paths.root.clone();
    let barrier = Arc::new(Barrier::new(2));
    let event_barrier = Arc::clone(&barrier);
    let event_root = root.clone();
    let terminal = thread::spawn(move || {
        let event_paths = RunPaths::new(event_root, RUN).unwrap();
        event_barrier.wait();
        octl_core::append_and_apply_event(
            &event_paths,
            "node.status",
            Some(&NODE.parse().unwrap()),
            None,
            json!({"status":"done"}),
        )
        .unwrap();
    });
    barrier.wait();
    let result = update_telemetry_with_clock(
        &paths,
        &update(0, TelemetryState::Shutdown),
        &FixedClock(time(0)),
    );
    terminal.join().unwrap();
    assert!(result.is_ok() || matches!(result, Err(TelemetryError::TerminalNode { .. })));
    assert!(matches!(
        update_telemetry_with_clock(
            &paths,
            &update(0, TelemetryState::Shutdown),
            &FixedClock(time(1))
        ),
        Err(TelemetryError::TerminalNode { .. })
    ));

    // Retry race.
    let (_temp, paths) = setup("running", 0);
    let root = paths.root.clone();
    let barrier = Arc::new(Barrier::new(2));
    let event_barrier = Arc::clone(&barrier);
    let event_root = root.clone();
    let retry = thread::spawn(move || {
        let event_paths = RunPaths::new(event_root, RUN).unwrap();
        event_barrier.wait();
        octl_core::append_and_apply_event(
            &event_paths,
            "node.retry",
            Some(&NODE.parse().unwrap()),
            None,
            json!({"attempt":1,"branch":"wt/retry","worktree_path":"/tmp/retry"}),
        )
        .unwrap();
    });
    barrier.wait();
    let result = update_telemetry_with_clock(
        &paths,
        &update(0, TelemetryState::ToolRunning),
        &FixedClock(time(0)),
    );
    retry.join().unwrap();
    assert!(result.is_ok() || matches!(result, Err(TelemetryError::AttemptMismatch { .. })));
    assert!(matches!(
        update_telemetry_with_clock(
            &paths,
            &update(0, TelemetryState::ToolRunning),
            &FixedClock(time(1))
        ),
        Err(TelemetryError::AttemptMismatch {
            expected: 1,
            found: 0
        })
    ));
    if sample_path(&paths).exists() {
        assert_eq!(
            read_telemetry_with_clock(&paths, &NODE.parse().unwrap(), &FixedClock(time(1)))
                .unwrap()
                .sample,
            TelemetrySampleStatus::Absent
        );
    }
}

#[test]
fn malformed_stored_unknown_fields_and_oversize_are_invalid() {
    let (_temp, paths) = setup("running", 0);
    let node_id = NODE.parse().unwrap();
    fs::create_dir_all(paths.root.join("telemetry")).unwrap();
    let malformed = json!({
        "schema_version":1,"protocol_version":1,"run_id":RUN,"node_id":NODE,
        "attempt":0,"state":"settled","state_since":"2026-08-24T12:00:00Z",
        "received_at":"2026-08-24T12:00:00Z","expires_at":"2026-08-24T12:01:30Z",
        "status":"done"
    });
    fs::write(sample_path(&paths), serde_json::to_vec(&malformed).unwrap()).unwrap();
    assert_eq!(
        read_telemetry_with_clock(&paths, &node_id, &FixedClock(time(0)))
            .unwrap()
            .sample,
        TelemetrySampleStatus::Invalid
    );
    fs::write(sample_path(&paths), vec![b'x'; TELEMETRY_MAX_BYTES + 1]).unwrap();
    assert_eq!(
        read_telemetry_with_clock(&paths, &node_id, &FixedClock(time(0)))
            .unwrap()
            .sample,
        TelemetrySampleStatus::Invalid
    );
}
