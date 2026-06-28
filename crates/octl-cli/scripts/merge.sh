#!/bin/bash
# merge.sh — merge a worktree branch back to its target branch.
#
# BUNDLED COPY. This script is embedded into the `orchestratectl` binary
# (see crates/octl-cli/src/run/merge.rs) and materialized to a temp file at
# runtime by `orchestratectl run merge`. It is the v1 merge-mechanics
# backend: it owns the rebase, the flock that serializes concurrent merges,
# the workmux merge, and the post-merge teardown of the worktree + tmux
# window + branch. `run merge` wraps it, then submits the terminal
# `node.report` so the supervisor can wind the run down.
#
# It descends from homebase `~/.claude/skills/worktree-merge/scripts/merge.sh`
# (now sunset). Keep the cleanup subshell: it is the proven path that handles
# the lingering-cwd race (a shell still parked inside the worktree) via
# detach + sleep + `--force`. The per-run supervisor also runs a best-effort
# cleanup on the terminal report; the two race and either finishing first
# leaves the other a clean no-op (see supervise/cleanup.rs).
#
# Usage: merge.sh [--target <branch>] [branch-name]
#   If no branch name given, uses the current branch.
#   If no --target given, the target is the main/master worktree.

set -euo pipefail

TARGET_BRANCH=""
POSITIONAL=()

while [[ $# -gt 0 ]]; do
    case "$1" in
        --target)
            TARGET_BRANCH="${2:-}"
            shift 2 || { echo "Error: --target requires a value" >&2; exit 1; }
            ;;
        --target=*)
            TARGET_BRANCH="${1#--target=}"
            shift
            ;;
        *)
            POSITIONAL+=("$1")
            shift
            ;;
    esac
done

# Get branch name (argument or current)
if [[ -n "${POSITIONAL[0]:-}" ]]; then
    BRANCH="${POSITIONAL[0]}"
else
    BRANCH=$(git branch --show-current)
fi

echo "Merge worktree: $BRANCH"

# Check not on main
if [[ "$BRANCH" == "main" ]] || [[ "$BRANCH" == "master" ]]; then
    echo "Error: Cannot merge from main/master branch" >&2
    exit 1
fi

# Resolve the target branch + the worktree it is checked out in.
if [[ -n "$TARGET_BRANCH" ]]; then
    # Explicit target: find the worktree that has this branch checked out.
    TARGET_PATH=$(git worktree list | grep -F "[$TARGET_BRANCH]" | awk '{print $1}' | head -n1)
    if [[ -z "$TARGET_PATH" ]]; then
        echo "Error: target branch '$TARGET_BRANCH' is not checked out in any worktree" >&2
        echo "       /orchestrate keeps its own worktree alive as the merge parent; ensure it still exists." >&2
        exit 1
    fi
else
    # Legacy: detect the main/master worktree.
    TARGET_PATH=$(git worktree list | grep -E '\[main\]|\[master\]' | awk '{print $1}' | head -n1)
    if [[ -z "$TARGET_PATH" ]]; then
        echo "Error: Could not find main worktree" >&2
        exit 1
    fi
    TARGET_BRANCH=$(git -C "$TARGET_PATH" branch --show-current)
fi

if [[ "$BRANCH" == "$TARGET_BRANCH" ]]; then
    echo "Error: refusing to merge '$BRANCH' into itself" >&2
    exit 1
fi

echo "Merge target: $TARGET_BRANCH ($TARGET_PATH)"
echo ""

# Check for uncommitted changes in worktree
if [[ -n "$(git status --porcelain)" ]]; then
    echo "Error: Uncommitted changes in worktree" >&2
    echo "Please commit first using /git-commit" >&2
    exit 1
fi

# Check the target worktree is clean
if [[ -n "$(git -C "$TARGET_PATH" status --porcelain)" ]]; then
    echo "Error: Uncommitted changes in target worktree ($TARGET_PATH)" >&2
    echo "Please commit or stash changes in the target before merging" >&2
    exit 1
fi

echo "Worktree status: clean"
echo "Target status: clean"
echo ""

# Serialize concurrent merge runs against the same repo.
# Without this, two parallel rebases race on the FF step and one fails.
# Use the shared common git dir so the lock works from linked worktrees too
# (a linked worktree's .git is a file, not a directory).
GIT_COMMON_DIR=$(git rev-parse --git-common-dir)
LOCK_FILE="$GIT_COMMON_DIR/worktree-merge.lock"
exec 9>"$LOCK_FILE"
if ! flock -w 600 9; then
    echo "Error: Could not acquire merge lock at $LOCK_FILE within 600s" >&2
    exit 1
fi
echo "Acquired merge lock"
echo ""

# Re-check the target is still clean now that we hold the lock — another merge
# may have just finished and left it in an unexpected state.
if [[ -n "$(git -C "$TARGET_PATH" status --porcelain)" ]]; then
    echo "Error: Target worktree became dirty while waiting for lock" >&2
    exit 1
fi

# Gather commits
COMMITS=$(git log --oneline "${TARGET_BRANCH}..HEAD")
COMMIT_COUNT=$(echo "$COMMITS" | grep -c . || echo "0")

echo "Commits to merge ($COMMIT_COUNT):"
echo "$COMMITS"
echo ""

echo "Running workmux merge --rebase --into $TARGET_BRANCH..."

# Capture current tmux window ID before merge (workmux will try to kill it but
# can't because we're still running inside it)
if [[ -n "${TMUX:-}" ]]; then
    TMUX_WINDOW_ID=$(tmux display-message -t "$TMUX_PANE" -p '#{window_id}')
fi

# Get worktree path before merge for cleanup
WORKTREE_PATH=$(pwd)

# Use --keep so workmux doesn't try (and fail) to kill our own window.
# --into pins the merge target (defaults to config main_branch when omitted).
workmux merge --rebase --keep --into "$TARGET_BRANCH"

echo ""
echo "Merge complete!"
echo "  Branch: $BRANCH"
echo "  Target: $TARGET_BRANCH"
echo "  Commits merged: $COMMIT_COUNT"

# Clean up worktree, branch, and tmux window in background.
# --keep skips all cleanup, so we must do it ourselves.
# `setsid` detaches the cleanup into its own session/pgroup so it survives the
# SIGHUP cascade when its own pane gets killed; `& disown` further keeps it
# alive across merge.sh's exit. </dev/null + stdio redirect frees the pane's
# pty so tmux can reap the window without waiting on our FDs.
if [[ -n "${TMUX_WINDOW_ID:-}" ]]; then
    # Pick the strongest available detacher: setsid > nohup > bare background.
    # macOS ships setsid as `setsid` on some versions; fall back gracefully.
    if command -v setsid >/dev/null 2>&1; then
        DETACH=(setsid)
    elif command -v nohup >/dev/null 2>&1; then
        DETACH=(nohup)
    else
        DETACH=()
    fi
    "${DETACH[@]}" bash -c '
        # Belt-and-braces SIGHUP/SIGTERM ignore — nohup only ignores HUP and
        # does not create a new session/pgroup on macOS, so if tmux ever
        # signals our pane'"'"'s pgroup with TERM during the kill cascade we
        # still survive.
        trap "" HUP TERM
        # Drop any inherited merge lock IMMEDIATELY, before any FS operation
        # that could block (e.g. a slow NFS or contended directory). The lock
        # must not gate the next queued merge on our cleanup.
        exec 9>&- 2>/dev/null || true
        # Step out of the worktree before removing it; otherwise our cwd is
        # invalid for the rest of cleanup, which makes spawning binaries
        # (notably tmux) flake on macOS/Linux.
        cd "$1" 2>/dev/null || cd /
        set +e
        TARGET_PATH="$1"; WORKTREE_PATH="$2"; BRANCH="$3"; TMUX_WINDOW_ID="$4"
        LOG="/tmp/worktree-merge-cleanup-${BRANCH//\//_}-$(date +%s)-$$.log"
        log() { printf "[%s] %s\n" "$(date "+%Y-%m-%dT%H:%M:%S%z")" "$*" >>"$LOG" 2>/dev/null || true; }
        log "subshell start: branch=$BRANCH window=$TMUX_WINDOW_ID worktree=$WORKTREE_PATH pid=$$ sid=$(ps -o sid= -p $$ 2>/dev/null | tr -d " ")"
        sleep 3
        log "sleep done"
        # --force: a lingering shell (e.g. the Claude Code bash session that
        # ran us) may still have its cwd inside the worktree, which makes a
        # plain `git worktree remove` fail and leak the worktree + its tmux
        # window. --force removes the worktree regardless.
        git -C "$TARGET_PATH" worktree remove --force "$WORKTREE_PATH" >>"$LOG" 2>&1
        log "git worktree remove exit=$?"
        git -C "$TARGET_PATH" branch -d "$BRANCH" >>"$LOG" 2>&1
        log "git branch -d exit=$?"
        # Always attempt the kill — kill-window on an already-dead target is
        # harmless (errors to stderr, swallowed). Do NOT precheck with
        # list-windows + grep: a transient empty/stale list makes the precheck
        # silently skip the kill and leak the window.
        for attempt in 1 2 3 4 5; do
            tmux kill-window -t "$TMUX_WINDOW_ID" >>"$LOG" 2>&1
            rc=$?
            log "kill-window attempt=$attempt exit=$rc"
            sleep 0.3
            wins=$(tmux list-windows -a -F "#{window_id}" 2>>"$LOG")
            list_rc=$?
            if [[ $list_rc -ne 0 ]]; then
                log "list-windows failed (exit=$list_rc), cannot verify — retrying"
                sleep 0.5
                continue
            fi
            if ! printf "%s\n" "$wins" | grep -qx "$TMUX_WINDOW_ID"; then
                log "window confirmed gone after attempt=$attempt"
                break
            fi
            log "window still present after attempt=$attempt, retrying"
            sleep 0.5
        done
        # Final state for diagnosability — if the window survived all retries,
        # this log line is the smoking gun for the next recurrence.
        wins=$(tmux list-windows -a -F "#{window_id}" 2>>"$LOG")
        if [[ $? -ne 0 ]]; then
            log "FINAL: list-windows failed, window state unknown for $TMUX_WINDOW_ID"
        elif printf "%s\n" "$wins" | grep -qx "$TMUX_WINDOW_ID"; then
            log "FAILED: window $TMUX_WINDOW_ID still present after all retries"
        fi
        log "cleanup done"
    ' bash "$TARGET_PATH" "$WORKTREE_PATH" "$BRANCH" "$TMUX_WINDOW_ID" </dev/null >/dev/null 2>&1 &
    disown
    echo "  Cleanup will happen in 3 seconds..."
fi
