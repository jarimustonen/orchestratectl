//! Integration-level **envelope snapshot** suite.
//!
//! The per-subcommand suites (`run.rs`, `event.rs`, …) assert *semantics*
//! field-by-field. This suite locks the *shape* of the machine-readable
//! contract with `insta` snapshots, so envelope drift (a renamed field, a
//! changed nesting, a dropped `schema_version`) is caught the moment it
//! happens — regardless of which subcommand a contributor touched.
//!
//! What is locked, per `AGENTS-AI-FIRST-CLI.md`:
//!
//! - **Success envelope** (§10, stdout): `{schema_version, data, warnings?}`
//! - **Error envelope** (§10, stderr): `{schema_version, error: {code,
//!   message, invalid_value?, expected?}}`, with exit 1 (user/validation)
//!   or 2 (refused-but-actionable / system)
//! - **Dry-run / planning envelopes** (§11): the `dry_run`/`would_be`
//!   shapes each mutating verb emits
//! - **Format coverage** (§9): every noun is snapshotted in `text`, `json`
//!   (pretty single document) and `jsonl` (compact one-line) so a change
//!   to *any* renderer is visible
//!
//! ## Determinism
//!
//! Three classes of field are non-deterministic and are redacted via
//! `insta` filters (regex substitution on the rendered output) before the
//! snapshot is compared — see [`snapshot`]:
//!
//! - `run_id` (a ULID) — per-test dynamic filter, also rewrites the copy
//!   embedded in `dir` paths and error messages
//! - the temp `$ORCHESTRATECTL_HOME` path — per-test dynamic filter
//! - `commit` (git HEAD hash), timestamps (`created_at`/`ts`/… in all
//!   three serialised forms) and live PIDs — global filters
//!
//! ## Updating snapshots
//!
//! See `tests/snapshots/README.md`. In short: `cargo insta test
//! --review`, or `INSTA_UPDATE=always cargo test -p octl-cli --release
//! --test envelope_snapshots` then inspect the diff.

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::{json, Value};
use tempfile::TempDir;

// ----------------------------------------------------------------------
// Harness
// ----------------------------------------------------------------------

/// A binary handle pointed at an isolated `$ORCHESTRATECTL_HOME`. The
/// `OCTL_TEST_SKIP_MATERIALIZE` escape hatch keeps `run create` from
/// shelling out to `create.sh` / spawning a supervisor (same pattern the
/// sibling suites use), so output is deterministic skeleton-only state.
fn bin(home: &TempDir) -> Command {
    let mut c = Command::cargo_bin("orchestratectl").expect("binary builds");
    c.env("ORCHESTRATECTL_HOME", home.path());
    c.env("OCTL_TEST_SKIP_MATERIALIZE", "1");
    c
}

/// Run a command expected to **succeed** (exit 0) and return its stdout.
/// stderr is asserted empty: §2 keeps the data channel and the
/// diagnostic channel separate.
fn ok_stdout(cmd: &mut Command) -> String {
    let out = cmd
        .assert()
        .success()
        .stderr(predicate::str::is_empty())
        .get_output()
        .stdout
        .clone();
    String::from_utf8(out).expect("stdout is utf8")
}

/// Run a command expected to **fail** with `code`, returning the trimmed
/// stderr error envelope. stdout is asserted empty (§2): a failure must
/// not also dribble a partial payload onto the data channel.
fn err_stderr(cmd: &mut Command, code: i32) -> String {
    let out = cmd
        .assert()
        .failure()
        .code(code)
        .stdout(predicate::str::is_empty())
        .get_output()
        .stderr
        .clone();
    String::from_utf8(out)
        .expect("stderr is utf8")
        .trim_end()
        .to_string()
}

/// Parse a success envelope's stdout into JSON (for extracting ids the
/// snapshot can't, since they're redacted).
fn data_value(stdout: &str) -> Value {
    serde_json::from_str(stdout).expect("stdout is a JSON envelope")
}

/// Per-test dynamic redactions: the temp home path, and (optionally) the
/// run-id ULID. Both are regex-escaped — the home path contains `.` and
/// (on some platforms) other metacharacters.
fn redactions(home: &TempDir, run_id: Option<&str>) -> Vec<(String, &'static str)> {
    let mut v = vec![(
        regex::escape(home.path().to_str().expect("home path is utf8")),
        "[HOME]",
    )];
    if let Some(r) = run_id {
        v.push((regex::escape(r), "[RUN_ID]"));
    }
    v
}

/// Find the first ULID-shaped token (lowercase Crockford base32, 26
/// chars) in `rendered`. Used by the `run create` cases, where the id is
/// minted by the call itself and so can't be passed in advance.
fn find_ulid(rendered: &str) -> Option<String> {
    let re = regex::Regex::new(r"[0-9a-hjkmnp-tv-z]{26}").expect("valid ulid regex");
    re.find(rendered).map(|m| m.as_str().to_string())
}

/// Like [`redactions`] but for `run create` output: pulls the minted
/// run-id straight out of the rendered text so both the `run_id` field
/// and the copy embedded in `dir` collapse to `[RUN_ID]`.
fn minted_redactions(home: &TempDir, rendered: &str) -> Vec<(String, &'static str)> {
    let mut v = redactions(home, None);
    if let Some(u) = find_ulid(rendered) {
        v.push((regex::escape(&u), "[RUN_ID]"));
    }
    v
}

/// Bind the global + per-test filters and assert the snapshot. `value` is
/// the raw rendered output (stdout for success, stderr for errors); the
/// filters rewrite every non-deterministic token before comparison.
fn snapshot(name: &str, value: &str, dynamic: &[(String, &'static str)]) {
    let mut settings = insta::Settings::clone_current();
    // git HEAD hash (`version` payload).
    settings.add_filter(r"[0-9a-f]{40}", "[COMMIT]");
    // Timestamps in every serialised form we emit:
    //   RFC3339 `…Z`              (manifest/json)
    //   RFC3339 `…+00:00`         (event tail jsonl/text)
    //   `YYYY-MM-DD HH:MM:SS… UTC` (chrono Display in text mode)
    settings.add_filter(
        r"\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2}| UTC)",
        "[TS]",
    );
    // Live PID inside the `supervisor_already_running` refusal message.
    settings.add_filter(r"pid \d+", "pid [PID]");
    for (pattern, replacement) in dynamic {
        settings.add_filter(pattern.as_str(), *replacement);
    }
    settings.bind(|| insta::assert_snapshot!(name, value));
}

// ----------------------------------------------------------------------
// Seed helpers — build deterministic on-disk state via the sanctioned
// `run create` + `event create` write paths (no direct file pokes).
// ----------------------------------------------------------------------

fn create_run(home: &TempDir) -> String {
    let stdout = ok_stdout(bin(home).args([
        "--output", "json", "run", "create", "--kind", "spinoff", "--title", "seed-run",
    ]));
    data_value(&stdout)["data"]["run_id"]
        .as_str()
        .expect("run_id present")
        .to_string()
}

fn write_json(home: &TempDir, name: &str, v: Value) -> String {
    let p = home.path().join(name);
    std::fs::write(&p, serde_json::to_vec(&v).unwrap()).unwrap();
    p.to_str().unwrap().to_string()
}

fn event_create(home: &TempDir, run_id: &str, kind: &str, node_id: Option<&str>, data: Value) {
    let file = write_json(home, &format!("ev-{kind}.json"), data);
    let mut args = vec![
        "--output".into(),
        "json".into(),
        "event".into(),
        "create".into(),
        run_id.into(),
        "--kind".into(),
        kind.into(),
        "--from-file".into(),
        file,
    ];
    if let Some(n) = node_id {
        args.push("--node-id".into());
        args.push(n.into());
    }
    ok_stdout(bin(home).args(&args));
}

/// Seed a single node (`n-0001`) with fixed, snapshot-stable fields.
fn seed_node(home: &TempDir, run_id: &str) {
    event_create(
        home,
        run_id,
        "node.created",
        Some("n-0001"),
        json!({
            "kind": "spinoff",
            "branch": "wt/seed",
            "worktree_path": "/tmp/seed-wt",
            "tmux_window": "seed-win",
            "agent_pid": 4242
        }),
    );
}

fn seed_discussion(home: &TempDir, run_id: &str) {
    event_create(
        home,
        run_id,
        "discussion.opened",
        None,
        json!({
            "discussion_id": "d-0001",
            "node_id": "n-0001",
            "topic": "seed topic",
            "severity": "discuss"
        }),
    );
}

fn seed_spinoff(home: &TempDir, run_id: &str) {
    event_create(
        home,
        run_id,
        "spinoff.proposed",
        None,
        json!({
            "proposal_id": "p-0001",
            "node_id": "n-0001",
            "proposed_title": "seed proposal",
            "proposed_kind": "spinoff",
            "rationale": "follow-up"
        }),
    );
}

// ----------------------------------------------------------------------
// version
// ----------------------------------------------------------------------

#[test]
fn version_envelopes() {
    let home = TempDir::new().unwrap();
    let red = redactions(&home, None);
    for (fmt, name) in [
        ("text", "version_text"),
        ("json", "version_json"),
        ("jsonl", "version_jsonl"),
    ] {
        let out = ok_stdout(bin(&home).args(["--output", fmt, "version"]));
        snapshot(name, &out, &red);
    }
}

#[test]
fn global_arg_error_envelopes() {
    let home = TempDir::new().unwrap();
    // Unknown global flag → clap-mapped user error, exit 1.
    snapshot(
        "global_unknown_flag_error",
        &err_stderr(bin(&home).args(["--frobnicate", "version"]), 1),
        &[],
    );
    // Invalid `--output` value → invalid_arguments, exit 1.
    snapshot(
        "global_invalid_output_error",
        &err_stderr(bin(&home).args(["--output", "yaml", "version"]), 1),
        &[],
    );
}

// ----------------------------------------------------------------------
// skill
// ----------------------------------------------------------------------

#[test]
fn skill_envelopes() {
    let home = TempDir::new().unwrap();
    let red = redactions(&home, None);
    for (fmt, name) in [
        ("text", "skill_list_text"),
        ("json", "skill_list_json"),
        ("jsonl", "skill_list_jsonl"),
    ] {
        let out = ok_stdout(bin(&home).args(["--output", fmt, "skill", "list"]));
        snapshot(name, &out, &red);
    }
    // Unknown skill name → error envelope with an `expected.one_of` list.
    snapshot(
        "skill_show_unknown_error",
        &err_stderr(bin(&home).args(["skill", "show", "no-such-skill"]), 1),
        &[],
    );
}

// ----------------------------------------------------------------------
// run
// ----------------------------------------------------------------------

#[test]
fn run_create_envelopes() {
    let home = TempDir::new().unwrap();
    // Success in all three formats. Each `create` mints a fresh run-id,
    // all rewritten to [RUN_ID].
    for (fmt, name) in [
        ("text", "run_create_text"),
        ("json", "run_create_json"),
        ("jsonl", "run_create_jsonl"),
    ] {
        let out = ok_stdout(bin(&home).args([
            "--output",
            fmt,
            "run",
            "create",
            "--kind",
            "spinoff",
            "--title",
            "envelope demo",
        ]));
        let red = minted_redactions(&home, &out);
        snapshot(name, &out, &red);
    }

    // Dry-run planning envelope (§11): same payload shape, `dry_run:true`,
    // `supervisor:"not-spawned-dry-run"`.
    let out = ok_stdout(bin(&home).args([
        "--output",
        "json",
        "run",
        "create",
        "--kind",
        "code",
        "--title",
        "dry",
        "--dry-run",
    ]));
    let red = minted_redactions(&home, &out);
    snapshot("run_create_dry_run_json", &out, &red);

    // Idempotent replay (§11): a repeat `--idempotency-key` returns the
    // original run (exit 0) with `idempotent_replay:true`.
    ok_stdout(bin(&home).args([
        "--output",
        "json",
        "run",
        "create",
        "--kind",
        "spinoff",
        "--title",
        "idem",
        "--idempotency-key",
        "snap-key",
    ]));
    let out = ok_stdout(bin(&home).args([
        "--output",
        "json",
        "run",
        "create",
        "--kind",
        "spinoff",
        "--title",
        "idem",
        "--idempotency-key",
        "snap-key",
    ]));
    let red = minted_redactions(&home, &out);
    snapshot("run_create_idempotent_replay_json", &out, &red);
}

#[test]
fn run_show_envelopes() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    let red = redactions(&home, Some(&run_id));
    for (fmt, name) in [
        ("text", "run_show_text"),
        ("json", "run_show_json"),
        ("jsonl", "run_show_jsonl"),
    ] {
        let out = ok_stdout(bin(&home).args(["--output", fmt, "run", "show", &run_id]));
        snapshot(name, &out, &red);
    }
}

#[test]
fn run_error_envelopes() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);

    // Validation error (exit 1): unknown `--kind` enum value (clap).
    snapshot(
        "run_create_unknown_kind_error",
        &err_stderr(
            bin(&home).args(["run", "create", "--kind", "bogus", "--title", "x"]),
            1,
        ),
        &[],
    );

    // Not-found error (exit 1): show a non-existent run.
    snapshot(
        "run_show_not_found_error",
        &err_stderr(bin(&home).args(["run", "show", "zzzzznotarun"]), 1),
        &[],
    );

    // Refused-but-actionable (exit 2): a live supervisor PID is recorded,
    // so `run reattach` refuses rather than double-spawning. We park the
    // test's *own* PID in the pid-file — guaranteed alive for the call.
    let pid_file = home
        .path()
        .join("runs")
        .join(&run_id)
        .join("supervisor.pid");
    std::fs::write(&pid_file, std::process::id().to_string()).unwrap();
    snapshot(
        "run_reattach_supervisor_alive_error",
        &err_stderr(bin(&home).args(["run", "reattach", &run_id]), 2),
        &redactions(&home, Some(&run_id)),
    );
}

// ----------------------------------------------------------------------
// event
// ----------------------------------------------------------------------

#[test]
fn event_envelopes() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    seed_node(&home, &run_id);
    let red = redactions(&home, Some(&run_id));

    // `event create` success in json/jsonl/text (a fresh node each time
    // keeps `seq`/projections stable-ish; node ids are deterministic).
    let nc = write_json(&home, "ev-extra.json", json!({"kind": "spinoff"}));
    for (i, (fmt, name)) in [
        ("json", "event_create_json"),
        ("jsonl", "event_create_jsonl"),
        ("text", "event_create_text"),
    ]
    .into_iter()
    .enumerate()
    {
        let node = format!("n-100{i}");
        let out = ok_stdout(bin(&home).args([
            "--output",
            fmt,
            "event",
            "create",
            &run_id,
            "--kind",
            "node.created",
            "--node-id",
            &node,
            "--from-file",
            &nc,
        ]));
        snapshot(name, &out, &red);
    }

    // `event tail` is the streaming verb: jsonl + text only (pretty json
    // is rejected — see error case below). Terminates with the §12
    // `{"event":"result"}` envelope.
    for (fmt, name) in [("jsonl", "event_tail_jsonl"), ("text", "event_tail_text")] {
        let out = ok_stdout(bin(&home).args(["--output", fmt, "event", "tail", &run_id]));
        snapshot(name, &out, &red);
    }

    // Dry-run envelope for a write event.
    let st = write_json(&home, "ev-status.json", json!({"status": "running"}));
    let out = ok_stdout(bin(&home).args([
        "--output",
        "json",
        "event",
        "create",
        &run_id,
        "--kind",
        "node.status",
        "--node-id",
        "n-0001",
        "--from-file",
        &st,
        "--dry-run",
    ]));
    snapshot("event_create_dry_run_json", &out, &red);

    // Validation error (exit 1): pretty json forbidden for a stream.
    snapshot(
        "event_tail_unsupported_format_error",
        &err_stderr(
            bin(&home).args(["--output", "json", "event", "tail", &run_id]),
            1,
        ),
        &red,
    );

    // Validation error (exit 1): unknown event kind, with the closed set
    // echoed in `expected`.
    let e = write_json(&home, "ev-bad.json", json!({}));
    snapshot(
        "event_create_unknown_kind_error",
        &err_stderr(
            bin(&home).args([
                "event",
                "create",
                &run_id,
                "--kind",
                "bogus.kind",
                "--from-file",
                &e,
            ]),
            1,
        ),
        &red,
    );
}

// ----------------------------------------------------------------------
// node
// ----------------------------------------------------------------------

#[test]
fn node_envelopes() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    seed_node(&home, &run_id);
    let red = redactions(&home, Some(&run_id));

    for (fmt, name) in [
        ("text", "node_list_text"),
        ("json", "node_list_json"),
        ("jsonl", "node_list_jsonl"),
    ] {
        let out = ok_stdout(bin(&home).args(["--output", fmt, "node", "list", &run_id]));
        snapshot(name, &out, &red);
    }

    let out = ok_stdout(bin(&home).args(["--output", "json", "node", "show", &run_id, "n-0001"]));
    snapshot("node_show_json", &out, &red);
    let out = ok_stdout(bin(&home).args(["--output", "text", "node", "show", &run_id, "n-0001"]));
    snapshot("node_show_text", &out, &red);

    // Dry-run envelope for `node report`.
    let rep = write_json(
        &home,
        "report.json",
        json!({"success": true, "summary": "ok"}),
    );
    let out = ok_stdout(bin(&home).args([
        "--output",
        "json",
        "node",
        "report",
        &run_id,
        "n-0001",
        "--from-file",
        &rep,
        "--dry-run",
    ]));
    snapshot("node_report_dry_run_json", &out, &red);

    // Validation error (exit 1): unknown node id.
    snapshot(
        "node_show_not_found_error",
        &err_stderr(bin(&home).args(["node", "show", &run_id, "n-9999"]), 1),
        &red,
    );
}

// ----------------------------------------------------------------------
// discussion
// ----------------------------------------------------------------------

#[test]
fn discussion_envelopes() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    seed_node(&home, &run_id);
    seed_discussion(&home, &run_id);
    let red = redactions(&home, Some(&run_id));

    for (fmt, name) in [
        ("text", "discussion_list_text"),
        ("json", "discussion_list_json"),
        ("jsonl", "discussion_list_jsonl"),
    ] {
        let out = ok_stdout(bin(&home).args(["--output", fmt, "discussion", "list", &run_id]));
        snapshot(name, &out, &red);
    }

    let out =
        ok_stdout(bin(&home).args(["--output", "json", "discussion", "show", &run_id, "d-0001"]));
    snapshot("discussion_show_json", &out, &red);

    // Dry-run envelope for `discussion resolve`.
    let out = ok_stdout(bin(&home).args([
        "--output",
        "json",
        "discussion",
        "resolve",
        &run_id,
        "d-0001",
        "--choice",
        "keep",
        "--dry-run",
    ]));
    snapshot("discussion_resolve_dry_run_json", &out, &red);

    // Validation error (exit 1): unknown discussion id.
    snapshot(
        "discussion_show_not_found_error",
        &err_stderr(
            bin(&home).args(["discussion", "show", &run_id, "d-nope"]),
            1,
        ),
        &red,
    );
}

// ----------------------------------------------------------------------
// spinoff
// ----------------------------------------------------------------------

#[test]
fn spinoff_envelopes() {
    let home = TempDir::new().unwrap();
    let run_id = create_run(&home);
    seed_node(&home, &run_id);
    seed_spinoff(&home, &run_id);
    let red = redactions(&home, Some(&run_id));

    for (fmt, name) in [
        ("text", "spinoff_list_text"),
        ("json", "spinoff_list_json"),
        ("jsonl", "spinoff_list_jsonl"),
    ] {
        let out = ok_stdout(bin(&home).args(["--output", fmt, "spinoff", "list", &run_id]));
        snapshot(name, &out, &red);
    }

    // Validation error (exit 1): whitespace-only `--reason` rejected (§1
    // strict validation), with the offending value echoed. Run before the
    // success case so the proposal is still `pending`.
    snapshot(
        "spinoff_reject_empty_reason_error",
        &err_stderr(
            bin(&home).args(["spinoff", "reject", &run_id, "p-0001", "--reason", "   "]),
            1,
        ),
        &red,
    );

    // `spinoff reject` success (a local-only write — no issuectl needed).
    let out = ok_stdout(bin(&home).args([
        "--output", "json", "spinoff", "reject", &run_id, "p-0001", "--reason", "not now",
    ]));
    snapshot("spinoff_reject_json", &out, &red);
}
