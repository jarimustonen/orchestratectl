//! Integration tests for `orchestratectl run merge` (issue
//! `bundle-worktree-merge`). The merge backend is stubbed via `OCTL_MERGE_SH`
//! so the tests exercise orchestratectl's integration — node resolution,
//! source resolution, terminal-report submission, failure handling — without
//! a real git worktree, workmux, or tmux.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use serde_json::Value;
use tempfile::TempDir;

mod common;
use common::TestHome;

fn bin(home: &TempDir) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_orchestratectl"));
    c.env("ORCHESTRATECTL_HOME", home.path());
    c.env("OCTL_TEST_SKIP_MATERIALIZE", "1");
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

fn create_run(home: &TempDir, kind: &str, title: &str) -> String {
    run_ok(bin(home).args([
        "--output", "json", "run", "create", "--kind", kind, "--title", title,
    ]))["data"]["run_id"]
        .as_str()
        .unwrap()
        .to_string()
}

fn run_dir(home: &TempDir, run_id: &str) -> std::path::PathBuf {
    home.path().join("runs").join(run_id)
}

/// Forge a `node.created` for `n-0001` carrying a real (existing) worktree
/// path + branch so `run merge` can `cd` into it and resolve the branch.
fn forge_worker_node(home: &TempDir, run_id: &str, kind: &str, worktree: &Path, branch: &str) {
    let node = home.path().join(format!("node-{run_id}.json"));
    std::fs::write(
        &node,
        format!(
            r#"{{"kind":"{kind}","task":"x","worktree_path":"{}","branch":"{branch}","tmux_session":"octl","tmux_window_id":"@42"}}"#,
            worktree.display()
        ),
    )
    .unwrap();
    run_ok(bin(home).args([
        "--output",
        "json",
        "event",
        "create",
        run_id,
        "--kind",
        "node.created",
        "--node-id",
        "n-0001",
        "--from-file",
        node.to_str().unwrap(),
    ]));
}

/// Write an executable fake merge backend that records its argv (one line) to
/// `<dir>/merge.log` and exits `code`.
fn fake_merge_sh(dir: &Path, code: i32, stderr: &str) -> std::path::PathBuf {
    let p = dir.join("fake-merge.sh");
    let log = dir.join("merge.log");
    let body = format!(
        "#!/bin/bash\nprintf '%s ' \"$@\" >> '{}'\nprintf '\\n' >> '{}'\n{}\nexit {code}\n",
        log.display(),
        log.display(),
        if stderr.is_empty() {
            String::new()
        } else {
            format!("echo '{stderr}' >&2")
        },
    );
    std::fs::write(&p, body).unwrap();
    let mut perms = std::fs::metadata(&p).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&p, perms).unwrap();
    p
}

fn read_events(events: &Path) -> Vec<Value> {
    std::fs::read_to_string(events)
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .collect()
}

fn node_reports(events: &Path) -> Vec<Value> {
    read_events(events)
        .into_iter()
        .filter(|v| v["kind"] == "node.report")
        .collect()
}

/// A clean merge: the backend exits 0, and `run merge` appends a terminal
/// `node.report` carrying `via: "explicit-merge"`.
#[test]
fn successful_merge_submits_explicit_merge_report() {
    let home = TestHome::new();
    let scratch = TempDir::new().unwrap();
    let worktree = TempDir::new().unwrap();
    let run_id = create_run(&home, "code", "merge-ok");
    forge_worker_node(&home, &run_id, "code", worktree.path(), "wt/test-x");

    let merge_sh = fake_merge_sh(scratch.path(), 0, "");
    let v = run_ok(bin(&home).env("OCTL_MERGE_SH", &merge_sh).args([
        "--output", "json", "run", "merge", &run_id, "--source", "main",
    ]));
    assert_eq!(v["data"]["merged"], true);
    assert_eq!(v["data"]["branch"], "wt/test-x");
    assert_eq!(v["data"]["source"], "main");

    // The backend was invoked with the resolved target and branch.
    let argv = std::fs::read_to_string(scratch.path().join("merge.log")).unwrap();
    assert!(
        argv.contains("--target main") && argv.contains("wt/test-x"),
        "merge backend argv was {argv:?}"
    );

    // Exactly one terminal report, stamped with the explicit-merge marker.
    let events = run_dir(&home, &run_id).join("events.jsonl");
    let reports = node_reports(&events);
    assert_eq!(reports.len(), 1, "expected one terminal node.report");
    assert_eq!(reports[0]["data"]["success"], true);
    assert_eq!(reports[0]["data"]["via"], "explicit-merge");
}

/// `--report-file` carries a rich §7.3 payload (`discussion_items`,
/// `spinoff_proposals`) so an autonomous kind merges AND delivers its
/// structured report in one call. `run merge` stamps `via: "explicit-merge"`
/// and submits the agent's payload verbatim otherwise.
#[test]
fn report_file_payload_is_submitted_with_marker() {
    let home = TestHome::new();
    let scratch = TempDir::new().unwrap();
    let worktree = TempDir::new().unwrap();
    let run_id = create_run(&home, "research", "merge-rich");
    forge_worker_node(&home, &run_id, "research", worktree.path(), "wt/test-x");

    let report = scratch.path().join("report.json");
    std::fs::write(
        &report,
        r#"{
            "success": true,
            "summary": "research delivered",
            "discussion_items": [{"topic": "scope creep", "severity": "discuss"}],
            "spinoff_proposals": [{"proposed_title": "follow-up", "proposed_kind": "research"}],
            "wrap_up_recommendations": ["read sources/"]
        }"#,
    )
    .unwrap();

    let merge_sh = fake_merge_sh(scratch.path(), 0, "");
    run_ok(bin(&home).env("OCTL_MERGE_SH", &merge_sh).args([
        "--output",
        "json",
        "run",
        "merge",
        &run_id,
        "--report-file",
        report.to_str().unwrap(),
    ]));

    let events = run_dir(&home, &run_id).join("events.jsonl");
    let reports = node_reports(&events);
    assert_eq!(reports.len(), 1);
    let data = &reports[0]["data"];
    assert_eq!(data["via"], "explicit-merge");
    assert_eq!(data["summary"], "research delivered");
    assert_eq!(data["discussion_items"][0]["topic"], "scope creep");
    assert_eq!(data["spinoff_proposals"][0]["proposed_title"], "follow-up");
    assert_eq!(data["wrap_up_recommendations"][0], "read sources/");
}

/// A malformed `--report-file` is rejected BEFORE the merge runs — the backend
/// is never invoked and no event is appended.
#[test]
fn bad_report_file_rejected_before_merge() {
    let home = TestHome::new();
    let scratch = TempDir::new().unwrap();
    let worktree = TempDir::new().unwrap();
    let run_id = create_run(&home, "spinoff", "merge-badreport");
    forge_worker_node(&home, &run_id, "spinoff", worktree.path(), "wt/test-x");

    // Missing the required `success` field.
    let report = scratch.path().join("bad.json");
    std::fs::write(&report, r#"{"summary": "no success field"}"#).unwrap();

    let merge_sh = fake_merge_sh(scratch.path(), 0, "");
    let out = bin(&home)
        .env("OCTL_MERGE_SH", &merge_sh)
        .args([
            "--output",
            "json",
            "run",
            "merge",
            &run_id,
            "--report-file",
            report.to_str().unwrap(),
        ])
        .output()
        .expect("spawn");
    assert!(!out.status.success());
    let err: Value = serde_json::from_slice(&out.stderr).expect("stderr JSON");
    assert_eq!(err["error"]["code"], "schema_violation");
    assert!(
        !scratch.path().join("merge.log").exists(),
        "merge must not run when the report file is invalid"
    );
    let events = run_dir(&home, &run_id).join("events.jsonl");
    assert_eq!(node_reports(&events).len(), 0);
}

/// A merge failure (conflict / dirty tree / lock timeout): the backend exits
/// non-zero, `run merge` surfaces `merge_failed`, and NO terminal report is
/// appended — the node stays live for the agent to recover and retry.
#[test]
fn failed_merge_surfaces_error_and_writes_no_report() {
    let home = TestHome::new();
    let scratch = TempDir::new().unwrap();
    let worktree = TempDir::new().unwrap();
    let run_id = create_run(&home, "code", "merge-fail");
    forge_worker_node(&home, &run_id, "code", worktree.path(), "wt/test-x");

    let merge_sh = fake_merge_sh(scratch.path(), 1, "Error: rebase conflict");
    let out = bin(&home)
        .env("OCTL_MERGE_SH", &merge_sh)
        .args(["--output", "json", "run", "merge", &run_id])
        .output()
        .expect("spawn");
    assert!(!out.status.success(), "merge failure must exit non-zero");
    let err: Value = serde_json::from_slice(&out.stderr).expect("stderr is JSON envelope");
    assert_eq!(err["error"]["code"], "merge_failed");

    let events = run_dir(&home, &run_id).join("events.jsonl");
    assert_eq!(
        node_reports(&events).len(),
        0,
        "a failed merge must not submit a terminal report"
    );
}

/// `--dry-run` resolves inputs and reports the planned merge without invoking
/// the backend or appending any event.
#[test]
fn dry_run_resolves_without_side_effects() {
    let home = TestHome::new();
    let scratch = TempDir::new().unwrap();
    let worktree = TempDir::new().unwrap();
    let run_id = create_run(&home, "code", "merge-dry");
    forge_worker_node(&home, &run_id, "code", worktree.path(), "wt/test-x");

    let merge_sh = fake_merge_sh(scratch.path(), 1, "should never run");
    let v = run_ok(bin(&home).env("OCTL_MERGE_SH", &merge_sh).args([
        "--output",
        "json",
        "run",
        "merge",
        &run_id,
        "--dry-run",
    ]));
    assert_eq!(v["data"]["dry_run"], true);
    assert_eq!(v["data"]["branch"], "wt/test-x");

    // The backend was never invoked and no report was written.
    assert!(
        !scratch.path().join("merge.log").exists(),
        "dry-run must not invoke the merge backend"
    );
    let events = run_dir(&home, &run_id).join("events.jsonl");
    assert_eq!(node_reports(&events).len(), 0);
}

/// A run id that names no run surfaces `run_not_found` (not a backend spawn).
#[test]
fn missing_run_is_run_not_found() {
    let home = TestHome::new();
    let out = bin(&home)
        .args([
            "--output",
            "json",
            "run",
            "merge",
            "01jxsnap000000000000000000",
        ])
        .output()
        .expect("spawn");
    assert!(!out.status.success());
    let err: Value = serde_json::from_slice(&out.stderr).expect("stderr is JSON envelope");
    assert_eq!(err["error"]["code"], "run_not_found");
}

/// Run `git <args>` in `cwd`, asserting success.
fn git(cwd: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("spawn git")
        .status
        .success();
    assert!(ok, "git {args:?} failed in {}", cwd.display());
}

/// True when local branch `branch` exists in `repo`.
fn branch_exists(repo: &Path, branch: &str) -> bool {
    Command::new("git")
        .current_dir(repo)
        .args(["rev-parse", "--verify", "--quiet", branch])
        .output()
        .expect("spawn git")
        .status
        .success()
}

/// Init a real repo on `main` with a linked worktree on `wt/foo`, returning
/// `(repo, worktree)` — enough for a full `git worktree remove` + `branch -D`
/// round-trip through `run merge`'s synchronous teardown.
fn init_repo_with_worktree(tmp: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let repo = tmp.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.email", "t@example.com"]);
    git(&repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join("README"), "x").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-qm", "init"]);
    let wt = tmp.join("wt");
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "wt/foo",
            wt.to_str().unwrap(),
        ],
    );
    (repo, wt)
}

/// Submit a terminal `node.report` for `n-0001` via the agent self-report path,
/// so a test can pre-terminalize a node the way the watchdog's synthesized
/// report does — before `run merge` runs.
fn append_node_report(home: &TempDir, run_id: &str, scratch: &Path, data: &str) {
    let f = scratch.join("pre-report.json");
    std::fs::write(&f, data).unwrap();
    run_ok(bin(home).args([
        "--output",
        "json",
        "node",
        "report",
        run_id,
        "n-0001",
        "--from-file",
        f.to_str().unwrap(),
    ]));
}

/// THE `merge-skips-teardown` / `agent-died-merge-no-teardown-interactive` fix
/// (issue `reducer-adopt-explicit-merge`): a long-lived interactive node the
/// watchdog falsely declared `agent-died` is already terminal when the still-alive
/// agent runs `run merge`. The octl-core reducer now ADOPTS the late
/// `via: "explicit-merge"` report even against that terminal node — overwriting
/// `last_report` and reconciling status to `Done` — so `any_node_merged_explicitly`
/// sees the merge and the SUPERVISOR (invariant #5) warrants teardown. `run merge`
/// no longer reclaims inline.
///
/// This run was never supervised (`--skip-materialize` skeleton), so there is no
/// live/restartable supervisor and the worktree/branch survive THIS call
/// (`supervisor: NotSupervised`) — real teardown is driven by a reattached
/// supervisor, proven end-to-end under a real detached supervisor in
/// `e2e_spinoff::swallowed_agent_died_then_merge_reattaches_and_tears_down`. Here
/// we assert the load-bearing projection change: the report is adopted.
#[test]
fn merge_adopts_swallowed_report_and_defers_teardown() {
    let home = TestHome::new();
    let scratch = TempDir::new().unwrap();
    let gitroot = TempDir::new().unwrap();
    let (repo, wt) = init_repo_with_worktree(gitroot.path());
    let run_id = create_run(&home, "code", "swallowed-merge");
    forge_worker_node(&home, &run_id, "code", &wt, "wt/foo");

    // Watchdog false positive: the node is terminalized as agent-died BEFORE the
    // merge. Pre-fix the reducer would swallow the explicit-merge report; now it
    // adopts it.
    append_node_report(
        &home,
        &run_id,
        scratch.path(),
        r#"{"success": false, "failed": true, "reason": "agent-died"}"#,
    );

    let merge_sh = fake_merge_sh(scratch.path(), 0, "");
    let v = run_ok(bin(&home).env("OCTL_MERGE_SH", &merge_sh).args([
        "--output", "json", "run", "merge", &run_id, "--source", "main",
    ]));

    assert_eq!(v["data"]["merged"], true);
    // Never supervised → no teardown actor to (re)start; the supervisor owns
    // teardown, so this call leaves the resources for it.
    assert_eq!(
        v["data"]["supervisor"]["state"], "not-supervised",
        "a never-supervised run has no teardown actor: {}",
        v["data"]
    );
    assert!(
        wt.exists(),
        "run merge no longer reclaims inline; the supervisor owns teardown"
    );
    assert!(
        branch_exists(&repo, "wt/foo"),
        "the branch is left for the supervisor"
    );

    // THE fix: the reducer ADOPTED the explicit-merge report onto the projection,
    // reconciling the watchdog-FAILED node to Done, so a supervisor can now warrant
    // teardown (contrast the pre-fix behavior, where last_report stayed agent-died).
    let node_show =
        run_ok(bin(&home).args(["--output", "json", "node", "show", &run_id, "n-0001"]));
    assert_eq!(node_show["data"]["last_report"]["via"], "explicit-merge");
    assert_eq!(node_show["data"]["status"], "done");
}

/// The healthy interactive path is unchanged: when the node is LIVE at merge
/// time the reducer adopts the `explicit-merge` report, so `run merge` leaves
/// teardown to the supervisor (invariant #5) and does NOT reclaim inline — the
/// worktree/branch survive this call (a real supervisor, absent in this test,
/// would tear them down). Guards against the fix over-reaching into the path
/// that already works.
#[test]
fn merge_defers_to_supervisor_when_report_adopted() {
    let home = TestHome::new();
    let scratch = TempDir::new().unwrap();
    let gitroot = TempDir::new().unwrap();
    let (repo, wt) = init_repo_with_worktree(gitroot.path());
    let run_id = create_run(&home, "code", "adopted-merge");
    forge_worker_node(&home, &run_id, "code", &wt, "wt/foo");

    // No pre-terminalization: the node is live, so the explicit-merge report is
    // adopted and a supervisor owns teardown.
    let merge_sh = fake_merge_sh(scratch.path(), 0, "");
    let v = run_ok(bin(&home).env("OCTL_MERGE_SH", &merge_sh).args([
        "--output", "json", "run", "merge", &run_id, "--source", "main",
    ]));

    assert_eq!(v["data"]["merged"], true);
    assert!(
        wt.exists(),
        "adopted path must NOT reclaim inline — the supervisor is the teardown actor"
    );
    assert!(
        branch_exists(&repo, "wt/foo"),
        "adopted path must leave the branch for the supervisor"
    );
    // The report was adopted onto the projection.
    let node_show =
        run_ok(bin(&home).args(["--output", "json", "node", "show", &run_id, "n-0001"]));
    assert_eq!(node_show["data"]["last_report"]["via"], "explicit-merge");
}

/// A FAILED merge (backend exits non-zero) on an already-terminal node must NOT
/// adopt or tear down anything — the worktree + branch survive and `run merge`
/// surfaces `merge_failed`. Guards the ordering: the terminal report is appended
/// (and thus the reducer's adoption + the supervisor's teardown are reachable)
/// ONLY AFTER `run_merge_sh` confirms the merge landed, so a failed merge can
/// never mark a branch merged or warrant its deletion.
#[test]
fn failed_merge_on_preterminal_node_reclaims_nothing() {
    let home = TestHome::new();
    let scratch = TempDir::new().unwrap();
    let gitroot = TempDir::new().unwrap();
    let (repo, wt) = init_repo_with_worktree(gitroot.path());
    let run_id = create_run(&home, "code", "swallowed-merge-fail");
    forge_worker_node(&home, &run_id, "code", &wt, "wt/foo");

    // Pre-terminalize the node so its report would be swallowed on a *successful*
    // merge — but here the merge itself fails.
    append_node_report(
        &home,
        &run_id,
        scratch.path(),
        r#"{"success": false, "failed": true, "reason": "agent-died"}"#,
    );

    let merge_sh = fake_merge_sh(scratch.path(), 1, "Error: rebase conflict");
    let out = bin(&home)
        .env("OCTL_MERGE_SH", &merge_sh)
        .args([
            "--output", "json", "run", "merge", &run_id, "--source", "main",
        ])
        .output()
        .expect("spawn");

    assert!(!out.status.success(), "a failed merge must exit non-zero");
    let err: Value = serde_json::from_slice(&out.stderr).expect("stderr is JSON envelope");
    assert_eq!(err["error"]["code"], "merge_failed");
    assert!(wt.exists(), "a failed merge must not reclaim the worktree");
    assert!(
        branch_exists(&repo, "wt/foo"),
        "a failed merge must not reclaim the branch"
    );
}
