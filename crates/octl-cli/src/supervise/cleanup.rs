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
    // `branch -D` from; resolve it while the linked worktree still exists,
    // falling back to the run's recorded source repo so branch cleanup still
    // has a valid `-C` target even when the worktree dir is already gone.
    let main_repo = main_worktree_of(worktree_path, git).or_else(|| manifest_source_repo(paths));
    let repo = main_repo.as_deref().unwrap_or(worktree_path);

    // `--force` so disposable untracked/modified scratch left in the worktree
    // does not refuse removal and orphan the worktree+branch (issue
    // `supervisor-worktree-remove-no-force`). By teardown the tracked work is
    // already merged, so anything still in the tree is throwaway. If removal
    // still fails AND the dir is simply gone (user removed it manually), record
    // a non-fatal `cleanup.worktree_missing` and continue.
    if !remove_worktree(repo, worktree_path, git) && !std::path::Path::new(worktree_path).exists() {
        record_worktree_missing(paths, n, worktree_path);
    }
    if let Some(branch) = n.branch.as_deref() {
        delete_branch(paths, n, repo, branch, git);
    }
}

/// The run's recorded source repository (`manifest.source_repo`), if any. Used
/// as the `-C` fallback for `branch -D` when the linked worktree dir is gone and
/// [`main_worktree_of`] can no longer resolve the main worktree from it.
fn manifest_source_repo(paths: &RunPaths) -> Option<String> {
    read_manifest_opt(paths)
        .ok()
        .flatten()
        .and_then(|m| m.source_repo)
}

/// Close the node's tmux window, recovering from the manual-rebase orphan case
/// (issue `worktree-merge-orphans-tmux-window`).
///
/// The primary target is the fully-qualified [`TmuxIdentity`](octl_core::schema::TmuxIdentity) (stable `@NNNN`
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
    //
    // **Safety constraint** (issue `find-window-by-path-cross-session-kill`):
    // scope the lookup to the spawn-time session and require the pane's cwd
    // to *equal* the worktree root, not just live inside it. Without this an
    // unrelated tmux pane (the user's main work pane, a sibling spinoff, a
    // `/worktree-code` review pane in another session) that happened to cd
    // into the worktree would be killed by `tmux kill-window`. The supervisor
    // already knows which session owns its window; query only that one.
    let session = n
        .tmux_identity
        .as_ref()
        .map(|id| id.session.as_str())
        .filter(|s| !s.is_empty());
    if let Some(worktree) = n.worktree_path.as_deref() {
        if let Some(recovered) = find_window_by_path(tmux, socket, session, worktree) {
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

/// Find the `window_id` of a tmux window whose active pane's cwd is **exactly**
/// `worktree_path`, scoped to a single session when possible.
///
/// This is the rename-proof handle the orphan-recovery path keys off — a
/// manually-resolved rebase mutates the branch/window name but not the pane's
/// cwd. Two safety constraints (issue `find-window-by-path-cross-session-kill`):
///
/// 1. **Session-scoped.** When `session` is `Some`, query `tmux list-windows -t
///    <session>`; otherwise fall back to `-a` (the supervisor lacks a session
///    record — pre-qualified-identity nodes). Without this scope, an unrelated
///    pane in a different session that happened to cd into the worktree would
///    match and get killed.
/// 2. **Exact-match cwd.** Match only `path == worktree_path`; never a
///    sub-path. A sibling pane that cd'd one level deeper into the worktree
///    (`worktree/src/foo`) would otherwise match and die.
///
/// `None` if tmux is unavailable, the server errors, or no pane matches.
fn find_window_by_path(
    tmux: &str,
    socket: Option<&str>,
    session: Option<&str>,
    worktree_path: &str,
) -> Option<String> {
    let mut cmd = Command::new(tmux);
    if let Some(s) = socket {
        cmd.args(["-S", s]);
    }
    match session {
        Some(name) => cmd.args(["list-windows", "-t", name]),
        None => cmd.args(["list-windows", "-a"]),
    };
    cmd.args(["-F", "#{window_id}\t#{pane_current_path}"]);
    cmd.stderr(Stdio::null());
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|line| {
            let (wid, path) = line.split_once('\t')?;
            if path.trim_end() != worktree_path {
                return None;
            }
            let wid = wid.trim();
            (!wid.is_empty()).then(|| wid.to_string())
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

/// Append a non-fatal `cleanup.worktree_missing` audit event when a node's
/// worktree dir is already gone at teardown (e.g. the operator removed it by
/// hand). `git worktree remove --force` then has nothing to remove, so cleanup
/// records the miss and continues — it must never fail the run. Idempotent by
/// `(run, node)` so a supervisor restart re-running cleanup appends at most once.
fn record_worktree_missing(paths: &RunPaths, n: &Node, worktree_path: &str) {
    warn!(
        target: "orchestratectl::supervise",
        node = %n.node_id,
        worktree_path,
        "worktree dir already gone during cleanup; recording cleanup.worktree_missing (run not failed)"
    );
    eprintln!(
        "supervisor cleanup: worktree {worktree_path} already gone (continuing; recorded cleanup.worktree_missing)"
    );
    let data = json!({
        "node_id": n.node_id.as_str(),
        "worktree_path": worktree_path,
        "branch": n.branch,
    });
    let key = format!(
        "cleanup.worktree_missing:{}:{}",
        paths.run_id.as_str(),
        n.node_id.as_str()
    );
    if let Err(e) = append_and_apply_event(
        paths,
        "cleanup.worktree_missing",
        Some(&n.node_id),
        Some(&key),
        data,
    ) {
        warn!(
            target: "orchestratectl::supervise",
            node = %n.node_id,
            error = %e,
            "failed to append cleanup.worktree_missing (continuing)"
        );
    }
}

/// Append a non-fatal `cleanup.branch_remove_failed` audit event when
/// `git branch -D` refuses (e.g. the branch unexpectedly has unmerged commits).
/// Branch-cleanup failures must never block run completion, so the supervisor
/// records the git stderr for the operator and continues. Idempotent by
/// `(run, node)`.
fn record_branch_remove_failed(paths: &RunPaths, n: &Node, branch: &str, detail: &str) {
    warn!(
        target: "orchestratectl::supervise",
        node = %n.node_id,
        branch,
        detail,
        "git branch -D failed during cleanup; recording cleanup.branch_remove_failed (run not failed)"
    );
    eprintln!(
        "supervisor cleanup: branch {branch} delete failed (continuing; recorded cleanup.branch_remove_failed): {detail}"
    );
    let data = json!({
        "node_id": n.node_id.as_str(),
        "branch": branch,
        "error": detail,
    });
    let key = format!(
        "cleanup.branch_remove_failed:{}:{}",
        paths.run_id.as_str(),
        n.node_id.as_str()
    );
    if let Err(e) = append_and_apply_event(
        paths,
        "cleanup.branch_remove_failed",
        Some(&n.node_id),
        Some(&key),
        data,
    ) {
        warn!(
            target: "orchestratectl::supervise",
            node = %n.node_id,
            error = %e,
            "failed to append cleanup.branch_remove_failed (continuing)"
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

/// `git -C <repo> worktree remove --force <worktree_path>` — lenient. `--force`
/// because by teardown the tracked work is already merged, so any untracked /
/// modified scratch left behind is disposable; without it git refuses to remove
/// a dirty tree and the cascade orphans the worktree AND branch (issue
/// `supervisor-worktree-remove-no-force`). Returns `true` on success so the
/// caller can distinguish an already-gone worktree from a genuine refusal.
fn remove_worktree(repo: &str, worktree_path: &str, git: &str) -> bool {
    let mut cmd = Command::new(git);
    cmd.arg("-C")
        .arg(repo)
        .args(["worktree", "remove", "--force", worktree_path]);
    run_lenient(cmd, &format!("git worktree remove --force {worktree_path}"))
}

/// `git -C <repo> branch -D <branch>` — force-delete, lenient. `-D` (not `-d`)
/// because a merged-and-removed worktree's branch is still worth dropping even
/// if git can't confirm the merge from the main worktree's vantage point. If git
/// refuses anyway (e.g. unexpected unmerged commits), record a non-fatal
/// `cleanup.branch_remove_failed` audit event and continue — branch-cleanup
/// failures must never block run completion.
fn delete_branch(paths: &RunPaths, n: &Node, repo: &str, branch: &str, git: &str) {
    let mut cmd = Command::new(git);
    cmd.arg("-C").arg(repo).args(["branch", "-D", branch]);
    if let Some(detail) = run_lenient_detail(cmd, &format!("git branch -D {branch}")) {
        record_branch_remove_failed(paths, n, branch, &detail);
    }
}

/// Run a cleanup command, logging its outcome to both `tracing` and stderr
/// (captured to `supervisor.stderr.log`) so the user can audit the teardown.
/// Returns `true` only when the command exited successfully; a non-zero exit or
/// spawn error is logged at `warn`, swallowed, and reported as `false` — cleanup
/// is best-effort by contract, but the boolean lets a caller (the tmux-window
/// teardown) fall back rather than leak.
fn run_lenient(cmd: Command, label: &str) -> bool {
    run_lenient_detail(cmd, label).is_none()
}

/// Like [`run_lenient`] but returns the captured failure detail on a non-zero
/// exit or spawn error (`None` on success), so a caller can record it in an
/// audit event (e.g. `cleanup.branch_remove_failed`). Logging is identical.
fn run_lenient_detail(mut cmd: Command, label: &str) -> Option<String> {
    cmd.stdout(Stdio::null()).stderr(Stdio::piped());
    match cmd.output() {
        Ok(out) if out.status.success() => {
            info!(target: "orchestratectl::supervise", step = label, "cleanup step ok");
            eprintln!("supervisor cleanup: {label}: ok");
            None
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let detail = stderr.trim().to_string();
            warn!(
                target: "orchestratectl::supervise",
                step = label,
                code = out.status.code(),
                detail = %detail,
                "cleanup step non-zero (treated as already-done/refused; continuing)"
            );
            eprintln!("supervisor cleanup: {label}: non-zero exit (continuing): {detail}");
            Some(detail)
        }
        Err(e) => {
            warn!(
                target: "orchestratectl::supervise",
                step = label,
                error = %e,
                "cleanup step could not spawn (continuing)"
            );
            eprintln!("supervisor cleanup: {label}: spawn failed (continuing): {e}");
            Some(format!("spawn failed: {e}"))
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

    /// All events of a given `kind` recorded in the run's event log.
    fn events_of_kind(paths: &RunPaths, kind: &str) -> Vec<serde_json::Value> {
        std::fs::read_to_string(paths.events())
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .filter(|v| v["kind"] == kind)
            .collect()
    }

    fn window_missing_events(paths: &RunPaths) -> Vec<serde_json::Value> {
        events_of_kind(paths, "cleanup.window_missing")
    }

    /// Run `git <args>` in `cwd`, asserting success. Used by the real-git
    /// cleanup tests below (no `GIT_BIN` stub — `git_bin()` defaults to `git`,
    /// so cleanup drives a real repo end-to-end).
    fn git(cwd: &std::path::Path, args: &[&str]) {
        let ok = Command::new("git")
            .current_dir(cwd)
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?} failed in {cwd:?}");
    }

    /// Init a real repo with one commit on `main` and a linked worktree on a new
    /// branch `wt/foo`, returning `(repo, worktree)`. The cleanup path resolves
    /// the main worktree from the linked one, so this is enough for a full
    /// `worktree remove` / `branch -D` round-trip against real git.
    fn init_repo_with_worktree(tmp: &TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.email", "t@example.com"]);
        git(&repo, &["config", "user.name", "t"]);
        std::fs::write(repo.join("README"), "x").unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-qm", "init"]);
        let wt = tmp.path().join("wt");
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

    /// True when branch `branch` still exists in `repo`.
    fn branch_exists(repo: &std::path::Path, branch: &str) -> bool {
        Command::new("git")
            .current_dir(repo)
            .args(["rev-parse", "--verify", "--quiet", branch])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success()
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

    /// The root-cause regression (`supervisor-worktree-remove-no-force`): an
    /// untracked scratch file left in the worktree must NOT block teardown. With
    /// `--force` the worktree dir AND its branch are both removed; without it git
    /// refused and the cascade orphaned both. Drives real git end-to-end.
    #[test]
    fn worktree_with_untracked_file_is_force_removed_with_branch() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 0);
        let (repo, wt) = init_repo_with_worktree(&tmp);
        // The exact orphan trigger from the issue: a stray untracked file.
        std::fs::write(wt.join(".report.json"), "scratch").unwrap();

        let n = forge_node(
            &paths,
            "n-0001",
            json!({ "worktree_path": wt.to_str().unwrap(), "branch": "wt/foo" }),
        );
        // No tmux fields → close_tmux_window is a no-op; `/usr/bin/true` is never
        // actually consulted but satisfies the signature.
        cleanup_node(&paths, &n, "/usr/bin/true", &git_bin());

        assert!(!wt.exists(), "worktree dir must be force-removed");
        assert!(
            !branch_exists(&repo, "wt/foo"),
            "branch must be deleted once the worktree is gone"
        );
        assert!(
            events_of_kind(&paths, "cleanup.worktree_missing").is_empty(),
            "a present (if dirty) worktree is not 'missing'"
        );
        assert!(
            events_of_kind(&paths, "cleanup.branch_remove_failed").is_empty(),
            "branch removal must succeed"
        );
    }

    /// Required behaviour #2: a worktree dir that is already gone (operator
    /// removed it by hand) records a non-fatal `cleanup.worktree_missing` and
    /// does NOT fail the run. No manifest `source_repo` is set, so the `-C`
    /// fallback lands on the (missing) worktree path and git refuses — exactly
    /// the path that proves the existence check, not a stub, decides "missing".
    #[test]
    fn missing_worktree_records_event_and_continues() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 0);
        let gone = tmp.path().join("gone-wt");
        // Branch left unset → keep this test focused on the worktree_missing arm.
        let n = forge_node(
            &paths,
            "n-0001",
            json!({ "worktree_path": gone.to_str().unwrap() }),
        );

        cleanup_node(&paths, &n, "/usr/bin/true", &git_bin());

        let evs = events_of_kind(&paths, "cleanup.worktree_missing");
        assert_eq!(evs.len(), 1, "exactly one worktree_missing event expected");
        assert_eq!(evs[0]["data"]["worktree_path"], gone.to_str().unwrap());
    }

    /// Required behaviour #3: when `git branch -D` itself refuses (here: the
    /// branch does not exist), the worktree is still removed and the failure is
    /// recorded as a non-fatal `cleanup.branch_remove_failed` — run completion is
    /// never blocked on branch cleanup.
    #[test]
    fn branch_remove_failure_records_event_and_continues() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 0);
        let (_repo, wt) = init_repo_with_worktree(&tmp);

        // Name a branch that does not exist so `git branch -D` refuses, while the
        // worktree removal still succeeds.
        let n = forge_node(
            &paths,
            "n-0001",
            json!({ "worktree_path": wt.to_str().unwrap(), "branch": "wt/never-existed" }),
        );

        cleanup_node(&paths, &n, "/usr/bin/true", &git_bin());

        assert!(!wt.exists(), "worktree must still be removed");
        let evs = events_of_kind(&paths, "cleanup.branch_remove_failed");
        assert_eq!(evs.len(), 1, "branch failure must be recorded once");
        assert_eq!(evs[0]["data"]["branch"], "wt/never-existed");
        assert!(
            !evs[0]["data"]["error"]
                .as_str()
                .unwrap_or_default()
                .is_empty(),
            "the git stderr must be captured for the operator"
        );
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
