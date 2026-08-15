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
//!      **Blocked reports are the exception to teardown.** A node whose terminal
//!      `node.report` is a BLOCKED handoff (`success: false` with no
//!      `via: "explicit-merge"`) classifies as
//!      [`TerminalOutcome::Blocked`](crate::supervise::outcome::TerminalOutcome) —
//!      a `Teardown::PreserveWork` policy — because it committed work that
//!      was never merged. Deleting its branch/worktree is silent data loss (issue
//!      `blocked-report-deletes-branch`), so the run still winds down (its tmux
//!      window may close) but its branch AND worktree are preserved for the human
//!      to pick up. As defense-in-depth, branch deletion on every non-merge path
//!      uses `git branch -d` (which refuses an unmerged branch) rather than the
//!      force `-D`, which is reserved for a confirmed `run merge`.
//!
//! Every external command is best-effort and lenient: a missing tmux window, an
//! already-removed worktree, or a `git` refusal (locked / dirty tree) is logged
//! and stepped past, never fatal — the merge skill's own detached cleanup races
//! us and either actor finishing first leaves the other a clean no-op. The tmux
//! and git binaries are resolved through `TMUX_BIN` / `GIT_BIN` overrides (as
//! the watchdog already does for tmux) so tests can stub them.

// Production git subprocesses now route through `crate::git::repo::Git`; the raw
// `Command`/`Stdio` types are only used by the real-git test fixtures below.
#[cfg(test)]
use std::process::{Command, Stdio};

use octl_core::{
    append_and_apply_event, read_manifest_opt, read_node_opt, Node, NodeId, RunPaths, Status,
    VIA_EXPLICIT_MERGE,
};

use crate::git::repo::Git;
use crate::multiplexer::tmux::Tmux;
use serde_json::json;
use tracing::{info, warn};

/// The tmux binary, honoring the `TMUX_BIN` override (tests, non-default
/// installs). Mirrors [`crate::supervise::watchdog`].
pub(crate) fn tmux_bin() -> String {
    std::env::var("TMUX_BIN").unwrap_or_else(|_| "tmux".to_string())
}

/// The git binary, honoring the `GIT_BIN` override (tests). Defaults to `git`.
pub(crate) fn git_bin() -> String {
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
    list_nodes(paths).iter().any(node_merged_explicitly)
}

/// True when THIS node's terminal `node.report` was submitted by an explicit
/// `run merge` (`last_report.via == "explicit-merge"`). This is the per-node
/// form [`any_node_merged_explicitly`] folds over, and the gate that decides
/// whether the node's branch may be force-deleted (`git branch -D`): only a
/// confirmed merge earns the force delete; every other terminal outcome falls
/// back to the safe `git branch -d`, which refuses an unmerged branch.
fn node_merged_explicitly(n: &Node) -> bool {
    report_via(n) == Some(VIA_EXPLICIT_MERGE)
}

/// The `via` field of a node's terminal report, if any.
fn report_via(n: &Node) -> Option<&str> {
    n.last_report
        .as_ref()
        .and_then(|r| r.get("via"))
        .and_then(serde_json::Value::as_str)
}

/// The `cleanup.branch_preserved` audit reason for a `Teardown::PreserveWork`
/// outcome — distinguishing a supervisor/worker `Failed` from an agent `Blocked`
/// handoff for observability. Both preserve identically, so the label never
/// changes teardown behavior.
fn preserve_reason(outcome: Option<crate::supervise::outcome::TerminalOutcome>) -> &'static str {
    match outcome {
        Some(crate::supervise::outcome::TerminalOutcome::Failed) => "failed (branch preserved)",
        _ => "blocked report",
    }
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

/// Kill the managed `--headless` / `--tmux-session` session orchestratectl
/// created for this run, once its last managed window has been torn down — so an
/// otherwise-empty session is not left lingering with only its synthetic
/// bootstrap shell window (issue `headless-tmux-session-not-torn-down`).
///
/// tmux opens a default shell window (`zsh`/`bash`) when a session is first
/// created (`tmux new-session -d`), and the agent windows are added alongside
/// it. Closing every agent window therefore does NOT remove the session — the
/// bootstrap window keeps it alive. This is the localized teardown that drops it.
///
/// Three safety gates, ALL required before the session is killed:
///
/// 1. **Managed-session only.** The name comes from
///    [`Manifest::managed_tmux_session`](octl_core::Manifest), recorded at spawn
///    time *only* when the run used `--parent-session` (headless). A foreground
///    run records `None`, so the user's own session is never a candidate — we
///    never kill a session orchestratectl did not create.
/// 2. **Not attached.** If a human attached to inspect it (`tmux attach -t
///    <name>`), the session is left alone — killing it would yank their
///    terminal. A `cleanup.session_retained` audit event records the skip.
/// 3. **No managed windows remain.** Every surviving window must be a synthetic
///    default shell ([`is_synthetic_default_window`]). A sibling run still
///    working in the same session keeps a non-default agent window alive, so the
///    *last* run to finish is the one that finds only the bootstrap shell and
///    kills the session — the multi-run teardown the original report described.
///
/// Best-effort throughout: a vanished session, an unavailable tmux, or a kill
/// refusal is logged and stepped past — session teardown never fails the run.
pub fn cleanup_managed_session(paths: &RunPaths) {
    cleanup_managed_session_with(paths, &tmux_bin());
}

/// [`cleanup_managed_session`] with the tmux binary injected, so tests can drive
/// the teardown against a stub without racing on the `TMUX_BIN` env var.
fn cleanup_managed_session_with(paths: &RunPaths, tmux: &str) {
    let Some(session) = managed_session(paths) else {
        // Foreground run (no managed session) — never a teardown candidate.
        return;
    };
    let socket = managed_session_socket(paths, &session);
    let mux = Tmux::with_bin(tmux);
    let Some((attached, names)) = mux.list_session_windows(socket.as_deref(), &session) else {
        // The session is already gone (its last window was the agent's, with no
        // surviving bootstrap shell) or tmux is unavailable — nothing to do.
        return;
    };
    if attached {
        // A human is attached: leave the session for them, record the skip.
        record_session_retained(paths, &session);
        return;
    }
    if names.is_empty() || !names.iter().all(|n| is_synthetic_default_window(n)) {
        // A non-default window survives — another run's agent is still working
        // in this shared session, or the window set is unexpected. Either way
        // the session is still in use; the last run to finish will kill it.
        return;
    }
    if mux.kill_session(socket.as_deref(), &session) {
        record_session_killed(paths, &session);
    }
}

/// The run's managed headless session name, set at spawn time only for a
/// `--headless` / `--tmux-session` run. `None` for a foreground run (whose
/// window lives in the user's own session) or a missing manifest.
fn managed_session(paths: &RunPaths) -> Option<String> {
    read_manifest_opt(paths)
        .ok()
        .flatten()
        .and_then(|m| m.managed_tmux_session)
}

/// The tmux server socket the managed session lives on, read from the first
/// node whose [`TmuxIdentity`](octl_core::schema::TmuxIdentity) names that
/// session. `None` falls back to tmux's default socket — which is where
/// create.sh's `tmux new-session -d` bootstraps a headless session anyway.
fn managed_session_socket(paths: &RunPaths, session: &str) -> Option<String> {
    for n in list_nodes(paths) {
        if let Some(id) = n.tmux_identity {
            if id.session == session {
                if let Some(sock) = id.socket {
                    if !sock.is_empty() {
                        return Some(sock);
                    }
                }
            }
        }
    }
    None
}

/// A window tmux opened as the synthetic bootstrap shell for a freshly-created
/// session — never an orchestratectl agent window (those carry the worktree /
/// branch name, usually with an emoji prefix). Matching this small set of login
/// shells is the heuristic the teardown keys off (issue
/// `headless-tmux-session-not-torn-down`). Conservative by design: an unknown
/// name is treated as a real window, so the session is *retained* rather than
/// risk killing one that is still in use.
fn is_synthetic_default_window(name: &str) -> bool {
    matches!(
        name.trim(),
        "zsh" | "bash" | "sh" | "fish" | "-zsh" | "-bash" | "-sh"
    )
}

/// Append a non-fatal `cleanup.session_killed` audit event when the managed
/// headless session was torn down. Idempotent by run id so a supervisor restart
/// re-running cleanup appends at most once. Recording the kill must never break
/// cleanup, so an append failure is itself swallowed.
fn record_session_killed(paths: &RunPaths, session: &str) {
    info!(
        target: "orchestratectl::supervise",
        session,
        "killed empty managed headless tmux session after last managed window torn down"
    );
    eprintln!("supervisor cleanup: tmux kill-session -t {session}: ok (empty managed session)");
    let data = json!({ "session": session });
    let key = format!("cleanup.session_killed:{}", paths.run_id.as_str());
    if let Err(e) = append_and_apply_event(paths, "cleanup.session_killed", None, Some(&key), data)
    {
        warn!(
            target: "orchestratectl::supervise",
            session,
            error = %e,
            "failed to append cleanup.session_killed (continuing)"
        );
    }
}

/// Append a non-fatal `cleanup.session_retained` audit event when the managed
/// headless session was left alive because a human had attached to it. Idempotent
/// by run id; an append failure is swallowed.
fn record_session_retained(paths: &RunPaths, session: &str) {
    info!(
        target: "orchestratectl::supervise",
        session,
        "managed headless tmux session is attached; leaving it for the human"
    );
    eprintln!("supervisor cleanup: tmux session {session} attached; not killing (recorded cleanup.session_retained)");
    let data = json!({ "session": session });
    let key = format!("cleanup.session_retained:{}", paths.run_id.as_str());
    if let Err(e) =
        append_and_apply_event(paths, "cleanup.session_retained", None, Some(&key), data)
    {
        warn!(
            target: "orchestratectl::supervise",
            session,
            error = %e,
            "failed to append cleanup.session_retained (continuing)"
        );
    }
}

/// Tear down one node's tmux window + worktree + branch. Order matters: derive
/// the main worktree *before* removing the linked worktree (a `git -C
/// <removed-path>` would then fail), and kill the tmux window first so the
/// agent's own Claude session ends before its worktree is pulled.
pub(crate) fn cleanup_node(paths: &RunPaths, n: &Node, tmux: &str, git: &str) {
    close_tmux_window(paths, n, tmux);

    let Some(worktree_path) = n.worktree_path.as_deref() else {
        // Nothing materialized for this node (e.g. a driver node) — only the
        // tmux window, if any, needed closing.
        return;
    };

    // The typed outcome table (design.md §2.6 / A6) is the SINGLE authority for
    // the teardown policy — never a re-derivation from a signal combination. A
    // node with no terminal report classifies as `None`; treat it as the
    // conservative source-relative policy (preserve any unmerged work).
    let outcome = crate::supervise::outcome::TerminalOutcome::classify(n);
    let teardown = outcome.map_or(
        crate::supervise::outcome::Teardown::SourceRelative,
        crate::supervise::outcome::TerminalOutcome::teardown,
    );

    // PreserveWork: a BLOCKED human handoff, or any non-merge failure (told
    // worker-exit, confirmed-death crash backstop). The agent committed work that
    // was never merged — its branch AND worktree must survive so a human / the
    // salvage skill can pick it up; tearing them down here is the silent data loss
    // of issue `blocked-report-deletes-branch` (invariant 5). Wind the run down
    // (the tmux window above may close) but leave the tree and branch untouched.
    if teardown == crate::supervise::outcome::Teardown::PreserveWork {
        record_branch_preserved(
            paths,
            n,
            n.branch.as_deref(),
            worktree_path,
            preserve_reason(outcome),
        );
        return;
    }

    // The main worktree is the canonical place to run `worktree remove` /
    // `branch -{d,D}` / `rev-list` from; resolve it while the linked worktree
    // still exists, falling back to the run's recorded source repo so branch
    // cleanup still has a valid `-C` target even when the worktree dir is gone.
    let main_repo = main_worktree_of(worktree_path, git).or_else(|| manifest_source_repo(paths));
    let repo = main_repo.as_deref().unwrap_or(worktree_path);

    // Only a confirmed explicit `run merge` (`Teardown::Full`) earns the force
    // `-D` teardown AND skips the source-relative unmerged-work check below — its
    // rebase/squash legitimately leaves the branch "ahead" of source, so the check
    // would false-positive. Every other reached-here outcome (cancel, a plain
    // success that skipped `run merge`) is `Teardown::SourceRelative`.
    let merged = outcome.is_some_and(crate::supervise::outcome::TerminalOutcome::is_explicit_merge);
    let skip_source_check = merged;

    // Defense-in-depth against any future outcome-gating miss (issue
    // `blocked-report-deletes-branch`): on ANY path other than a confirmed explicit
    // merge — a plain success that skipped `run merge`, a `run cancel`, or a
    // terminal outcome not yet gated above —
    // if the branch carries commits not reachable from the run's OWN source branch
    // (`manifest.source_branch`), preserve BOTH the worktree and the branch rather
    // than force anything. This protects committed work from being discarded even
    // when the primary gate does not fire. The ancestry check is against the run's
    // source branch, NOT the main worktree's ambient `HEAD` — which may be on any
    // branch when the supervisor ticks.
    //
    // The check FAILS CLOSED (issue `non-merge-teardown-dirty-worktree`): if the
    // rev-list count cannot be computed (a git error / unparseable output), the
    // teardown preserves rather than proceeds. The older code returned "nothing
    // unmerged" on a git error and leaned on `git branch -d` to catch it — but the
    // worktree was still removed on that path, so a transient git failure could
    // discard a live tree. For teardown, an unverifiable branch is treated exactly
    // like a provably-unmerged one.
    if !skip_source_check {
        if let (Some(branch), Some(source)) = (n.branch.as_deref(), manifest_source_branch(paths)) {
            match branch_unmerged_vs_source(repo, &source, branch, git) {
                UnmergedCheck::HasUnmerged => {
                    record_branch_preserved(
                        paths,
                        n,
                        Some(branch),
                        worktree_path,
                        "unmerged commits vs source (no explicit merge)",
                    );
                    return;
                }
                UnmergedCheck::Unverifiable => {
                    record_branch_preserved(
                        paths,
                        n,
                        Some(branch),
                        worktree_path,
                        "unmerged-commit check unavailable (git error; preserving)",
                    );
                    return;
                }
                UnmergedCheck::NoUnmerged => {}
            }
        }

        // Dirty-worktree guard (issue `non-merge-teardown-dirty-worktree`): the
        // source-relative check above protects COMMITTED work, but a
        // non-explicit-merge teardown (a `run cancel`, a plain success that skipped
        // `run merge`, a no-report node caught by a cancel) can still reach a
        // worktree holding UNCOMMITTED staged/modified/untracked edits that
        // `worktree remove --force` would silently discard. Only a confirmed
        // explicit `run merge` authorizes forced cleanup. If the tree is dirty — or
        // its cleanliness cannot be verified (`worktree_is_clean` is conservatively
        // false on a git error, i.e. fail-closed) — preserve BOTH the worktree and
        // the branch so the human / salvage skill can pick the edits up. A worktree
        // whose dir is already gone reads as clean and falls through to the
        // `cleanup.worktree_missing` audit as before.
        if !worktree_is_clean(Some(worktree_path), git) {
            record_branch_preserved(
                paths,
                n,
                n.branch.as_deref(),
                worktree_path,
                "uncommitted changes in worktree (no explicit merge)",
            );
            return;
        }
    }

    // `--force` so any residual untracked/modified scratch left in the worktree
    // does not refuse removal and orphan the worktree+branch (issue
    // `supervisor-worktree-remove-no-force`). On the reached-here paths the branch
    // is either merged (explicit merge, which skips the guards above) or the tree
    // was verified CLEAN and has no unmerged work vs its source (the dirty-worktree
    // + source-relative guards preserved every other case), so anything still in
    // the tree is throwaway. If removal still fails AND the dir is simply gone (user
    // removed it manually), record a non-fatal `cleanup.worktree_missing` and
    // continue.
    if !remove_worktree(repo, worktree_path, git) && !std::path::Path::new(worktree_path).exists() {
        record_worktree_missing(paths, n, worktree_path);
    }
    if let Some(branch) = n.branch.as_deref() {
        let _ = delete_branch(paths, n, repo, branch, git, merged);
    }
}

/// The outcome of the source-relative unmerged-work check that gates a
/// non-explicit-merge teardown ([`branch_unmerged_vs_source`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnmergedCheck {
    /// `branch` has at least one commit not reachable from `source` — committed
    /// work a force teardown would strand. Preserve the branch + worktree.
    HasUnmerged,
    /// The rev-list count could not be computed (a git error / non-zero exit /
    /// unparseable output). For teardown this FAILS CLOSED (issue
    /// `non-merge-teardown-dirty-worktree`): preserve, exactly like `HasUnmerged`,
    /// rather than risk discarding committed work on a transient git failure.
    Unverifiable,
    /// `branch` is provably level with / behind `source` — nothing committed to
    /// lose. Teardown may proceed (still subject to the dirty-worktree guard).
    NoUnmerged,
}

/// The source-relative "is there unmerged work?" check the teardown gate uses
/// instead of `git branch -d`'s ambient-`HEAD`-relative one (issue
/// `blocked-report-deletes-branch`): `git -C <repo> rev-list --count
/// <source>..<branch>`.
///
/// Both a positive count AND an unverifiable result ([`UnmergedCheck::Unverifiable`])
/// preserve — the check is fail-closed for teardown (issue
/// `non-merge-teardown-dirty-worktree`). The earlier form returned a bare `false`
/// on a git error and proceeded to remove the worktree (leaning on the
/// [`delete_branch`] `-d` fallback to keep the branch), but that still discarded a
/// live tree on a transient git hiccup. Only a *confirmed* zero count green-lights
/// teardown.
fn branch_unmerged_vs_source(repo: &str, source: &str, branch: &str, git: &str) -> UnmergedCheck {
    match Git::with_bin(git).rev_list_count(repo, source, branch) {
        Some(count) if count > 0 => UnmergedCheck::HasUnmerged,
        Some(_) => UnmergedCheck::NoUnmerged,
        None => UnmergedCheck::Unverifiable,
    }
}

/// A machine-readable recoverability signal for a dead agent's stranded work
/// (issue `agent-death-strands-recoverable-work`). Computed when the supervisor
/// synthesizes an `agent-died` FAILED `node.report`: the agent's process exited,
/// but its branch may hold complete, mergeable commits ahead of the run's source
/// that were never merged. Stamped into the failed report under the
/// `recoverable_work` key so `run show` / `run wait` can surface "N unmerged
/// commits recoverable on `<branch>`" instead of a bare failure — a caller can
/// then salvage without hand-rolling `git log <source>..<branch>`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Recoverability {
    /// Commits on the branch not reachable from the run's source branch
    /// (`git rev-list --count <source>..<branch>`). Always `> 0` — a signal is
    /// only produced when the branch carries unmerged work, so a genuine
    /// empty-handed death emits no `recoverable_work` block at all.
    pub unmerged_commits: u64,
    /// Whether the branch merges into source without conflict (a trivial
    /// fast-forward when source has not advanced, else a `git merge-tree` probe).
    /// Conservatively `false` on any git error, so a transient hiccup never
    /// green-lights an unclean salvage.
    pub merges_cleanly: bool,
    /// The preserved branch carrying the stranded commits.
    pub branch: String,
    /// The node's worktree path, if still recorded (the operator's salvage
    /// starting point). `None` once the worktree has been forgotten.
    pub worktree_path: Option<String>,
}

impl Recoverability {
    /// `true` when the stranded work is cleanly salvageable — unmerged commits
    /// exist AND they merge into source without conflict. The `unmerged_commits`
    /// invariant (`> 0`) means this collapses to `merges_cleanly`, but the field
    /// is spelled out for the wire so a consumer never has to re-derive it.
    #[must_use]
    pub fn recoverable(&self) -> bool {
        self.unmerged_commits > 0 && self.merges_cleanly
    }

    /// The `recoverable_work` JSON block stamped into a synthesized failed
    /// report (and surfaced verbatim by `run show` / `run wait`).
    #[must_use]
    pub fn to_report_value(&self) -> serde_json::Value {
        serde_json::json!({
            "recoverable": self.recoverable(),
            "unmerged_commits": self.unmerged_commits,
            "merges_cleanly": self.merges_cleanly,
            "branch": self.branch,
            "worktree_path": self.worktree_path,
        })
    }
}

/// Compute the [`Recoverability`] signal for a dead node, or `None` when there
/// is nothing to recover (or nothing can be proven).
///
/// Returns `Some` **only** when the branch has at least one commit ahead of the
/// run's source branch (`git rev-list --count <source>..<branch> > 0`). A branch
/// level with source (a genuine empty-handed death) or any missing input
/// (`source_branch`, `branch`, a repo to probe in) yields `None`, so the
/// failed-report envelope is byte-for-byte unchanged in those cases — no
/// spurious `recoverable_work` block.
///
/// This answers "unmerged but salvageable?" — it requires the branch to be AHEAD
/// of source (has commits source lacks), the stranded-work case the residual
/// crash backstop stamps into its failed report. (The old reconcile-to-SUCCESS
/// gate that answered "already merged?" was deleted with the git-reconcile
/// heuristic in the A6 thin-supervisor cut — merge is now the only success
/// truth, and A2 merge recovery covers the crash-during-merge case.)
///
/// KNOWN LIMITATION: `rev-list --count` is topological reachability, not a
/// content diff. If the branch's changes already landed in source via a SQUASH
/// or CHERRY-PICK (whose report was then lost), its commits are still unreachable
/// from source, so the count is `> 0` and this reports "recoverable". That is not
/// a regression — such a run reads as a bare `failed` today, so a `failed +
/// recoverable` verdict is strictly more informative, and re-merging
/// already-landed squashed work is a clean no-op. The signal is advisory; the
/// operator reviews before merging.
pub fn node_recoverability(paths: &RunPaths, n: &Node, git: &str) -> Option<Recoverability> {
    let branch = n.branch.as_deref().filter(|s| !s.is_empty())?;
    let manifest = read_manifest_opt(paths).ok().flatten()?;
    let source = manifest
        .source_branch
        .as_deref()
        .filter(|s| !s.is_empty())?;
    // Prefer the recorded source repo (survives worktree removal); fall back to
    // the node's own worktree while it still exists. Both refs resolve there.
    let repo = manifest
        .source_repo
        .as_deref()
        .filter(|s| !s.is_empty())
        .or(n.worktree_path.as_deref())?;

    // Only stranded work produces a signal: a branch level with (or behind)
    // source has nothing unmerged to recover. A git error (`None`) declines
    // rather than fabricate a signal.
    let unmerged_commits = match git_ahead_count(repo, source, branch, git) {
        Some(ahead) if ahead > 0 => ahead,
        _ => return None,
    };

    let merges_cleanly = branch_merges_cleanly(repo, source, branch, git);

    Some(Recoverability {
        unmerged_commits,
        merges_cleanly,
        branch: branch.to_string(),
        worktree_path: n
            .worktree_path
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(str::to_string),
    })
}

/// Whether a dead node is **positively empty-handed** — its branch carries ZERO
/// commits ahead of the run's source branch (`git rev-list --count
/// <source>..<branch> == 0`) AND its worktree holds no uncommitted work. This is
/// the precondition for the bounded auto-retry (issue `autoretry-agent-died-worker`):
/// only a worker that produced NOTHING recoverable may be re-spawned from a clean
/// worktree at source, because a retry starts from base and the stale worktree is
/// torn down — so anything it left must be provably disposable.
///
/// Two conditions, BOTH required:
/// 1. **No commits ahead of source** (`git rev-list --count source..branch == 0`) —
///    the dual of [`node_recoverability`] (which fires only when AHEAD `> 0`). The
///    two are mutually exclusive, so a death routes to exactly one of retry
///    (empty-handed) or salvage (committed work).
/// 2. **Clean worktree** ([`worktree_is_clean`]) — no staged, modified, or untracked
///    files. A dead agent frequently leaves uncommitted scratch; requiring a clean
///    tree means the retry's teardown never force-removes work a human might want.
///    A death with a dirty tree is NOT retried — it falls through to the terminal
///    `agent-died` report, whose blocked-handoff gate PRESERVES the worktree.
///
/// Conservative on EVERY uncertainty: a git error, an unparseable count, an
/// unreadable worktree status, or any missing input (`source_branch`, `branch`, a
/// repo to probe in) yields `false` — never retry unless empty-handedness is
/// positively proven. This is the retry ⟂ salvage safety: a transient git hiccup
/// declines to fabricate a "safe to discard" verdict, so a branch/worktree that
/// might hold work is never rebuilt from base.
pub fn node_is_empty_handed(paths: &RunPaths, n: &Node, git: &str) -> bool {
    let Some(branch) = n.branch.as_deref().filter(|s| !s.is_empty()) else {
        return false;
    };
    let Some(manifest) = read_manifest_opt(paths).ok().flatten() else {
        return false;
    };
    let Some(source) = manifest.source_branch.as_deref().filter(|s| !s.is_empty()) else {
        return false;
    };
    let repo = match manifest
        .source_repo
        .as_deref()
        .filter(|s| !s.is_empty())
        .or(n.worktree_path.as_deref())
    {
        Some(r) => r,
        None => return false,
    };
    matches!(git_ahead_count(repo, source, branch, git), Some(0))
        && worktree_is_clean(n.worktree_path.as_deref(), git)
}

/// `git -C <repo> rev-list --count <base>..<branch>` → the number of commits
/// reachable from `branch` but not from `base` (how far the branch advanced
/// forward past its fork point). `None` on a git error / unparseable output, so
/// the caller declines rather than guess.
fn git_ahead_count(repo: &str, base: &str, branch: &str, git: &str) -> Option<u64> {
    Git::with_bin(git).rev_list_count(repo, base, branch)
}

/// Whether `branch` merges into `source` without conflict, WITHOUT mutating any
/// working tree.
///
/// Two rungs, cheapest first:
///
/// 1. **Fast-forward:** if `source` is an ancestor of `branch` the merge is a
///    trivial fast-forward — always clean, one cheap `merge-base` call, and the
///    common case (source untouched since the agent forked).
/// 2. **Three-way probe:** otherwise `git merge-tree --write-tree <source>
///    <branch>` performs the merge in the object store only (no worktree touched)
///    and exits `0` on a conflict-free merge, `1` on conflicts, `≥2` on a git
///    error (verified against git 2.50).
///
/// Conservative throughout: any spawn failure or non-zero exit reads as "does
/// not merge cleanly", so a transient git error never over-reports recoverability.
///
/// Rung 2 requires **git ≥ 2.38** (`--write-tree`, Oct 2022). On an older git the
/// probe fails and reads as "not clean" — but rung 1 still covers the common case
/// (source untouched since the fork), so only a source-that-advanced-into-a-
/// three-way on a pre-2.38 git degrades to a conservative `merges_cleanly: false`.
fn branch_merges_cleanly(repo: &str, source: &str, branch: &str, git: &str) -> bool {
    // (1) Fast-forward: source ⊆ branch ⇒ clean by construction.
    if git_is_ancestor(repo, source, branch, git) {
        return true;
    }
    // (2) In-memory three-way merge. `--write-tree` writes only to the object
    // store (never a worktree); exit 0 = clean, 1 = conflicts, ≥2 = git error.
    Git::with_bin(git).merge_tree_clean(repo, source, branch)
}

/// True when the node's worktree holds no unsaved work — `git -C <worktree>
/// status --porcelain` produces no output (no tracked, staged, or untracked
/// changes). A worktree path that is `None` or no longer exists on disk has
/// nothing to lose and is clean. A worktree that exists but whose `git status`
/// cannot be read is conservatively treated as **dirty** (not clean), so a
/// transient git failure never green-lights tearing a live tree down.
fn worktree_is_clean(worktree_path: Option<&str>, git: &str) -> bool {
    let Some(wt) = worktree_path.filter(|s| !s.is_empty()) else {
        return true;
    };
    if !std::path::Path::new(wt).exists() {
        return true;
    }
    Git::with_bin(git).worktree_is_clean(wt)
}

/// `git -C <repo> merge-base --is-ancestor <ancestor> <descendant>` — true when
/// the command exits 0 (`ancestor` is reachable from `descendant`). A non-zero
/// exit (not an ancestor, or exit 128 for an unknown ref) or a spawn failure →
/// false. Parameters are named by their TOPOLOGICAL role, not domain concepts:
/// the "merged into source?" check passes `(branch, source)` while the
/// "fast-forwards?" check passes `(source, branch)` — the argument ORDER is what
/// distinguishes them, so don't conflate order with the source/branch nouns.
fn git_is_ancestor(repo: &str, ancestor: &str, descendant: &str, git: &str) -> bool {
    Git::with_bin(git).is_ancestor(repo, ancestor, descendant)
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

/// The branch the run was started from (`manifest.source_branch`), if recorded.
/// This is the ref the teardown gate measures "unmerged work" against
/// ([`branch_unmerged_vs_source`]) — the run's actual base, not the main
/// worktree's ambient `HEAD`. `None` for a run created without a recorded base,
/// in which case the source-relative safety net cannot run and teardown falls
/// through to `delete_branch`'s `-d` backstop.
fn manifest_source_branch(paths: &RunPaths) -> Option<String> {
    read_manifest_opt(paths)
        .ok()
        .flatten()
        .and_then(|m| m.source_branch)
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

    let mux = Tmux::with_bin(tmux);
    // Always attempt the kill against the recorded target first.
    if mux.kill_window(socket, &target) {
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
        if let Some(recovered) = mux.find_window_by_path(socket, session, worktree) {
            if recovered != target && mux.kill_window(socket, &recovered) {
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

/// Append a non-fatal `cleanup.branch_preserved` audit event when the supervisor
/// intentionally leaves a node's branch AND worktree in place for the human,
/// instead of tearing them down (issue `blocked-report-deletes-branch`). Fired on
/// two paths: a BLOCKED terminal report (`success: false`, no explicit merge),
/// and the defense-in-depth catch where a non-merge outcome's branch still has
/// commits not reachable from its source. `reason` records which. Unlike a delete
/// *failure* this is the *intended* outcome, so it gets its own event kind and an
/// explicit "left unmerged for you to merge" line on stderr so the work is
/// discoverable instead of silently torn down.
///
/// `branch` is `Option` so a preserved worktree with no recorded branch (a
/// malformed / detached-HEAD node) is still surfaced — the worktree persists on
/// disk regardless, so the human must be told where to look. Idempotent by
/// `(run, node)`.
fn record_branch_preserved(
    paths: &RunPaths,
    n: &Node,
    branch: Option<&str>,
    worktree_path: &str,
    reason: &str,
) {
    let branch_display = branch.unwrap_or("<none>");
    info!(
        target: "orchestratectl::supervise",
        node = %n.node_id,
        branch = branch_display,
        worktree_path,
        reason,
        "preserving branch + worktree for the human (not tearing down)"
    );
    eprintln!(
        "supervisor cleanup: branch {branch_display} left unmerged for you to merge ({reason}; worktree preserved at {worktree_path})"
    );
    let data = json!({
        "node_id": n.node_id.as_str(),
        "branch": branch,
        "worktree_path": worktree_path,
        "reason": reason,
    });
    let key = format!(
        "cleanup.branch_preserved:{}:{}",
        paths.run_id.as_str(),
        n.node_id.as_str()
    );
    if let Err(e) = append_and_apply_event(
        paths,
        "cleanup.branch_preserved",
        Some(&n.node_id),
        Some(&key),
        data,
    ) {
        warn!(
            target: "orchestratectl::supervise",
            node = %n.node_id,
            error = %e,
            "failed to append cleanup.branch_preserved (continuing)"
        );
    }
}

/// Resolve the main worktree path for a linked worktree by reading the FIRST
/// `worktree <path>` line of `git -C <worktree_path> worktree list --porcelain`
/// (git always lists the main worktree first). `None` if git is unavailable, the
/// path is no longer a worktree, or the output is unparseable.
fn main_worktree_of(worktree_path: &str, git: &str) -> Option<String> {
    Git::with_bin(git).main_worktree(worktree_path)
}

/// `git -C <repo> worktree remove --force <worktree_path>` — lenient. `--force`
/// because on the paths that reach it the branch is either merged (explicit
/// merge) or provably has no unmerged work vs its source — a BLOCKED report or a
/// non-merge branch with source-unmerged commits never gets here, its worktree is
/// preserved upstream in [`cleanup_node`] — so any untracked / modified scratch
/// left behind is disposable; without it git refuses to remove a dirty tree and
/// the cascade orphans the worktree AND branch (issue
/// `supervisor-worktree-remove-no-force`). Returns `true` on success so the
/// caller can distinguish an already-gone worktree from a genuine refusal.
fn remove_worktree(repo: &str, worktree_path: &str, git: &str) -> bool {
    Git::with_bin(git).worktree_remove(repo, worktree_path)
}

/// `git -C <repo> branch -{d|D} <branch>` — lenient. The flag is the
/// defense-in-depth safety net against the silent data loss of issue
/// `blocked-report-deletes-branch`:
///
/// - `merged == true` (this node's report carries `via: "explicit-merge"`) →
///   `-D`, force-delete. The merge is confirmed, and the branch may already be
///   removed from the main worktree's vantage point, so the force is safe and
///   necessary.
/// - `merged == false` → `-d`, the LAST-resort backstop. On this arm
///   [`cleanup_node`] has already preserved a branch with source-unmerged commits
///   (its stronger, source-relative [`branch_unmerged_vs_source`] check), so a
///   branch reaching here is expected to be clean. `-d` still refuses a branch not
///   merged into `HEAD`/upstream, catching the residual case where the source
///   check could not run (no `manifest.source_branch`) — it keeps such commits
///   rather than force-dropping them. Note `-d`'s check is ambient-`HEAD`-relative
///   and weaker than the source check, which is why it is only the fallback.
///
/// The branch name is passed after `--` so a name beginning with `-` can never be
/// misparsed as a flag. Either way, if git refuses (unmerged commits, or the
/// branch simply does not exist), record a non-fatal `cleanup.branch_remove_failed`
/// audit event and continue — branch-cleanup failures must never block run
/// completion, and the recorded stderr shows the operator a preserved branch.
///
/// Returns the captured failure detail (`Some`) when the delete did not succeed,
/// `None` on success, so a caller (`run merge`'s synchronous teardown) can also
/// surface the incompletion as an envelope warning in addition to the audit event.
fn delete_branch(
    paths: &RunPaths,
    n: &Node,
    repo: &str,
    branch: &str,
    git: &str,
    merged: bool,
) -> Option<String> {
    // `merged` selects the force flag: a confirmed `run merge` (`via:
    // "explicit-merge"`) force-deletes with `-D`; every other path uses the
    // unmerged-safe `-d`. See [`Git::branch_delete`] for the full safety rationale.
    match Git::with_bin(git).branch_delete(repo, branch, merged) {
        Some(detail) => {
            record_branch_remove_failed(paths, n, branch, &detail);
            Some(detail)
        }
        None => None,
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
    /// untracked scratch file left in the worktree must NOT block teardown on a
    /// CONFIRMED explicit merge. With `--force` the worktree dir AND its branch are
    /// both removed; without it git refused and the cascade orphaned both. Drives
    /// real git end-to-end.
    ///
    /// The scratch here is exercised on the MERGE path deliberately: on a
    /// non-explicit-merge teardown a dirty tree is now PRESERVED, not force-removed
    /// (issue `non-merge-teardown-dirty-worktree` — see
    /// [`dirty_worktree_preserved_on_plain_success`]), so `--force`'s throwaway-
    /// scratch removal only applies where a `run merge` confirmed the work landed.
    #[test]
    fn worktree_with_untracked_file_is_force_removed_with_branch() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 0);
        let (repo, wt) = init_repo_with_worktree(&tmp);
        // The exact orphan trigger from the issue: a stray untracked file.
        std::fs::write(wt.join(".report.json"), "scratch").unwrap();

        let _ = forge_node(
            &paths,
            "n-0001",
            json!({ "worktree_path": wt.to_str().unwrap(), "branch": "wt/foo" }),
        );
        // A confirmed explicit `run merge` authorizes the force teardown.
        report(
            &paths,
            "n-0001",
            json!({ "success": true, "via": "explicit-merge" }),
        );
        let n = read_node_opt(&paths, &nid("n-0001")).unwrap().unwrap();
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

    /// Commit `file` with `msg` on whatever branch the worktree currently has
    /// checked out, so a test can put real (unmerged) work on a `wt/*` branch
    /// before teardown.
    fn commit_in_worktree(wt: &std::path::Path, file: &str, msg: &str) {
        std::fs::write(wt.join(file), "work").unwrap();
        git(wt, &["add", "-A"]);
        git(wt, &["commit", "-qm", msg]);
    }

    /// Count commits on `branch` not reachable from `base` in `repo`
    /// (`git rev-list --count <base>..<branch>`).
    fn commits_ahead(repo: &std::path::Path, base: &str, branch: &str) -> usize {
        let out = Command::new("git")
            .current_dir(repo)
            .args(["rev-list", "--count", &format!("{base}..{branch}")])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
    }

    /// THE data-loss regression (`blocked-report-deletes-branch`): a node whose
    /// terminal report is a BLOCKED handoff (`success: false`, no explicit merge)
    /// has committed, unmerged work. Teardown must PRESERVE both the branch and
    /// the worktree so the human can pick the work up — deleting them is silent
    /// data loss. A `cleanup.branch_preserved` audit event records the handoff.
    #[test]
    fn blocked_report_preserves_branch_and_worktree() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 0);
        let (repo, wt) = init_repo_with_worktree(&tmp);
        // The agent committed real work on wt/foo and never merged it.
        commit_in_worktree(&wt, "fix.rs", "agent work");
        assert_eq!(commits_ahead(&repo, "main", "wt/foo"), 1);

        let _ = forge_node(
            &paths,
            "n-0001",
            json!({ "worktree_path": wt.to_str().unwrap(), "branch": "wt/foo" }),
        );
        // The BLOCKED terminal report: success:false, plain node report (no
        // `via: explicit-merge`).
        report(
            &paths,
            "n-0001",
            json!({ "success": false, "discussion_items": [{ "q": "need sudo" }] }),
        );
        let n = read_node_opt(&paths, &nid("n-0001")).unwrap().unwrap();

        cleanup_node(&paths, &n, "/usr/bin/true", &git_bin());

        assert!(
            branch_exists(&repo, "wt/foo"),
            "blocked path must leave the branch for the human"
        );
        assert!(wt.exists(), "blocked path must preserve the worktree too");
        assert_eq!(
            commits_ahead(&repo, "main", "wt/foo"),
            1,
            "the agent's commit must survive on the branch"
        );
        let evs = events_of_kind(&paths, "cleanup.branch_preserved");
        assert_eq!(evs.len(), 1, "the preserved branch must be recorded once");
        assert_eq!(evs[0]["data"]["branch"], "wt/foo");
    }

    /// The blocked-preserve is idempotent: a second cleanup pass (supervisor
    /// restart) reuses the `(run, node)` key and appends no duplicate event, and
    /// still leaves the branch + worktree intact.
    #[test]
    fn blocked_preserve_event_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 0);
        let (repo, wt) = init_repo_with_worktree(&tmp);
        commit_in_worktree(&wt, "fix.rs", "agent work");
        let _ = forge_node(
            &paths,
            "n-0001",
            json!({ "worktree_path": wt.to_str().unwrap(), "branch": "wt/foo" }),
        );
        report(&paths, "n-0001", json!({ "success": false }));
        let n = read_node_opt(&paths, &nid("n-0001")).unwrap().unwrap();

        cleanup_node(&paths, &n, "/usr/bin/true", &git_bin());
        cleanup_node(&paths, &n, "/usr/bin/true", &git_bin());

        assert!(branch_exists(&repo, "wt/foo"));
        assert!(wt.exists());
        assert_eq!(events_of_kind(&paths, "cleanup.branch_preserved").len(), 1);
    }

    /// No regression on the SUCCESS/merge path: a node whose report carries
    /// `via: "explicit-merge"` is force-torn-down exactly as before — worktree
    /// removed, branch `-D`'d — even though (as after a real squash-merge) the
    /// branch still shows commits ahead of the source. The confirmed merge earns
    /// the force delete.
    #[test]
    fn explicit_merge_report_still_deletes_branch() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 0);
        let (repo, wt) = init_repo_with_worktree(&tmp);
        // Commit ahead of main and do NOT merge into main — mirrors a squash /
        // rebase merge where `-d` would refuse but the merge really happened.
        commit_in_worktree(&wt, "feat.rs", "merged work");
        assert_eq!(commits_ahead(&repo, "main", "wt/foo"), 1);

        let _ = forge_node(
            &paths,
            "n-0001",
            json!({ "worktree_path": wt.to_str().unwrap(), "branch": "wt/foo" }),
        );
        report(
            &paths,
            "n-0001",
            json!({ "success": true, "via": "explicit-merge" }),
        );
        let n = read_node_opt(&paths, &nid("n-0001")).unwrap().unwrap();

        cleanup_node(&paths, &n, "/usr/bin/true", &git_bin());

        assert!(!wt.exists(), "merge path must remove the worktree");
        assert!(
            !branch_exists(&repo, "wt/foo"),
            "an explicit-merge node's branch is force-deleted as before"
        );
        assert!(events_of_kind(&paths, "cleanup.branch_preserved").is_empty());
    }

    /// The durable agent-pane capture (`<run-dir>/agent.log`, issue
    /// `worker-process-hang`) must survive teardown — its whole purpose is
    /// post-mortem readability after the tmux window + worktree are gone. Drives
    /// the most destructive path (a confirmed explicit-merge, which force-removes
    /// the worktree and `-D`'s the branch) and asserts the run-dir log is
    /// untouched. A future change that adds run-dir cleanup to `cleanup_node`
    /// would break this.
    #[test]
    fn agent_log_survives_teardown() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 0);
        let (repo, wt) = init_repo_with_worktree(&tmp);
        commit_in_worktree(&wt, "feat.rs", "merged work");

        // The capture the supervisor would have armed, with real pane output.
        std::fs::write(paths.agent_log(), b"agent stdout line\napi error trace\n").unwrap();

        let _ = forge_node(
            &paths,
            "n-0001",
            json!({ "worktree_path": wt.to_str().unwrap(), "branch": "wt/foo" }),
        );
        report(
            &paths,
            "n-0001",
            json!({ "success": true, "via": "explicit-merge" }),
        );
        let n = read_node_opt(&paths, &nid("n-0001")).unwrap().unwrap();

        cleanup_node(&paths, &n, "/usr/bin/true", &git_bin());

        // Worktree + branch are torn down, but the run-dir capture persists.
        assert!(!wt.exists(), "merge path must remove the worktree");
        assert!(!branch_exists(&repo, "wt/foo"));
        assert!(
            paths.agent_log().exists(),
            "agent.log must survive teardown"
        );
        assert_eq!(
            std::fs::read_to_string(paths.agent_log()).unwrap(),
            "agent stdout line\napi error trace\n",
            "agent.log content must be intact"
        );
    }

    /// Defense-in-depth, `-d` FALLBACK arm (no recorded `source_branch`): a
    /// NON-merge report the primary blocked gate misses (here a bare
    /// `success: true` with no `via`) with no `manifest.source_branch` to run the
    /// stronger source-relative check against. The worktree is removed, but
    /// `git branch -d` refuses to force-drop the unmerged commits — the branch
    /// survives and the refusal is recorded as `cleanup.branch_remove_failed`.
    /// (When `source_branch` IS known, both are preserved — see
    /// [`unmerged_branch_preserves_both_when_source_known`].)
    #[test]
    fn unmerged_branch_not_force_deleted_without_explicit_merge() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 0); // no source_branch → source-relative check can't run
        let (repo, wt) = init_repo_with_worktree(&tmp);
        commit_in_worktree(&wt, "x.rs", "unmerged work");
        assert_eq!(commits_ahead(&repo, "main", "wt/foo"), 1);

        let _ = forge_node(
            &paths,
            "n-0001",
            json!({ "worktree_path": wt.to_str().unwrap(), "branch": "wt/foo" }),
        );
        // success:true but NO `via: explicit-merge` — not blocked, not merged.
        report(&paths, "n-0001", json!({ "success": true }));
        let n = read_node_opt(&paths, &nid("n-0001")).unwrap().unwrap();

        cleanup_node(&paths, &n, "/usr/bin/true", &git_bin());

        assert!(
            branch_exists(&repo, "wt/foo"),
            "unmerged commits must not be force-dropped without a confirmed merge"
        );
        assert_eq!(commits_ahead(&repo, "main", "wt/foo"), 1);
        let evs = events_of_kind(&paths, "cleanup.branch_remove_failed");
        assert_eq!(evs.len(), 1, "the refused delete must be recorded");
        assert_eq!(evs[0]["data"]["branch"], "wt/foo");
    }

    /// Defense-in-depth, PRIMARY arm (`source_branch` known): the same non-merge
    /// miss (a `success: true` with no `via`) but with `manifest.source_branch`
    /// recorded. The source-relative check (`rev-list main..wt/foo` > 0) fires
    /// FIRST, so BOTH the worktree and the branch are preserved — the ambient-HEAD
    /// `git branch -d` weakness never gets a chance to matter, and the worktree is
    /// not destroyed on a gating miss. Recorded as `cleanup.branch_preserved`.
    #[test]
    fn unmerged_branch_preserves_both_when_source_known() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        // Record the run's source branch on the manifest.
        append_and_apply_event(
            &paths,
            "run.created",
            None,
            None,
            json!({
                "kind": "spinoff",
                "lifecycle": "autonomous",
                "title": "t",
                "source_branch": "main",
            }),
        )
        .unwrap();
        let (repo, wt) = init_repo_with_worktree(&tmp);
        commit_in_worktree(&wt, "x.rs", "unmerged work");
        assert_eq!(commits_ahead(&repo, "main", "wt/foo"), 1);

        let _ = forge_node(
            &paths,
            "n-0001",
            json!({ "worktree_path": wt.to_str().unwrap(), "branch": "wt/foo" }),
        );
        report(&paths, "n-0001", json!({ "success": true }));
        let n = read_node_opt(&paths, &nid("n-0001")).unwrap().unwrap();

        cleanup_node(&paths, &n, "/usr/bin/true", &git_bin());

        assert!(
            wt.exists(),
            "worktree must be preserved on the source-check arm"
        );
        assert!(branch_exists(&repo, "wt/foo"), "branch must be preserved");
        assert_eq!(commits_ahead(&repo, "main", "wt/foo"), 1);
        let evs = events_of_kind(&paths, "cleanup.branch_preserved");
        assert_eq!(evs.len(), 1, "preservation must be recorded once");
        assert_eq!(evs[0]["data"]["branch"], "wt/foo");
        assert_eq!(
            evs[0]["data"]["reason"], "unmerged commits vs source (no explicit merge)",
            "the reason must distinguish this from a blocked report"
        );
        // No worktree removal was attempted, so no worktree_missing / remove_failed.
        assert!(events_of_kind(&paths, "cleanup.branch_remove_failed").is_empty());
    }

    /// The source check must NOT preserve a branch with nothing unmerged: a
    /// `success: true` non-merge report whose branch is fully merged into the
    /// recorded `source_branch` proceeds to normal teardown (worktree removed,
    /// branch deleted). Proves the source gate is not over-eager.
    #[test]
    fn merged_branch_torn_down_when_source_known() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        append_and_apply_event(
            &paths,
            "run.created",
            None,
            None,
            json!({
                "kind": "spinoff",
                "lifecycle": "autonomous",
                "title": "t",
                "source_branch": "main",
            }),
        )
        .unwrap();
        // wt/foo has NO commits beyond main → nothing unmerged vs source.
        let (repo, wt) = init_repo_with_worktree(&tmp);
        assert_eq!(commits_ahead(&repo, "main", "wt/foo"), 0);

        let _ = forge_node(
            &paths,
            "n-0001",
            json!({ "worktree_path": wt.to_str().unwrap(), "branch": "wt/foo" }),
        );
        report(&paths, "n-0001", json!({ "success": true }));
        let n = read_node_opt(&paths, &nid("n-0001")).unwrap().unwrap();

        cleanup_node(&paths, &n, "/usr/bin/true", &git_bin());

        assert!(!wt.exists(), "a source-merged branch's worktree is removed");
        assert!(
            !branch_exists(&repo, "wt/foo"),
            "a source-merged branch is deleted"
        );
        assert!(events_of_kind(&paths, "cleanup.branch_preserved").is_empty());
    }

    /// Write an executable fake `git` that always exits non-zero, so a test can
    /// drive the conservative-on-error (fail-closed) teardown branches.
    fn failing_git(dir: &std::path::Path) -> String {
        use std::os::unix::fs::PermissionsExt as _;
        let p = dir.join("fake-git.sh");
        std::fs::write(&p, "#!/bin/sh\nexit 3\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p.to_str().unwrap().to_string()
    }

    /// Record the run's source repo + branch on the manifest (`main`) so the
    /// source-relative and dirty-worktree teardown guards have a base to measure
    /// against. Mirrors the inline `run.created` in the source-known tests.
    fn bootstrap_source_main(paths: &RunPaths, repo: &std::path::Path) {
        append_and_apply_event(
            paths,
            "run.created",
            None,
            None,
            json!({
                "kind": "spinoff",
                "lifecycle": "autonomous",
                "title": "t",
                "source_repo": repo.to_str().unwrap(),
                "source_branch": "main",
            }),
        )
        .unwrap();
    }

    /// THE new data-loss regression (`non-merge-teardown-dirty-worktree`): a
    /// non-explicit-merge teardown (here a plain `success: true`) whose branch has
    /// NO committed work ahead of source but whose worktree holds UNCOMMITTED edits
    /// must PRESERVE both the worktree and the branch — `worktree remove --force`
    /// would otherwise silently discard the edits. Recorded as
    /// `cleanup.branch_preserved` with the uncommitted-changes reason.
    #[test]
    fn dirty_worktree_preserved_on_plain_success() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        let (repo, wt) = init_repo_with_worktree(&tmp);
        bootstrap_source_main(&paths, &repo);
        // Nothing committed ahead of source, but an uncommitted (untracked) edit
        // sits in the tree — the exact case the source-relative check misses.
        assert_eq!(commits_ahead(&repo, "main", "wt/foo"), 0);
        std::fs::write(wt.join("scratch.rs"), "half-done edit").unwrap();

        let _ = forge_node(
            &paths,
            "n-0001",
            json!({ "worktree_path": wt.to_str().unwrap(), "branch": "wt/foo" }),
        );
        report(&paths, "n-0001", json!({ "success": true }));
        let n = read_node_opt(&paths, &nid("n-0001")).unwrap().unwrap();

        cleanup_node(&paths, &n, "/usr/bin/true", &git_bin());

        assert!(wt.exists(), "a dirty worktree must be preserved");
        assert!(
            branch_exists(&repo, "wt/foo"),
            "its branch must survive too"
        );
        assert!(
            wt.join("scratch.rs").exists(),
            "the uncommitted edit must not be discarded"
        );
        let evs = events_of_kind(&paths, "cleanup.branch_preserved");
        assert_eq!(evs.len(), 1, "the preservation must be recorded once");
        assert_eq!(
            evs[0]["data"]["reason"],
            "uncommitted changes in worktree (no explicit merge)"
        );
    }

    /// The same dirty-tree preservation on a `run cancel`, and WITHOUT a recorded
    /// `source_branch` — proving the dirty-worktree guard is independent of the
    /// source-relative committed-work check (which cannot run here). A cancel with
    /// an agent mid-edit must not lose the uncommitted work.
    #[test]
    fn dirty_worktree_preserved_on_cancel_without_source() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 0); // no source_branch → committed-work guard can't run
        let (repo, wt) = init_repo_with_worktree(&tmp);
        std::fs::write(wt.join("scratch.rs"), "mid-edit at cancel").unwrap();

        let _ = forge_node(
            &paths,
            "n-0001",
            json!({ "worktree_path": wt.to_str().unwrap(), "branch": "wt/foo" }),
        );
        report(
            &paths,
            "n-0001",
            json!({ "success": false, "cancelled": true, "reason": "cancelled by user" }),
        );
        let n = read_node_opt(&paths, &nid("n-0001")).unwrap().unwrap();

        cleanup_node(&paths, &n, "/usr/bin/true", &git_bin());

        assert!(wt.exists(), "a dirty worktree must survive a cancel");
        assert!(branch_exists(&repo, "wt/foo"));
        assert!(wt.join("scratch.rs").exists());
        let evs = events_of_kind(&paths, "cleanup.branch_preserved");
        assert_eq!(evs.len(), 1);
        assert_eq!(
            evs[0]["data"]["reason"],
            "uncommitted changes in worktree (no explicit merge)"
        );
    }

    /// Fail-closed on a git error in the source-relative unmerged-commit check
    /// (`non-merge-teardown-dirty-worktree`): when `rev-list --count` cannot be
    /// computed, the teardown PRESERVES the worktree + branch rather than proceed
    /// to remove them. A fully-failing git makes `main_worktree` resolution fall
    /// back to the recorded `source_repo` and the count return `None`. The old
    /// behavior treated the error as "nothing unmerged" and removed the worktree.
    #[test]
    fn git_error_in_source_check_preserves_worktree() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        let (repo, wt) = init_repo_with_worktree(&tmp);
        bootstrap_source_main(&paths, &repo);
        let fail_git = failing_git(tmp.path());

        let _ = forge_node(
            &paths,
            "n-0001",
            json!({ "worktree_path": wt.to_str().unwrap(), "branch": "wt/foo" }),
        );
        report(&paths, "n-0001", json!({ "success": true }));
        let n = read_node_opt(&paths, &nid("n-0001")).unwrap().unwrap();

        cleanup_node(&paths, &n, "/usr/bin/true", &fail_git);

        assert!(
            wt.exists(),
            "an unverifiable source check must preserve the worktree"
        );
        assert!(
            branch_exists(&repo, "wt/foo"),
            "its branch must survive too"
        );
        let evs = events_of_kind(&paths, "cleanup.branch_preserved");
        assert_eq!(
            evs.len(),
            1,
            "the fail-closed preservation must be recorded"
        );
        assert_eq!(
            evs[0]["data"]["reason"],
            "unmerged-commit check unavailable (git error; preserving)"
        );
    }

    /// The `UnmergedCheck` classification, asserted directly against real git:
    /// commits ahead → `HasUnmerged`, level → `NoUnmerged`, git error →
    /// `Unverifiable` (the fail-closed bucket).
    #[test]
    fn branch_unmerged_vs_source_classifies() {
        let tmp = TempDir::new().unwrap();
        let (repo, wt) = init_repo_with_worktree(&tmp);
        let repo_s = repo.to_str().unwrap();
        // Level with source → NoUnmerged.
        assert_eq!(
            branch_unmerged_vs_source(repo_s, "main", "wt/foo", &git_bin()),
            UnmergedCheck::NoUnmerged
        );
        // A commit ahead → HasUnmerged.
        commit_in_worktree(&wt, "f.rs", "work");
        assert_eq!(
            branch_unmerged_vs_source(repo_s, "main", "wt/foo", &git_bin()),
            UnmergedCheck::HasUnmerged
        );
        // A failing git → Unverifiable (fail-closed).
        let fail_git = failing_git(tmp.path());
        assert_eq!(
            branch_unmerged_vs_source(repo_s, "main", "wt/foo", &fail_git),
            UnmergedCheck::Unverifiable
        );
    }

    /// The typed outcome table (design §2.6) is what `cleanup_node` reads for its
    /// teardown policy. A `PreserveWork` teardown (blocked handoff or non-merge
    /// failure) is what leaves BOTH branch and worktree in place; a merge is
    /// `Full`; a cancel is `SourceRelative`. Encoded here on real on-disk nodes so
    /// the teardown gate can never silently re-derive from raw signals.
    #[test]
    fn blocked_classification() {
        use crate::supervise::outcome::{Teardown, TerminalOutcome};
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 0);
        let no_report = forge_node(&paths, "n-0001", json!({ "branch": "wt/a" }));
        // No report yet → not terminal (nothing to classify).
        assert_eq!(TerminalOutcome::classify(&no_report), None);

        let cases = [
            (
                json!({ "success": false }),
                Teardown::PreserveWork,
                "plain blocked handoff preserves work",
            ),
            (
                json!({ "success": false, "cancelled": true }),
                Teardown::SourceRelative,
                "a run-cancel is a deliberate teardown, not a blocked handoff",
            ),
            (
                json!({ "success": true, "via": "explicit-merge" }),
                Teardown::Full,
                "an explicit merge earns full teardown",
            ),
            (
                json!({ "success": true }),
                Teardown::SourceRelative,
                "a plain success without run merge is source-relative",
            ),
        ];
        for (i, (report_data, want, why)) in cases.into_iter().enumerate() {
            let node = format!("n-1{i:03}");
            let _ = forge_node(&paths, &node, json!({ "branch": "wt/x" }));
            report(&paths, &node, report_data);
            let n = read_node_opt(&paths, &nid(&node)).unwrap().unwrap();
            let teardown = TerminalOutcome::classify(&n)
                .expect("terminal report classifies")
                .teardown();
            assert_eq!(teardown, want, "{why}");
        }
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

    /// Like [`bootstrap`] but records a managed `--headless` session on the
    /// manifest (the spawn-time marker `cleanup_managed_session` gates on).
    fn bootstrap_headless(paths: &RunPaths, session: &str) {
        append_and_apply_event(
            paths,
            "run.created",
            None,
            None,
            json!({
                "kind": "spinoff",
                "lifecycle": "autonomous",
                "title": "t",
                "managed_tmux_session": session,
            }),
        )
        .unwrap();
    }

    /// The teardown path: a managed session whose only surviving window is the
    /// synthetic bootstrap `zsh` shell is killed, and a `cleanup.session_killed`
    /// audit event is recorded.
    #[test]
    fn managed_session_killed_when_only_default_window_remains() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap_headless(&paths, "headless");
        // list-windows reports a single unattached default shell window.
        let tmux = fake_tmux(
            tmp.path(),
            r#"case "$*" in
                 *list-windows*) printf '0\tzsh\n';;
               esac"#,
        );

        cleanup_managed_session_with(&paths, &tmux);

        let log = tmux_log(tmp.path());
        assert!(
            log.contains("kill-session -t headless"),
            "empty managed session must be killed: {log:?}"
        );
        let evs = events_of_kind(&paths, "cleanup.session_killed");
        assert_eq!(evs.len(), 1, "exactly one session_killed event expected");
        assert_eq!(evs[0]["data"]["session"], "headless");
    }

    /// Safety gate #3: a still-live sibling agent window (non-default name) keeps
    /// the shared session in use, so it is NOT killed and no event is recorded.
    #[test]
    fn managed_session_retained_when_agent_window_remains() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap_headless(&paths, "headless");
        let tmux = fake_tmux(
            tmp.path(),
            r#"case "$*" in
                 *list-windows*) printf '0\tzsh\n0\t🎬 wt/sibling\n';;
               esac"#,
        );

        cleanup_managed_session_with(&paths, &tmux);

        let log = tmux_log(tmp.path());
        assert!(
            !log.contains("kill-session"),
            "a live sibling agent window must keep the session alive: {log:?}"
        );
        assert!(events_of_kind(&paths, "cleanup.session_killed").is_empty());
    }

    /// Safety gate #2: a human attached to the session means it is left alone —
    /// killing it would yank their terminal. The skip is recorded as
    /// `cleanup.session_retained`.
    #[test]
    fn managed_session_retained_when_attached() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap_headless(&paths, "headless");
        // session_attached = 1 even though the only window is a default shell.
        let tmux = fake_tmux(
            tmp.path(),
            r#"case "$*" in
                 *list-windows*) printf '1\tzsh\n';;
               esac"#,
        );

        cleanup_managed_session_with(&paths, &tmux);

        let log = tmux_log(tmp.path());
        assert!(
            !log.contains("kill-session"),
            "an attached session must not be killed: {log:?}"
        );
        let evs = events_of_kind(&paths, "cleanup.session_retained");
        assert_eq!(evs.len(), 1, "the skip must be recorded once");
        assert_eq!(evs[0]["data"]["session"], "headless");
    }

    /// Safety gate #1: a foreground run records no managed session, so the
    /// teardown is a complete no-op — tmux is never even consulted (the user's
    /// own session is never a candidate).
    #[test]
    fn foreground_run_never_touches_tmux() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 0); // no managed_tmux_session on the manifest
        let tmux = fake_tmux(tmp.path(), "");

        cleanup_managed_session_with(&paths, &tmux);

        // The fake tmux logs every invocation; a foreground run must produce none.
        assert!(
            std::fs::read_to_string(tmp.path().join("tmux.log")).is_err(),
            "tmux must not be invoked for a foreground run"
        );
        assert!(events_of_kind(&paths, "cleanup.session_killed").is_empty());
        assert!(events_of_kind(&paths, "cleanup.session_retained").is_empty());
    }

    /// An already-gone session (its last window WAS the agent's, no bootstrap
    /// shell survived) makes `list-windows` exit non-zero — a clean no-op, no
    /// kill, no event.
    #[test]
    fn already_gone_session_is_noop() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap_headless(&paths, "headless");
        let tmux = fake_tmux(tmp.path(), r#"case "$*" in *list-windows*) exit 1;; esac"#);

        cleanup_managed_session_with(&paths, &tmux);

        let log = tmux_log(tmp.path());
        assert!(!log.contains("kill-session"), "log={log:?}");
        assert!(events_of_kind(&paths, "cleanup.session_killed").is_empty());
        assert!(events_of_kind(&paths, "cleanup.session_retained").is_empty());
    }

    /// The `session_killed` audit event is idempotent across supervisor restarts:
    /// a second teardown pass reuses the run-scoped key and appends no duplicate.
    #[test]
    fn session_killed_event_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap_headless(&paths, "headless");
        let tmux = fake_tmux(
            tmp.path(),
            r#"case "$*" in *list-windows*) printf '0\tzsh\n';; esac"#,
        );

        cleanup_managed_session_with(&paths, &tmux);
        cleanup_managed_session_with(&paths, &tmux);

        assert_eq!(events_of_kind(&paths, "cleanup.session_killed").len(), 1);
    }

    /// The socket recorded on a node's tmux identity is threaded into the
    /// teardown commands, so a headless session on a non-default tmux server is
    /// still found and killed.
    #[test]
    fn managed_session_socket_threaded_into_tmux() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap_headless(&paths, "headless");
        forge_node(
            &paths,
            "n-0001",
            json!({
                "tmux_session": "headless",
                "tmux_window_id": "@5",
                "tmux_socket": "/tmp/sock-7",
                "worktree_path": "/fake/wt",
            }),
        );
        let tmux = fake_tmux(
            tmp.path(),
            r#"case "$*" in *list-windows*) printf '0\tzsh\n';; esac"#,
        );

        cleanup_managed_session_with(&paths, &tmux);

        let log = tmux_log(tmp.path());
        assert!(
            log.contains("-S /tmp/sock-7 list-windows"),
            "socket must be threaded into list-windows: {log:?}"
        );
        assert!(
            log.contains("-S /tmp/sock-7 kill-session -t headless"),
            "socket must be threaded into kill-session: {log:?}"
        );
    }

    #[test]
    fn synthetic_default_window_classification() {
        for n in ["zsh", "bash", "sh", "fish", "-zsh", " bash "] {
            assert!(is_synthetic_default_window(n), "{n:?} should be default");
        }
        for n in ["🎬 wt/foo", "vim", "claude", "wt/abc"] {
            assert!(!is_synthetic_default_window(n), "{n:?} is a real window");
        }
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

    // --- Git-reconcile of a self-merged branch (issues `false-failed-after-merge`
    // / `supervisor-stuck-pending-after-self-merge`) -----------------------------

    /// `git -C <repo> rev-parse <rev>` → the resolved SHA (test helper).
    fn rev(repo: &std::path::Path, r: &str) -> String {
        let out = Command::new("git")
            .current_dir(repo)
            .args(["rev-parse", r])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// Bootstrap an autonomous spinoff run whose manifest records `source_repo`
    /// + `source_branch` — the two refs the reconcile check measures against.
    fn bootstrap_with_source(paths: &RunPaths, repo: &std::path::Path, source_branch: &str) {
        append_and_apply_event(
            paths,
            "run.created",
            None,
            None,
            json!({
                "kind": "spinoff",
                "lifecycle": "autonomous",
                "title": "t",
                "source_repo": repo.to_str().unwrap(),
                "source_branch": source_branch,
            }),
        )
        .unwrap();
    }

    // --- Recoverability signal for a dead agent's stranded work
    // (issue `agent-death-strands-recoverable-work`) -----------------------------

    /// THE positive case: a dead agent committed real, unmerged work that would
    /// fast-forward into source. `node_recoverability` reports it recoverable,
    /// with the commit count, a clean-merge verdict, and the branch + worktree.
    #[test]
    fn stranded_unmerged_work_is_recoverable() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        let (repo, wt) = init_repo_with_worktree(&tmp);
        let base = rev(&repo, "main");
        bootstrap_with_source(&paths, &repo, "main");
        // Committed but NEVER merged — exactly the stranded incident.
        commit_in_worktree(&wt, "impl.rs", "green implementation");
        assert_eq!(commits_ahead(&repo, "main", "wt/foo"), 1);

        let n = forge_node(
            &paths,
            "n-0001",
            json!({ "worktree_path": wt.to_str().unwrap(), "branch": "wt/foo", "base_sha": base }),
        );

        let r = node_recoverability(&paths, &n, &git_bin())
            .expect("stranded unmerged work must produce a signal");
        assert_eq!(r.unmerged_commits, 1);
        assert!(
            r.merges_cleanly,
            "an untouched source fast-forwards cleanly"
        );
        assert!(r.recoverable());
        assert_eq!(r.branch, "wt/foo");
        assert_eq!(r.worktree_path.as_deref(), wt.to_str());
    }

    /// Recoverable even when source has ADVANCED since the fork, as long as the
    /// branch still merges without conflict (a real three-way merge, not a
    /// fast-forward). Disjoint files → clean.
    #[test]
    fn stranded_work_recoverable_when_source_advanced_but_no_conflict() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        let (repo, wt) = init_repo_with_worktree(&tmp);
        let base = rev(&repo, "main");
        bootstrap_with_source(&paths, &repo, "main");
        // Agent commits its work on wt/foo (touches impl.rs).
        commit_in_worktree(&wt, "impl.rs", "agent work");
        // Meanwhile source advances on a DISJOINT file → no conflict on merge.
        std::fs::write(repo.join("other.rs"), "unrelated").unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-qm", "concurrent source work"]);
        assert_eq!(commits_ahead(&repo, "main", "wt/foo"), 1);

        let n = forge_node(
            &paths,
            "n-0001",
            json!({ "worktree_path": wt.to_str().unwrap(), "branch": "wt/foo", "base_sha": base }),
        );

        let r = node_recoverability(&paths, &n, &git_bin()).expect("unmerged work → signal");
        assert_eq!(r.unmerged_commits, 1);
        assert!(
            r.merges_cleanly,
            "disjoint concurrent edits merge cleanly via three-way merge-tree"
        );
        assert!(r.recoverable());
    }

    /// Unmerged work that CONFLICTS with source is still surfaced (a signal is
    /// produced) but flagged `merges_cleanly: false` / `recoverable: false` — it
    /// must never be auto-salvaged. Both sides edit the same file divergently.
    #[test]
    fn conflicting_unmerged_work_is_flagged_not_recoverable() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        let (repo, wt) = init_repo_with_worktree(&tmp);
        let base = rev(&repo, "main");
        bootstrap_with_source(&paths, &repo, "main");
        // Both branches edit the SAME file (README exists from init) divergently
        // → a modify/modify conflict on merge.
        std::fs::write(wt.join("README"), "agent version\n").unwrap();
        git(&wt, &["add", "-A"]);
        git(&wt, &["commit", "-qm", "agent edits README"]);
        std::fs::write(repo.join("README"), "source version\n").unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-qm", "source edits README"]);
        assert_eq!(commits_ahead(&repo, "main", "wt/foo"), 1);

        let n = forge_node(
            &paths,
            "n-0001",
            json!({ "worktree_path": wt.to_str().unwrap(), "branch": "wt/foo", "base_sha": base }),
        );

        let r = node_recoverability(&paths, &n, &git_bin())
            .expect("conflicting-but-unmerged work is still surfaced");
        assert_eq!(r.unmerged_commits, 1);
        assert!(!r.merges_cleanly, "divergent edits to one file conflict");
        assert!(
            !r.recoverable(),
            "a conflicting branch is not auto-recoverable"
        );
    }

    /// A genuine empty-handed death — the branch never advanced past its fork
    /// point — yields NO signal. The failed-report envelope is unchanged: no
    /// spurious `recoverable_work` block claiming there is something to salvage.
    #[test]
    fn empty_handed_death_produces_no_signal() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        let (repo, wt) = init_repo_with_worktree(&tmp);
        let base = rev(&repo, "main");
        bootstrap_with_source(&paths, &repo, "main");
        // wt/foo == main: the agent committed nothing before dying.
        assert_eq!(commits_ahead(&repo, "main", "wt/foo"), 0);

        let n = forge_node(
            &paths,
            "n-0001",
            json!({ "worktree_path": wt.to_str().unwrap(), "branch": "wt/foo", "base_sha": base }),
        );

        assert!(
            node_recoverability(&paths, &n, &git_bin()).is_none(),
            "no unmerged commits → no recoverability signal"
        );
    }

    /// Without a recorded `source_branch` the helper cannot measure "ahead of
    /// source" and declines (returns `None`) rather than guess — the same
    /// missing-input conservatism the reconcile check applies.
    #[test]
    fn missing_source_branch_declines_signal() {
        let tmp = TempDir::new().unwrap();
        let paths = fresh_run(&tmp);
        bootstrap(&paths, 0); // no source_repo / source_branch recorded
        let (_repo, wt) = init_repo_with_worktree(&tmp);
        commit_in_worktree(&wt, "impl.rs", "work");

        let n = forge_node(
            &paths,
            "n-0001",
            json!({ "worktree_path": wt.to_str().unwrap(), "branch": "wt/foo" }),
        );

        assert!(
            node_recoverability(&paths, &n, &git_bin()).is_none(),
            "no source_branch → cannot compute ahead-of-source → decline"
        );
    }

    /// The stamped `recoverable_work` block mirrors the struct verbatim, so the
    /// wire shape `run show` / `run wait` surface is pinned.
    #[test]
    fn recoverability_to_report_value_shape() {
        let r = Recoverability {
            unmerged_commits: 2,
            merges_cleanly: true,
            branch: "wt/foo".to_string(),
            worktree_path: Some("/tmp/wt".to_string()),
        };
        assert_eq!(
            r.to_report_value(),
            json!({
                "recoverable": true,
                "unmerged_commits": 2,
                "merges_cleanly": true,
                "branch": "wt/foo",
                "worktree_path": "/tmp/wt",
            })
        );
    }
}
