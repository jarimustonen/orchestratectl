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

# Check for uncommitted changes in OUR OWN worktree. This is safe to test
# before taking the merge lock: it is the agent's own source worktree, not the
# shared target, so it is never touched by a concurrent merge.
#
# Capture the status into a variable rather than testing $(...) inline: a
# `git status` FAILURE (e.g. the worktree vanished) inside `[[ -n "$(...)" ]]`
# yields an empty string and would be silently read as "clean" — `set -e` does
# not fire on a substitution nested in a test. Fail loud instead.
if ! SOURCE_STATUS=$(git status --porcelain); then
    echo "Error: could not inspect worktree status" >&2
    exit 1
fi
if [[ -n "$SOURCE_STATUS" ]]; then
    echo "Error: Uncommitted changes in worktree" >&2
    echo "Please commit first using /git-commit" >&2
    exit 1
fi

echo "Worktree status: clean"
echo ""

# Serialize concurrent merge runs against the same repo BEFORE inspecting the
# target worktree — the target-cleanliness check MUST live inside this critical
# section. While another merge holds this lock it is mid-rebase and the target
# worktree is transiently dirty; checking BEFORE the lock let a concurrent merge
# observe that in-flight state and fail with a spurious "uncommitted changes in
# target" (issue concurrent-self-merge-race). Inside the lock the only merge
# touching the target is ours, so a dirty target there is genuine user work that
# must still block.
#
# Without the lock, two parallel rebases would also race on the FF step and one
# would fail. Use the shared common git dir so the lock works from linked
# worktrees too (a linked worktree's .git is a file, not a directory).
GIT_COMMON_DIR=$(git rev-parse --git-common-dir)
LOCK_FILE="$GIT_COMMON_DIR/worktree-merge.lock"
LOCK_TIMEOUT="${MERGE_LOCK_TIMEOUT:-600}"
# Validate the timeout: flock treats a non-numeric / non-positive `-w` as an
# error, which would look like a lock-contention failure and be misreported as
# `merge_in_progress`. Reject a bad value up front with a plain error instead.
if ! [[ "$LOCK_TIMEOUT" =~ ^[0-9]+$ ]] || [[ "$LOCK_TIMEOUT" -lt 1 ]]; then
    echo "Error: MERGE_LOCK_TIMEOUT must be a positive integer (got '$LOCK_TIMEOUT')" >&2
    exit 1
fi
# `>>` (not `>`): open the lock file without truncating it — the advisory lock
# is on the inode and survives truncation, but truncating a file another merge
# is actively using is a needless surprise when inspecting it.
exec 9>>"$LOCK_FILE"
# Exit 75 (EX_TEMPFAIL) is RESERVED for the lock-timeout case below and mapped
# to `merge_in_progress` by `run merge`. `flock`'s own timeout returns 1, so it
# never collides; the `exit 75` here is the sole producer of that status (the
# workmux invocation later normalizes its exit so a downstream 75 can't leak).
if ! flock -w "$LOCK_TIMEOUT" 9; then
    # A concurrent merge into this target held the lock longer than we waited.
    # This is a serialization timeout, NOT a dirty tree: exit 75 so `run merge`
    # surfaces a distinct, retryable `merge_in_progress` error rather than the
    # misleading dirty-target failure this race used to produce.
    echo "Error: another merge is holding the target branch '$TARGET_BRANCH'; could not acquire the merge lock at $LOCK_FILE within ${LOCK_TIMEOUT}s" >&2
    exit 75
fi
echo "Acquired merge lock"
echo ""

# Now that we hold the lock, no COOPERATING merge (one that also takes this lock)
# is touching the target, so the transient mid-rebase dirt of a concurrent
# self-merge can no longer be observed here. A dirty target at this point is
# therefore genuine uncommitted work — whether a human's edit or a non-merge
# writer — and must still block (this is the real safety check, replacing the
# racy pre-lock one that produced the false positive). As above, capture the
# status so a `git status` failure can't be misread as "clean".
if ! TARGET_STATUS=$(git -C "$TARGET_PATH" status --porcelain); then
    echo "Error: could not inspect target worktree status ($TARGET_PATH)" >&2
    exit 1
fi
if [[ -n "$TARGET_STATUS" ]]; then
    echo "Error: Uncommitted changes in target worktree ($TARGET_PATH)" >&2
    echo "Please commit or stash changes in the target before merging" >&2
    exit 1
fi

echo "Target status: clean"
echo ""

# Gather commits
COMMITS=$(git log --oneline "${TARGET_BRANCH}..HEAD")
COMMIT_COUNT=$(echo "$COMMITS" | grep -c . || echo "0")

echo "Commits to merge ($COMMIT_COUNT):"
echo "$COMMITS"
echo ""

echo "Running workmux merge --rebase --into $TARGET_BRANCH..."

# Use --keep so workmux doesn't try (and fail) to kill our own window.
# --into pins the merge target (defaults to config main_branch when omitted).
#
# `9>&-` closes the lock fd for the child: bash fds are not close-on-exec, so
# workmux (and anything it spawns, e.g. a tmux server) would otherwise inherit
# fd 9 and — if a descendant outlived this script — hold the advisory lock
# forever, deadlocking every future merge. We still hold fd 9 in this shell, so
# the critical section is unaffected.
#
# Capture the status instead of letting `set -e` propagate it: a downstream exit
# 75 (from workmux or git) would otherwise leak out as the script's status and
# be misclassified as the lock-timeout `merge_in_progress`. Any workmux failure
# is a genuine merge failure — normalize it to exit 1 (`merge_failed`).
merge_rc=0
workmux merge --rebase --keep --into "$TARGET_BRANCH" 9>&- || merge_rc=$?
if [[ "$merge_rc" -ne 0 ]]; then
    echo "Error: workmux merge failed (exit $merge_rc)" >&2
    exit 1
fi

echo ""
echo "Merge complete!"
echo "  Branch: $BRANCH"
echo "  Target: $TARGET_BRANCH"
echo "  Commits merged: $COMMIT_COUNT"

# Tmux window teardown is now the supervisor's responsibility — it can find
# the window rename-proof via worktree-path (session-scoped, exact match),
# while merge.sh only saw $TMUX_PANE and would target the WRONG window when
# a retry came from a different pane after a manual rebase resolution
# (issue: merge-sh-tmux-pane-recovery). The supervisor's cleanup also
# handles `git worktree remove --force` and `git branch -D`, so merge.sh
# no longer races it on those either — the merge call here advances the
# run to terminal, and the supervisor's next tick tears down everything.
