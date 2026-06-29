//! Terminal run completion + worktree teardown (design.md §7.4, §7.5).
//!
//! Two cooperating responsibilities the per-run supervisor runs each tick:
//!
//!   1. **Run-status roll-up** ([`rollup_status`]). The reducer terminalizes
//!      *nodes* from `node.report` events but never the *run* — only `run
//!      cancel` ever produced a `run.status` event before this. So an agent
//!      that submits a successful terminal `node.report` left its run `pending`
//!      forever and the supervisor polling indefinitely
//!      (`supervisor-complete-run-on-terminal-report`). Here the supervisor —
//!      the single arbiter of its run's lifecycle — observes that every one of
//!      its own nodes (and every tracked child) is terminal and reports the
//!      aggregate `run.status` (Done if all nodes succeeded, Failed if any did
//!      not), mirroring how `cancel` appends a `run.status` under the run flock.
//!      The existing `all_work_done` loop then sees the manifest terminal and
//!      exits naturally.
//!
//!   2. **Cleanup on terminal transition** ([`cleanup_terminal_nodes`]).
//!      Once the run is terminal *and* cleanup is warranted, the supervisor
//!      closes each node's tmux window, removes its git worktree, and deletes
//!      its branch — so a `/worktree-spinoff` (and every other fire-and-forget
//!      kind) tears itself fully down with no manual `tmux kill-window` / `git
//!      worktree remove` / `git branch -D` (`supervisor-close-tmux-on-terminal`).
//!      Cleanup is warranted when the kind is autonomous
//!      ([`octl_core::Lifecycle::Autonomous`]) OR an interactive kind (`code`,
//!      `orchestrate`) reached terminal via an explicit `run merge`
//!      ([`any_node_merged_explicitly`]). At spawn time the human owns an
//!      interactive review window, so it is excluded; but running `run merge`
//!      is the user's signal that the window may close (issue
//!      `bundle-worktree-merge`).
//!
//! Every external command is best-effort and lenient: a missing tmux window, an
//! already-removed worktree, or a `git` refusal (locked / dirty tree) is logged
//! and stepped past, never fatal — the merge skill's own detached cleanup races
//! us and either actor finishing first leaves the other a clean no-op. The tmux
//! and git binaries are resolved through `TMUX_BIN` / `GIT_BIN` overrides (as
//! the watchdog already does for tmux) so tests can stub them.

use std::process::{Command, Stdio};

use octl_core::{
    append_and_apply_event, read_manifest_opt, read_node_opt, Node, NodeId, RunPaths, Status,
};
use serde_json::json;
use tracing::{info, warn};

/// The tmux binary, honoring the `TMUX_BIN` override (tests, non-default
/// installs). Mirrors [`crate::supervise::watchdog`].
fn tmux_bin() -> String {
    std::env::var("TMUX_BIN").unwrap_or_else(|_| "tmux".to_string())
}

/// The git binary, honoring the `GIT_BIN` override (tests). Defaults to `git`.
fn git_bin() -> String {
    std::env::var("GIT_BIN").unwrap_or_else(|_| "git".to_string())
}

/// Aggregate the terminal `run.status` a non-terminal run should record, or
/// `None` when it is not yet complete.
///
/// Returns `Some(Status::Done)` when the run has at least one node and every
/// node is `Done`; `Some(Status::Failed)` when every node is terminal but at
/// least one is `Failed`/`Cancelled`. Returns `None` when:
/// - the manifest is missing or already terminal (nothing to roll up),
/// - any tracked child run is still non-terminal (`children_all_terminal` is
///   false) — a driver must not complete before its children,
/// - the run has no nodes yet (a freshly-created run must not vacuously
///   complete), or
/// - any own node is still non-terminal (`Pending`/`Running`/`Blocked`).
///
/// The caller appends the returned status as a `run.status` event under a
/// deterministic idempotency key, so re-evaluating it every tick appends at
/// most once and a concurrent `run cancel` (which freezes the manifest
/// terminal) makes this a clean no-op.
pub fn rollup_status(paths: &RunPaths, children_all_terminal: bool) -> Option<Status> {
    let manifest = read_manifest_opt(paths).ok().flatten()?;
    if manifest.status.is_terminal() {
        return None;
    }
    if !children_all_terminal {
        return None;
    }
    let nodes = list_nodes(paths);
    if nodes.is_empty() {
        return None;
    }
    let mut any_failed = false;
    for n in &nodes {
        match n.status {
            Status::Done => {}
            Status::Failed | Status::Cancelled => any_failed = true,
            // Any live node means the run is not done yet.
            Status::Pending | Status::Running | Status::Blocked => return None,
        }
    }
    Some(if any_failed {
        Status::Failed
    } else {
        Status::Done
    })
}

/// True when any node's terminal `node.report` was submitted by an explicit
/// `run merge` — i.e. its `last_report` carries `via: "explicit-merge"`.
///
/// This is the gate that lets *interactive* kinds (`code`, `orchestrate`) be
/// torn down. At spawn time the human owns the review window, so interactive
/// kinds are excluded from auto-cleanup. But running `run merge` IS the user's
/// signal that the window has served its purpose: the verb stamps the report
/// with `via: "explicit-merge"`, and the supervisor reads that here to extend
/// the same teardown autonomous kinds always get (issue `bundle-worktree-merge`).
/// Autonomous kinds don't depend on this — they clean up on any terminal report.
pub fn any_node_merged_explicitly(paths: &RunPaths) -> bool {
    list_nodes(paths).iter().any(|n| {
        n.last_report
            .as_ref()
            .and_then(|r| r.get("via"))
            .and_then(serde_json::Value::as_str)
            == Some("explicit-merge")
    })
}

/// Close the tmux window, remove the worktree, and delete the branch for every
/// node of a run that has just reached a terminal status.
///
/// The caller must have already confirmed the run is terminal AND that cleanup
/// is warranted — either the kind is
/// [`Lifecycle::Autonomous`](octl_core::Lifecycle::Autonomous) or an
/// interactive kind reached terminal via an explicit `run merge`
/// ([`any_node_merged_explicitly`]). This function does not re-check (it is the
/// cleanup mechanism, not the gate). Every step is best-effort and never
/// panics, so a partially-torn-down run still makes forward progress.
pub fn cleanup_terminal_nodes(paths: &RunPaths) {
    let tmux = tmux_bin();
    let git = git_bin();
    for n in list_nodes(paths) {
        cleanup_node(paths, &n, &tmux, &git);
    }
}

/// Tear down one node's tmux window + worktree + branch. Order matters: derive
/// the main worktree *before* removing the linked worktree (a `git -C
/// <removed-path>` would then fail), and kill the tmux window first so the
/// agent's own Claude session ends before its worktree is pulled.
fn cleanup_node(paths: &RunPaths, n: &Node, tmux: &str, git: &str) {
    close_tmux_window(paths, n, tmux);

    let Some(worktree_path) = n.worktree_path.as_deref() else {
        // Nothing materialized for this node (e.g. a driver node) — only the
        // tmux window, if any, needed closing.
        return;
    };
    // The main worktree is the canonical place to run `worktree remove` /
    // `branch -D` from; resolve it while the linked worktree still exists.
    let main_repo = main_worktree_of(worktree_path, git);
    let repo = main_repo.as_deref().unwrap_or(worktree_path);
    remove_worktree(repo, worktree_path, git);
    if let Some(branch) = n.branch.as_deref() {
        delete_branch(repo, branch, git);
    }
}

/// Close the node's tmux window, recovering from the manual-rebase orphan case
/// (issue `worktree-merge-orphans-tmux-window`).
///
/// The primary target is the fully-qualified [`TmuxIdentity`] (stable `@NNNN`
/// window id on the recorded socket); a node registered before create.sh emitted
/// the qualified fields falls back to the legacy bare window *name*. A node with
/// neither has no window to close.
///
/// The kill is issued unconditionally first — we never precheck with
/// `list-windows`, because a transient empty/stale list would silently skip a
/// real kill and leak the window (the same hard-won rule the merge.sh cleanup
/// follows). Only *after* a kill that reports the target missing do we fall
/// back: when a user resolves a rebase conflict by hand, the manual
/// `git rebase --continue` / detached-HEAD state makes tmux auto-rename the
/// window, so the spawn-time name no longer matches and a name-based kill is a
/// silent no-op — orphaning the window. The pane's cwd is still the worktree,
/// though, so we re-find the window by [`Node::worktree_path`] and kill that.
/// If even that fails we record a non-fatal `cleanup.window_missing` audit event
/// so the orphan is visible instead of silent — cleanup never fails the run.
fn close_tmux_window(paths: &RunPaths, n: &Node, tmux: &str) {
    let socket = n
        .tmux_identity
        .as_ref()
        .and_then(|id| id.socket.as_deref())
        .filter(|s| !s.is_empty());
    let target = match n.tmux_identity.as_ref() {
        Some(id) => id.window_id.clone(),
        None => match n.tmux_window.as_deref() {
            Some(name) => name.to_string(),
            // Nothing to target — no qualified identity and no legacy name.
            None => return,
        },
    };

    // Always attempt the kill against the recorded target first.
    if tmux_kill_window(tmux, socket, &target) {
        return;
    }

    // The recorded target was not found. A manual rebase resolution can rename
    // the window or leave it on a detached HEAD, so the spawn-time id/name no
    // longer matches — but the window's pane is still parked in the worktree.
    // Re-find it by path and kill that before giving up.
    if let Some(worktree) = n.worktree_path.as_deref() {
        if let Some(recovered) = find_window_by_path(tmux, socket, worktree) {
            if recovered != target && tmux_kill_window(tmux, socket, &recovered) {
                info!(
                    target: "orchestratectl::supervise",
                    node = %n.node_id,
                    primary = %target,
                    recovered = %recovered,
                    "tmux window recovered by worktree path after the recorded target was missing"
                );
                eprintln!(
                    "supervisor cleanup: tmux window {target} missing; closed {recovered} found by worktree path"
                );
                return;
            }
        }
    }

    // Could not find the window by id/name or by worktree path: it is either
    // already gone (a benign race with merge.sh's own cleanup) or genuinely
    // orphaned. Record it, non-fatally, so a recurrence is auditable.
    record_window_missing(paths, n, &target);
}

/// Issue `tmux [-S <socket>] kill-window -t <target>` leniently; returns `true`
/// when tmux reported success (window found and killed). A non-zero exit
/// (typically "can't find window") returns `false` so the caller can fall back.
fn tmux_kill_window(tmux: &str, socket: Option<&str>, target: &str) -> bool {
    let mut cmd = Command::new(tmux);
    if let Some(s) = socket {
        cmd.args(["-S", s]);
    }
    cmd.args(["kill-window", "-t", target]);
    run_lenient(cmd, &format!("tmux kill-window -t {target}"))
}

/// Find the `window_id` of a tmux window whose active pane is parked inside
/// `worktree_path` (exact dir or a sub-path), across all sessions on the given
/// socket. This is the rename-proof handle the orphan-recovery path keys off:
/// a manually-resolved rebase mutates the branch/window name but not the pane's
/// cwd. `None` if tmux is unavailable, the server errors, or no pane matches.
fn find_window_by_path(tmux: &str, socket: Option<&str>, worktree_path: &str) -> Option<String> {
    let mut cmd = Command::new(tmux);
    if let Some(s) = socket {
        cmd.args(["-S", s]);
    }
    cmd.args([
        "list-windows",
        "-a",
        "-F",
        "#{window_id}\t#{pane_current_path}",
    ]);
    cmd.stderr(Stdio::null());
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    let prefix = format!("{worktree_path}/");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|line| {
            let (wid, path) = line.split_once('\t')?;
            let path = path.trim_end();
            if path == worktree_path || path.starts_with(&prefix) {
                let wid = wid.trim();
                (!wid.is_empty()).then(|| wid.to_string())
            } else {
                None
            }
        })
}

/// Append a non-fatal `cleanup.window_missing` audit event when a node's tmux
/// window could not be located for teardown. The event mutates no projection
/// (it folds to a clean no-op in the reducer); it exists so an orphaned window
/// is visible in the run log instead of a silent leak. Failure to append is
/// itself swallowed — recording the miss must never break cleanup.
fn record_window_missing(paths: &RunPaths, n: &Node, target: &str) {
    let method = if n.tmux_identity.is_some() {
        "window-id"
    } else {
        "window-name"
    };
    warn!(
        target: "orchestratectl::supervise",
        node = %n.node_id,
        window = %target,
        method,
        "tmux window not found during cleanup; recording cleanup.window_missing (run not failed)"
    );
    eprintln!(
        "supervisor cleanup: tmux window {target} not found (continuing; recorded cleanup.window_missing)"
    );
    let data = json!({
        "node_id": n.node_id.as_str(),
        "window": target,
        "method": method,
        "worktree_path": n.worktree_path,
    });
    // Idempotency key so a supervisor restart re-running cleanup does not append
    // a duplicate audit line for the same orphaned window.
    let key = format!(
        "cleanup.window_missing:{}:{}",
        paths.run_id.as_str(),
        n.node_id.as_str()
    );
    if let Err(e) = append_and_apply_event(
        paths,
        "cleanup.window_missing",
        Some(&n.node_id),
        Some(&key),
        data,
    ) {
        warn!(
            target: "orchestratectl::supervise",
            node = %n.node_id,
            error = %e,
            "failed to append cleanup.window_missing (continuing)"
        );
    }
}

/// Resolve the main worktree path for a linked worktree by reading the FIRST
/// `worktree <path>` line of `git -C <worktree_path> worktree list --porcelain`
/// (git always lists the main worktree first). `None` if git is unavailable, the
/// path is no longer a worktree, or the output is unparseable.
fn main_worktree_of(worktree_path: &str, git: &str) -> Option<String> {
    let out = Command::new(git)
        .arg("-C")
        .arg(worktree_path)
        .args(["worktree", "list", "--porcelain"])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|l| l.strip_prefix("worktree ").map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty())
}

/// `git -C <repo> worktree remove <worktree_path>` — lenient. A dirty/locked
/// tree makes git refuse; we log and proceed rather than force, leaving the
/// merge skill's `--force` cleanup (or an operator) to finish it.
fn remove_worktree(repo: &str, worktree_path: &str, git: &str) {
    let mut cmd = Command::new(git);
    cmd.arg("-C")
        .arg(repo)
        .args(["worktree", "remove", worktree_path]);
    run_lenient(cmd, &format!("git worktree remove {worktree_path}"));
}

/// `git -C <repo> branch -D <branch>` — force-delete, lenient. `-D` (not `-d`)
/// because a merged-and-removed worktree's branch is still worth dropping even
/// if git can't confirm the merge from the main worktree's vantage point.
fn delete_branch(repo: &str, branch: &str, git: &str) {
    let mut cmd = Command::new(git);
    cmd.arg("-C").arg(repo).args(["branch", "-D", branch]);
    run_lenient(cmd, &format!("git branch -D {branch}"));
}

/// Run a cleanup command, logging its outcome to both `tracing` and stderr
/// (captured to `supervisor.stderr.log`) so the user can audit the teardown.
/// Returns `true` only when the command exited successfully; a non-zero exit or
/// spawn error is logged at `warn`, swallowed, and reported as `false` — cleanup
/// is best-effort by contract, but the boolean lets a caller (the tmux-window
/// teardown) fall back rather than leak.
fn run_lenient(mut cmd: Command, label: &str) -> bool {
    cmd.stdout(Stdio::null()).stderr(Stdio::piped());
    match cmd.output() {
        Ok(out) if out.status.success() => {
            info!(target: "orchestratectl::supervise", step = label, "cleanup step ok");
            eprintln!("supervisor cleanup: {label}: ok");
            true
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let detail = stderr.trim();
            warn!(
                target: "orchestratectl::supervise",
                step = label,
                code = out.status.code(),
                detail = %detail,
                "cleanup step non-zero (treated as already-done/refused; continuing)"
            );
            eprintln!("supervisor cleanup: {label}: non-zero exit (continuing): {detail}");
            false
        }
        Err(e) => {
            warn!(
                target: "orchestratectl::supervise",
                step = label,
                error = %e,
                "cleanup step could not spawn (continuing)"
            );
            eprintln!("supervisor cleanup: {label}: spawn failed (continuing): {e}");
            false
        }
    }
}

/// Read every `nodes/*.json` projection for the run. Unreadable or
/// non-`node-id` entries are skipped; a missing `nodes/` dir yields an empty
/// list. Mirrors the watchdog's own scan.
fn list_nodes(paths: &RunPaths) -> Vec<Node> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(paths.nodes_dir()) else {
        return out;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let Ok(nid) = NodeId::parse_str(stem) else {
            continue;
        };
        if let Ok(Some(n)) = read_node_opt(paths, &nid) {
            out.push(n);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use octl_core::{append_and_apply_event, NodeId, RunPaths};
    use serde_json::json;
    use tempfile::TempDir;

    fn fresh_run(tmp: &TempDir) -> RunPaths {
        let run_id = "01jxsnap000000000000000000";
        let dir = tmp.path().join(run_id);
        std::fs::create_dir_all(&dir).unwrap();
        RunPaths::new(dir, run_id).unwrap()
    }

    fn nid(s: &str) -> NodeId {
        NodeId::parse_str(s).unwrap()
    }

    fn bootstrap(paths: &RunPaths, count: usize) {
        append_and_apply_event(
            paths,
            "run.created",
            None,
            None,
            json!({ "kind": "spinoff", "lifecycle": "autonomous", "title": "t" }),
        )
        .unwrap();
        for i in 1..=count {
            append_and_apply_event(
                paths,
                "node.created",
                Some(&nid(&format!("n-{i:04}"))),
                None,
                json!({ "kind": "spinoff" }),
            )
            .unwrap();
        }
    }

    fn report(paths: &RunPaths, node: &str, data: serde_json::Value) {
        append_and_apply_event(paths, "node.report", Some(&nid(node)), None, data).unwrap();
    }

    /// Forge a `node.created` carrying the worktree/tmux fields the cleanup path
    /// consumes, then read back the materialized [`Node`] projection. `data` is
    /// merged over a `{"kind":"spinoff"}` base so callers supply only the tmux /
    /// worktree shape they care about.
    fn forge_node(paths: &RunPaths, node: &str, mut data: serde_json::Value) -> Node {
        let obj = data.as_object_mut().unwrap();
        obj.entry("kind").or_insert_with(|| json!("spinoff"));
        append_and_apply_event(paths, "node.created", Some(&nid(node)), None, data).unwrap();
        read_node_opt(paths, &nid(node)).unwrap().unwrap()
    }

    /// Write an executable fake `tmux` to `<dir>/fake-tmux.sh` that appends each
    /// invocation's argv (space-joined, one line per call) to `<dir>/tmux.log`,
    /// runs `body` (raw bash — e.g. to branch on the subcommand and emit canned
    /// `list-windows` output or a non-zero exit), and falls through to `exit 0`.
    fn fake_tmux(dir: &std::path::Path, body: &str) -> String {
        use std::os::unix::fs::PermissionsExt as _;
        let p = dir.join("fake-tmux.sh");
        let log = dir.join("tmux.log");
        let script = format!(
            "#!/bin/bash\nprintf '%s ' \"$@\" >> '{log}'\nprintf '\\n' >> '{log}'\n{body}\nexit 0\n",
            log = log.display(),
        );
        std::fs::write(&p, script).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p.to_str().unwrap().to_string()
    }

    fn tmux_log(dir: &std::path::Path) -> String {
        std::fs::read_to_string(dir.join("tmux.log")).unwrap_or_default()
    }

    fn window_missing_events(paths: &RunPaths) -> Vec<serde_json::Value> {
        std::fs::read_to_string(paths.events())
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .filter(|v| v["kind"] == "cleanup.window_missing")
            .collect()
    }

    /// The happy path: `kill-window` succeeds against the recorded id, so no
    /// `list-windows` probe runs and no `cleanup.window_missing` is recorded.
    #[test]
    fn window_killed_by_id_no_fallback() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 0);
        let n = forge_node(
            &paths,
            "n-0001",
            json!({ "tmux_session": "octl", "tmux_window_id": "@42", "worktree_path": "/fake/wt" }),
        );
        let tmux = fake_tmux(tmp.path(), "");

        close_tmux_window(&paths, &n, &tmux);

        let log = tmux_log(tmp.path());
        assert!(log.contains("kill-window -t @42"), "log={log:?}");
        assert!(!log.contains("list-windows"), "must not probe on success");
        assert!(window_missing_events(&paths).is_empty());
    }

    /// The "window already gone" path: the recorded target is missing and no
    /// pane sits in the worktree, so the supervisor records a non-fatal
    /// `cleanup.window_missing` event and returns without panicking. Exercises
    /// the orphan-detection branch the issue calls for.
    #[test]
    fn missing_window_records_event_and_does_not_fail() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 0);
        let n = forge_node(
            &paths,
            "n-0001",
            json!({ "tmux_session": "octl", "tmux_window_id": "@42", "worktree_path": "/fake/wt" }),
        );
        // kill-window fails (window not found); list-windows lists an unrelated
        // window whose pane is NOT in the worktree → no path recovery.
        let tmux = fake_tmux(
            tmp.path(),
            r#"case "$*" in
                 *kill-window*) exit 1;;
                 *list-windows*) echo "@99	/some/other/path";;
               esac"#,
        );

        close_tmux_window(&paths, &n, &tmux);

        let events = window_missing_events(&paths);
        assert_eq!(events.len(), 1, "exactly one audit event expected");
        assert_eq!(events[0]["data"]["window"], "@42");
        assert_eq!(events[0]["data"]["method"], "window-id");
    }

    /// The root-cause recovery: a manually-resolved rebase renamed the window so
    /// the recorded id/name no longer matches, but the pane is still parked in
    /// the worktree. The supervisor re-finds the window by `worktree_path` and
    /// kills it — no orphan, no `cleanup.window_missing`.
    #[test]
    fn renamed_window_recovered_by_worktree_path() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 0);
        // Legacy node (no qualified identity) → name-based primary target.
        let n = forge_node(
            &paths,
            "n-0001",
            json!({ "tmux_window": "wt/abc-old-name", "worktree_path": "/fake/wt" }),
        );
        // kill-window fails for any name (the recorded name is stale), but
        // list-windows shows @55 parked in the worktree. The recovery kill of
        // @55 must succeed — so only the name kill exits non-zero.
        let tmux = fake_tmux(
            tmp.path(),
            r#"case "$*" in
                 *'kill-window -t @55'*) exit 0;;
                 *kill-window*) exit 1;;
                 *list-windows*) printf '@7\t/other\n@55\t/fake/wt\n';;
               esac"#,
        );

        close_tmux_window(&paths, &n, &tmux);

        let log = tmux_log(tmp.path());
        assert!(
            log.contains("kill-window -t wt/abc-old-name"),
            "primary kill attempted first: {log:?}"
        );
        assert!(
            log.contains("kill-window -t @55"),
            "recovered window killed by path: {log:?}"
        );
        assert!(
            window_missing_events(&paths).is_empty(),
            "recovery must not record a missing-window event"
        );
    }

    /// Recording is idempotent across supervisor restarts: a second cleanup pass
    /// over the same orphaned window reuses the idempotency key and appends no
    /// duplicate audit line.
    #[test]
    fn missing_window_event_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 0);
        let n = forge_node(
            &paths,
            "n-0001",
            json!({ "tmux_session": "octl", "tmux_window_id": "@42", "worktree_path": "/fake/wt" }),
        );
        let tmux = fake_tmux(
            tmp.path(),
            r#"case "$*" in *kill-window*) exit 1;; *list-windows*) ;; esac"#,
        );

        close_tmux_window(&paths, &n, &tmux);
        close_tmux_window(&paths, &n, &tmux);

        assert_eq!(window_missing_events(&paths).len(), 1);
    }

    #[test]
    fn rollup_none_while_a_node_is_live() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 2);
        report(&paths, "n-0001", json!({ "success": true }));
        // n-0002 still pending → not done.
        assert_eq!(rollup_status(&paths, true), None);
    }

    #[test]
    fn rollup_done_when_all_nodes_succeed() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 2);
        report(&paths, "n-0001", json!({ "success": true }));
        report(&paths, "n-0002", json!({ "success": true }));
        assert_eq!(rollup_status(&paths, true), Some(Status::Done));
    }

    #[test]
    fn rollup_failed_when_any_node_fails() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 2);
        report(&paths, "n-0001", json!({ "success": true }));
        report(&paths, "n-0002", json!({ "success": false }));
        assert_eq!(rollup_status(&paths, true), Some(Status::Failed));
    }

    #[test]
    fn rollup_failed_when_a_node_is_cancelled() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 1);
        report(
            &paths,
            "n-0001",
            json!({ "success": false, "cancelled": true, "reason": "x" }),
        );
        assert_eq!(rollup_status(&paths, true), Some(Status::Failed));
    }

    #[test]
    fn rollup_none_when_children_pending() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 1);
        report(&paths, "n-0001", json!({ "success": true }));
        // A child is still running: the driver must not complete.
        assert_eq!(rollup_status(&paths, false), None);
        // ...but once the child is terminal, it completes.
        assert_eq!(rollup_status(&paths, true), Some(Status::Done));
    }

    #[test]
    fn rollup_none_with_no_nodes() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        append_and_apply_event(
            &paths,
            "run.created",
            None,
            None,
            json!({ "kind": "spinoff", "lifecycle": "autonomous", "title": "t" }),
        )
        .unwrap();
        assert_eq!(rollup_status(&paths, true), None);
    }

    #[test]
    fn explicit_merge_marker_detected() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 1);
        // A plain success report (autonomous path) carries no `via`.
        report(&paths, "n-0001", json!({ "success": true }));
        assert!(!any_node_merged_explicitly(&paths));
    }

    #[test]
    fn explicit_merge_marker_present() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 2);
        report(&paths, "n-0001", json!({ "success": true }));
        // The `run merge` verb stamps the terminal report; any one node
        // carrying it warrants interactive-kind cleanup.
        report(
            &paths,
            "n-0002",
            json!({ "success": true, "via": "explicit-merge" }),
        );
        assert!(any_node_merged_explicitly(&paths));
    }

    #[test]
    fn other_via_value_does_not_trigger_cleanup() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 1);
        // A non-explicit-merge `via` (e.g. a future watchdog source) must
        // NOT extend cleanup to interactive kinds.
        report(
            &paths,
            "n-0001",
            json!({ "success": true, "via": "watchdog" }),
        );
        assert!(!any_node_merged_explicitly(&paths));
    }

    #[test]
    fn rollup_none_when_already_terminal() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 1);
        report(&paths, "n-0001", json!({ "success": true }));
        // Record the run terminal, then a re-evaluation must be a no-op.
        append_and_apply_event(
            &paths,
            "run.status",
            None,
            None,
            json!({ "status": "done" }),
        )
        .unwrap();
        assert_eq!(rollup_status(&paths, true), None);
    }
}
