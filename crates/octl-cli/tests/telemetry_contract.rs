//! Public worker-telemetry adapter contract and fixture conformance.
//!
//! Endpoint cases execute against the real public command. The trace oracle is
//! test-only and harness-neutral: it checks that published reference traces are
//! internally consistent, but it does not execute pi hooks or claim conformance
//! of the separately owned adapter runtime.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use chrono::{DateTime, Duration, Utc};
use serde_json::{json, Map, Value};
use tempfile::TempDir;

const CONTRACT: &str = include_str!("../../../contracts/worker-telemetry-v1/contract.json");
const FIXTURES: &str = include_str!("../../../contracts/worker-telemetry-v1/conformance.json");

fn contract() -> Value {
    serde_json::from_str(CONTRACT).expect("valid contract JSON")
}

fn fixtures() -> Value {
    serde_json::from_str(FIXTURES).expect("valid conformance JSON")
}

fn bin(home: &TempDir) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_orchestratectl"));
    command.env("ORCHESTRATECTL_HOME", home.path());
    // Run creation needs no materialized worktree for an endpoint-only test.
    command.env("OCTL_TEST_SKIP_MATERIALIZE", "1");
    command
}

fn output(command: &mut Command) -> Output {
    command.output().expect("spawn orchestratectl")
}

fn success_json(command: &mut Command) -> Value {
    let result = output(command);
    assert!(
        result.status.success(),
        "exit={:?} stderr={}",
        result.status,
        String::from_utf8_lossy(&result.stderr)
    );
    serde_json::from_slice(&result.stdout).expect("success JSON")
}

fn write_json(home: &TempDir, name: &str, value: &Value) -> PathBuf {
    let path = home.path().join(name);
    std::fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
    path
}

fn create_run_and_node(home: &TempDir) -> String {
    let created = success_json(bin(home).args([
        "--output",
        "json",
        "run",
        "create",
        "--kind",
        "spinoff",
        "--title",
        "telemetry-contract-fixture",
    ]));
    let run_id = created["data"]["run_id"].as_str().unwrap().to_string();
    let node = write_json(home, "node.json", &json!({"kind": "spinoff"}));
    success_json(bin(home).args([
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
        node.to_str().unwrap(),
    ]));
    run_id
}

fn contract_endpoint_args() -> Vec<String> {
    let contract = contract();
    let argv = contract["endpoint"]["argv"].as_array().unwrap();
    assert_eq!(argv[0], "orchestratectl");
    argv[1..]
        .iter()
        .map(|value| value.as_str().unwrap().to_string())
        .collect()
}

fn update_from_bytes(home: &TempDir, bytes: &[u8]) -> Output {
    let mut child = bin(home)
        .args(contract_endpoint_args())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn telemetry endpoint");
    use std::io::Write as _;
    // An oversized request may be rejected before the pipe is fully drained.
    let _ = child.stdin.take().unwrap().write_all(bytes);
    child.wait_with_output().unwrap()
}

fn error_envelope(result: &Output) -> Value {
    assert!(!result.status.success(), "expected endpoint failure");
    let stderr = String::from_utf8_lossy(&result.stderr);
    serde_json::from_str(stderr.lines().last().expect("error envelope"))
        .expect("versioned error JSON")
}

fn request_for_context(mut request: Value, run_id: &str) -> Value {
    if request.get("run_id") == Some(&Value::String("$RUN_ID".to_string())) {
        request["run_id"] = Value::String(run_id.to_string());
    }
    if request.get("node_id") == Some(&Value::String("$NODE_ID".to_string())) {
        request["node_id"] = Value::String("n-0001".to_string());
    }
    request
}

fn telemetry_row<'a>(shown: &'a Value, node_id: &str) -> &'a Value {
    shown["data"]["telemetry"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["node_id"] == node_id)
        .expect("telemetry row")
}

fn prepare_attempt(home: &TempDir, run_id: &str, attempt: u64) {
    if attempt == 0 {
        return;
    }
    // This establishes an endpoint precondition; it does not test the
    // supervisor-owned retry transition or expose projection setup publicly.
    let node_path = home
        .path()
        .join("runs")
        .join(run_id)
        .join("nodes/n-0001.json");
    let mut node: Value = serde_json::from_slice(&std::fs::read(&node_path).unwrap()).unwrap();
    assert!(
        node["retry_attempts"].is_u64(),
        "retry projection shape changed"
    );
    node["retry_attempts"] = Value::from(attempt);
    std::fs::write(&node_path, serde_json::to_vec(&node).unwrap()).unwrap();
}

fn generated_request(generator: &Value, run_id: &str) -> Vec<u8> {
    match generator["kind"].as_str().unwrap() {
        "repeated_byte" => vec![
            generator["byte_value"].as_u64().unwrap() as u8;
            generator["count"].as_u64().unwrap() as usize
        ],
        "tool_name_length" => serde_json::to_vec(&json!({
            "schema_version": 1,
            "protocol_version": 1,
            "run_id": run_id,
            "node_id": "n-0001",
            "attempt": 0,
            "state": "tool_running",
            "active_tool_count": 1,
            "tool_name": "a".repeat(generator["length"].as_u64().unwrap() as usize)
        }))
        .unwrap(),
        other => panic!("unknown request generator {other}"),
    }
}

#[test]
fn published_contract_pins_the_public_boundary() {
    let contract = contract();
    assert_eq!(contract["schema_version"], 1);
    assert_eq!(contract["protocol_version"], 1);
    assert_eq!(contract["endpoint"]["request_max_bytes"], 4096);
    assert_eq!(contract["endpoint"]["request_max_bytes_inclusive"], true);
    assert_eq!(contract["timing"]["refresh_interval_ms"], 30_000);
    assert_eq!(contract["timing"]["freshness_ms"], 90_000);
    assert_eq!(contract["timing"]["send_floor_ms"], 2_000);
    assert_eq!(contract["timing"]["shutdown_flush_timeout_ms"], 2_000);
    assert_eq!(contract["timing"]["maximum_in_flight"], 1);
    assert_eq!(contract["timing"]["adapter_clock"], "monotonic");
    assert_eq!(
        contract["state_precedence"],
        json!(["shutdown", "tool_running", "agent_active", "settled"])
    );
    assert_eq!(
        contract["environment"]["export_condition"],
        "selected candidate has harness=pi and telemetry=worker-v1"
    );
    assert_eq!(
        contract["environment"]["attempt_source"],
        "absolute node attempt: 0 at initial creation, current retry attempt thereafter"
    );
    assert_eq!(contract["environment"]["run_id"]["name"], "OCTL_RUN_ID");
    assert_eq!(contract["environment"]["node_id"]["name"], "OCTL_NODE_ID");
    assert_eq!(contract["environment"]["attempt"]["name"], "OCTL_ATTEMPT");
    assert_eq!(
        contract["request"]["fields"]["tool_name"]["pattern"],
        "^[A-Za-z0-9_.:-]{1,64}$"
    );
    assert_eq!(
        contract["request"]["additional_properties"],
        "reject with invalid_telemetry_request"
    );
}

#[test]
fn endpoint_fixtures_execute_against_the_exact_public_command() {
    let contract = contract();
    let freshness = Duration::milliseconds(contract["timing"]["freshness_ms"].as_i64().unwrap());
    for case in fixtures()["endpoint_cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let home = TempDir::new().unwrap();
        let run_id = create_run_and_node(&home);
        let setup = case.get("setup").cloned().unwrap_or_else(|| json!({}));
        prepare_attempt(
            &home,
            &run_id,
            setup
                .get("current_attempt")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        );
        if let Some(prior) = setup.get("prior_sample") {
            let request = request_for_context(prior.clone(), &run_id);
            let seeded = update_from_bytes(&home, &serde_json::to_vec(&request).unwrap());
            assert!(seeded.status.success(), "{id}: seed prior sample");
        }

        let bytes = if let Some(generator) = case.get("request_generator") {
            generated_request(generator, &run_id)
        } else {
            serde_json::to_vec(&request_for_context(case["request"].clone(), &run_id)).unwrap()
        };
        let result = update_from_bytes(&home, &bytes);
        let expected = &case["expect"];
        if expected["accepted"] == true {
            assert!(
                result.status.success(),
                "{id}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            let envelope: Value = serde_json::from_slice(&result.stdout).unwrap();
            let request = request_for_context(case["request"].clone(), &run_id);
            assert_eq!(envelope["schema_version"], 1, "{id}");
            assert_eq!(envelope["data"]["accepted"], true, "{id}");
            assert_eq!(envelope["data"]["run_id"], run_id, "{id}");
            assert_eq!(envelope["data"]["node_id"], "n-0001", "{id}");
            assert_eq!(envelope["data"]["attempt"], request["attempt"], "{id}");
            assert_eq!(envelope["warnings"], json!([]), "{id}");
            let received: DateTime<Utc> = envelope["data"]["received_at"]
                .as_str()
                .unwrap()
                .parse()
                .unwrap();
            let expires: DateTime<Utc> = envelope["data"]["expires_at"]
                .as_str()
                .unwrap()
                .parse()
                .unwrap();
            assert_eq!(expires - received, freshness, "{id}");

            let shown = success_json(bin(&home).args(["--output", "json", "run", "show", &run_id]));
            let row = telemetry_row(&shown, "n-0001");
            assert_eq!(row["state"], expected["stored_state"], "{id}");
            assert_eq!(
                &row["attempt"],
                expected
                    .get("stored_attempt")
                    .unwrap_or(&request["attempt"]),
                "{id}"
            );
            assert_optional(
                row,
                expected,
                "active_tool_count",
                "stored_active_tool_count",
                id,
            );
            assert_optional(row, expected, "tool_name", "stored_tool_name", id);
        } else {
            let envelope = error_envelope(&result);
            assert_eq!(envelope["schema_version"], 1, "{id}");
            assert_eq!(envelope["error"]["code"], expected["error_code"], "{id}");
            if let Some(attempt) = expected.get("expected_attempt") {
                assert_eq!(&envelope["error"]["expected"], attempt, "{id}");
            }
            if let Some(maximum) = expected.get("maximum_bytes") {
                assert_eq!(
                    &envelope["error"]["expected"]["maximum_bytes"], maximum,
                    "{id}"
                );
            }
        }

        if let Some(prior_state) = expected.get("prior_state") {
            let shown = success_json(bin(&home).args(["--output", "json", "run", "show", &run_id]));
            let row = telemetry_row(&shown, "n-0001");
            assert_eq!(shown["data"]["status"], expected["run_status"], "{id}");
            assert_eq!(&row["state"], prior_state, "{id}");
            assert_eq!(&row["attempt"], &expected["prior_attempt"], "{id}");
            assert_eq!(row["sample"], "current", "{id}");
        }
    }
}

fn assert_optional(row: &Value, expected: &Value, row_key: &str, expected_key: &str, id: &str) {
    if let Some(value) = expected.get(expected_key) {
        assert_eq!(&row[row_key], value, "{id}: {row_key}");
    } else {
        assert!(row.get(row_key).is_none(), "{id}: unexpected {row_key}");
    }
}

#[derive(Default)]
struct ReferenceTrace {
    started: bool,
    shutdown: bool,
    agent_active: bool,
    tools: BTreeMap<String, String>,
}

impl ReferenceTrace {
    fn apply(&mut self, step: &Value) {
        if self.shutdown {
            return;
        }
        match step["event"].as_str().unwrap() {
            "session_open" => self.started = true,
            "turn_start" => {
                self.started = true;
                self.agent_active = true;
            }
            "turn_settled" => self.agent_active = false,
            "tool_open" => {
                self.started = true;
                self.tools
                    .entry(step["tool_ref"].as_str().unwrap().to_string())
                    .or_insert_with(|| step["tool_name"].as_str().unwrap().to_string());
            }
            "tool_close" => {
                self.tools.remove(step["tool_ref"].as_str().unwrap());
            }
            "session_close" => self.shutdown = true,
            "refresh_due" => {}
            other => panic!("unknown harness-neutral fixture event {other}"),
        }
    }

    fn state(&self) -> &'static str {
        if self.shutdown {
            "shutdown"
        } else if !self.tools.is_empty() {
            "tool_running"
        } else if self.agent_active {
            "agent_active"
        } else {
            assert!(
                self.started,
                "fixture asks for state before session activity"
            );
            "settled"
        }
    }
}

#[test]
fn adapter_reference_traces_are_internally_consistent() {
    let contract = contract();
    let fixtures = fixtures();
    let floor = contract["timing"]["send_floor_ms"].as_u64().unwrap();
    let refresh = contract["timing"]["refresh_interval_ms"].as_u64().unwrap();
    let shutdown_budget = contract["timing"]["shutdown_flush_timeout_ms"]
        .as_u64()
        .unwrap();
    let allowed_payload: BTreeSet<&str> = contract["desired_snapshot"]["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|field| field.as_str().unwrap())
        .collect();
    let mut ids = BTreeSet::new();

    for sequence in fixtures["adapter_sequences"].as_array().unwrap() {
        let id = sequence["id"].as_str().unwrap();
        assert!(ids.insert(id), "duplicate fixture id {id}");
        let window = sequence["observation_window_ms"].as_u64().unwrap();
        let mut trace = ReferenceTrace::default();
        let mut prior_step = 0;
        for step in sequence["steps"].as_array().unwrap() {
            let at = step["at_ms"].as_u64().unwrap();
            assert!(
                at >= prior_step && at <= window,
                "{id}: step outside ordered window"
            );
            prior_step = at;
            trace.apply(step);
            assert_eq!(trace.state(), step["expect_state"], "{id}: {step}");
            if let Some(expected) = step.get("expect_active_tool_count") {
                assert_eq!(trace.tools.len() as u64, expected.as_u64().unwrap(), "{id}");
            }
            if let Some(expected) = step.get("expect_tool_name") {
                assert_eq!(
                    trace.tools.values().next().unwrap(),
                    expected.as_str().unwrap(),
                    "{id}"
                );
            }
        }

        let sends = sequence
            .get("expected_sends")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice);
        let results = sequence
            .get("sender_results")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice);
        assert_eq!(
            sends.len(),
            results.len(),
            "{id}: every send needs a scripted result"
        );
        let mut previous_start = None;
        let mut previous_end = None;
        for (index, send) in sends.iter().enumerate() {
            let start = send["start_ms"].as_u64().unwrap();
            assert!(start <= window, "{id}: send outside observation window");
            let result = &results[index];
            assert_eq!(result["send_index"], index as u64, "{id}");
            let end = result["complete_at_ms"].as_u64().unwrap();
            assert!(end >= start, "{id}: completion before start");
            if let Some(previous_end) = previous_end {
                assert!(start >= previous_end, "{id}: more than one send in flight");
            }
            if let Some(previous_start) = previous_start {
                if send["reason"] != "shutdown_flush" {
                    assert!(start >= previous_start + floor, "{id}: send floor violated");
                }
                if send["reason"] == "refresh" {
                    assert_eq!(
                        start,
                        previous_start + refresh,
                        "{id}: refresh anchor drift"
                    );
                }
            }
            let payload = send["payload"].as_object().unwrap();
            assert_payload(payload, &allowed_payload, id);
            previous_start = Some(start);
            previous_end = Some(end);
        }

        if let Some(deadline) = sequence.get("shutdown_return_by_ms") {
            let shutdown_at = sequence["steps"]
                .as_array()
                .unwrap()
                .iter()
                .find(|step| step["event"] == "session_close")
                .unwrap()["at_ms"]
                .as_u64()
                .unwrap();
            assert!(
                deadline.as_u64().unwrap() <= shutdown_at + shutdown_budget,
                "{id}"
            );
        }
    }
}

fn assert_payload(payload: &Map<String, Value>, allowed: &BTreeSet<&str>, id: &str) {
    assert!(
        payload.keys().all(|key| allowed.contains(key.as_str())),
        "{id}: private payload key"
    );
    let state = payload["state"].as_str().unwrap();
    if state != "tool_running" {
        assert!(
            payload.get("active_tool_count").is_none(),
            "{id}: non-tool count"
        );
        assert!(payload.get("tool_name").is_none(), "{id}: non-tool name");
    }
    if let Some(count) = payload.get("active_tool_count").and_then(Value::as_u64) {
        assert!((1..=32).contains(&count), "{id}: count out of range");
    }
    if let Some(name) = payload.get("tool_name").and_then(Value::as_str) {
        assert_eq!(payload["active_tool_count"], 1, "{id}: named tool count");
        assert!(valid_tool_name(name), "{id}: unsanitary tool name");
    }
}

fn valid_tool_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b':' | b'-'))
}

#[test]
fn accepted_reference_trace_payloads_fit_the_real_endpoint() {
    for sequence in fixtures()["adapter_sequences"].as_array().unwrap() {
        let Some(sends) = sequence.get("expected_sends").and_then(Value::as_array) else {
            continue;
        };
        let results = sequence["sender_results"].as_array().unwrap();
        let id = sequence["id"].as_str().unwrap();
        let home = TempDir::new().unwrap();
        let run_id = create_run_and_node(&home);
        let mut last_accepted = None;
        for (index, send) in sends.iter().enumerate() {
            if results[index]["result"] != "accepted" {
                continue;
            }
            let mut request = json!({
                "schema_version": 1,
                "protocol_version": 1,
                "run_id": run_id,
                "node_id": "n-0001",
                "attempt": 0
            });
            for (key, value) in send["payload"].as_object().unwrap() {
                request[key] = value.clone();
            }
            let result = update_from_bytes(&home, &serde_json::to_vec(&request).unwrap());
            assert!(
                result.status.success(),
                "{id}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            last_accepted = Some(send["payload"].clone());
        }
        if let Some(expected) = last_accepted {
            let shown = success_json(bin(&home).args(["--output", "json", "run", "show", &run_id]));
            let row = telemetry_row(&shown, "n-0001");
            assert_eq!(
                shown["data"]["status"], "pending",
                "{id}: telemetry became run truth"
            );
            assert_eq!(row["sample"], "current", "{id}");
            assert_eq!(row["state"], expected["state"], "{id}");
            for key in ["active_tool_count", "tool_name"] {
                if let Some(value) = expected.get(key) {
                    assert_eq!(&row[key], value, "{id}: {key}");
                } else {
                    assert!(row.get(key).is_none(), "{id}: unexpected {key}");
                }
            }
        }
    }
}

#[test]
fn fixture_files_are_portable_json_and_use_only_the_public_endpoint() {
    serde_json::from_str::<Value>(CONTRACT).unwrap();
    serde_json::from_str::<Value>(FIXTURES).unwrap();
    assert!(Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../contracts/worker-telemetry-v1/README.md")
        .is_file());
    assert_eq!(
        contract()["endpoint"]["argv"],
        json!([
            "orchestratectl",
            "node",
            "telemetry",
            "update",
            "--input-file",
            "-",
            "--output",
            "json"
        ])
    );
}
