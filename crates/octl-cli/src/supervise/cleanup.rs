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
//!   2. **Cleanup on terminal transition** ([`cleanup_terminal_autonomous`]).
//!      Once the run is terminal *and* its kind is autonomous
//!      ([`Lifecycle::Autonomous`]), the supervisor closes each node's tmux
//!      window, removes its git worktree, and deletes its branch — so a
//!      `/worktree-spinoff` (and every other fire-and-forget kind) tears itself
//!      fully down with no manual `tmux kill-window` / `git worktree remove` /
//!      `git branch -D` (`supervisor-close-tmux-on-terminal`). Interactive
//!      kinds (`code`, `orchestrate`) are skipped: the human owns that window.
//!
//! Every external command is best-effort and lenient: a missing tmux window, an
//! already-removed worktree, or a `git` refusal (locked / dirty tree) is logged
//! and stepped past, never fatal — the merge skill's own detached cleanup races
//! us and either actor finishing first leaves the other a clean no-op. The tmux
//! and git binaries are resolved through `TMUX_BIN` / `GIT_BIN` overrides (as
//! the watchdog already does for tmux) so tests can stub them.

use std::process::{Command, Stdio};

use octl_core::schema::TmuxIdentity;
use octl_core::{read_manifest_opt, read_node_opt, Node, NodeId, RunPaths, Status};
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

/// Close the tmux window, remove the worktree, and delete the branch for every
/// node of a run that has just reached a terminal status.
///
/// The caller must have already confirmed the run is terminal AND its kind is
/// [`Lifecycle::Autonomous`](octl_core::Lifecycle::Autonomous); this function
/// does not re-check (it is the cleanup mechanism, not the gate). Every step is
/// best-effort and never panics, so a partially-torn-down run still makes
/// forward progress.
pub fn cleanup_terminal_autonomous(paths: &RunPaths) {
    let tmux = tmux_bin();
    let git = git_bin();
    for n in list_nodes(paths) {
        cleanup_node(&n, &tmux, &git);
    }
}

/// Tear down one node's tmux window + worktree + branch. Order matters: derive
/// the main worktree *before* removing the linked worktree (a `git -C
/// <removed-path>` would then fail), and kill the tmux window first so the
/// agent's own Claude session ends before its worktree is pulled.
fn cleanup_node(n: &Node, tmux: &str, git: &str) {
    kill_tmux_window(n.tmux_identity.as_ref(), n.tmux_window.as_deref(), tmux);

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

/// `tmux [-S <socket>] kill-window -t <window_id|name>` for the node's window.
///
/// Prefers the fully-qualified [`TmuxIdentity`] (stable `@NNNN` window id on the
/// recorded socket) and falls back to the legacy bare window name on the default
/// socket. A node with neither has no window to close.
fn kill_tmux_window(identity: Option<&TmuxIdentity>, window_name: Option<&str>, tmux: &str) {
    let mut cmd = Command::new(tmux);
    let target = if let Some(id) = identity {
        if let Some(socket) = id.socket.as_deref().filter(|s| !s.is_empty()) {
            cmd.args(["-S", socket]);
        }
        id.window_id.clone()
    } else if let Some(name) = window_name {
        name.to_string()
    } else {
        // Nothing to target — no qualified identity and no legacy name.
        return;
    };
    cmd.args(["kill-window", "-t", &target]);
    run_lenient(cmd, &format!("tmux kill-window -t {target}"));
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
/// (captured to `supervisor.stderr.log`) so the user can audit the teardown. A
/// non-zero exit or spawn error is logged at `warn` and swallowed — cleanup is
/// best-effort by contract.
fn run_lenient(mut cmd: Command, label: &str) {
    cmd.stdout(Stdio::null()).stderr(Stdio::piped());
    match cmd.output() {
        Ok(out) if out.status.success() => {
            info!(target: "orchestratectl::supervise", step = label, "cleanup step ok");
            eprintln!("supervisor cleanup: {label}: ok");
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
        }
        Err(e) => {
            warn!(
                target: "orchestratectl::supervise",
                step = label,
                error = %e,
                "cleanup step could not spawn (continuing)"
            );
            eprintln!("supervisor cleanup: {label}: spawn failed (continuing): {e}");
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
