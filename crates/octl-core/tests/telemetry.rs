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

fn setup(status: &str, attempt: u32) -> (TempDir, RunPaths) {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("run");
    fs::create_dir_all(root.join("nodes")).unwrap();
    fs::write(
        root.join("nodes/n-0001.json"),
        serde_json::to_vec(&node_value(status, attempt)).unwrap(),
    )
    .unwrap();
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
    let manifest = json!({
        "schema_version":1,"applied_seq":0,"run_id":RUN,"kind":"spinoff",
        "lifecycle":"autonomous","title":"fixture","status":"running",
        "created_at":"2026-08-24T11:00:00Z","updated_at":"2026-08-24T11:00:00Z",
        "source_repo":null,"source_branch":null,"worktree_root":null,
        "managed_tmux_session":null,"notify_cmd":null,"harness":"pi","node_count":1,
        "parent_run_id":null,"parent_node_id":null
    });
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
        Err(TelemetryError::RunStateBehind)
    ));
    assert_eq!(fs::read(paths.manifest()).unwrap(), manifest_before);
    assert_eq!(
        fs::read(paths.node(&NODE.parse().unwrap())).unwrap(),
        node_before
    );
    assert!(!sample_path(&paths).exists());
    assert_eq!(
        read_telemetry_with_clock(&paths, &NODE.parse().unwrap(), &FixedClock(time(0)))
            .unwrap()
            .sample,
        TelemetrySampleStatus::Invalid
    );
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
