---
created: 2026-07-13
updated: 2026-08-10
type: improvement
status: done
priority: normal
related: ['@vendor-workmux-multiplexer', '@orchestratectl-headless-spawn', '@spinoff-e2e-harness', '@bundle-worktree-merge']
commits:
- hash: '1538680'
  summary: vendor typed git-worktree wrapper; cleanup.rs routes git through git::repo::Git
- hash: a6bc8ed
  summary: tighten git-wrapper tests per llm-review (assert worktree_remove)
closed: 2026-08-10
---

# workmux-extract-libs: vendor a typed git-worktree wrapper (multiplexer slice already landed)

_Source: workmux (raine/workmux)_

## Status — re-scoped 2026-08-05

The original ask covered TWO slices — a typed **multiplexer** wrapper and a typed
**git-worktree** wrapper — plus a speculative sandbox slice. Reconciled against what
has actually landed:

- **Multiplexer slice — DONE (subsumed).** raine declined the upstream crate split
  (2026-07-13) and suggested vendoring instead; the tmux slice was vendored under
  `crates/octl-cli/src/multiplexer/` (`kill_window` / `kill_session` /
  `new_session(headless)` / `find_window_by_path`) and the supervisor's tmux-cleanup
  path now makes typed `Tmux` calls instead of shelling out. Tracked and closed by
  `@vendor-workmux-multiplexer` (done 2026-07-31, commit `20ec690`). Nothing remains
  here for the multiplexer.
- **Sandbox slice — DROPPED.** No orchestratectl worktree kind uses sandboxing and
  there is no concrete driver; the original "useful for some worktree kinds" line was
  speculative. Out of scope — refile if a real need appears.
- **Git-worktree slice — REMAINING (this issue, narrowed below).**

## Remaining slice — typed git-worktree wrapper

Spawn still shells out through `~/.claude/skills/worktree/scripts/create.sh`
(`crates/octl-cli/src/run/spawn.rs`), which itself calls `workmux add`, and the
supervisor cleanup + merge paths issue ~40 raw `git` subprocesses
(`crates/octl-cli/src/supervise/cleanup.rs`, `run/merge.rs`) — `git worktree remove`,
`git branch -d/-D`, `git rev-list --count`, ancestry checks, etc. There is no typed
git wrapper; git logic is scattered as `Command::new(git)` call sites.

The workmux counterpart is `src/git/` (~50KB: branch + worktree + merge + remote +
status). Following the multiplexer precedent, the route is **vendor the minimum**, not
depend on a crate.

### Why this is NOT blocking

`create.sh` + raw git-shelling is fully functional today. The state-integrity
invariants (branch-preservation gates, source-relative ancestry checks) live in
`cleanup.rs` and work. This is a **cleanliness / coupling** improvement — removing the
`create.sh` stdout-contract parsing and the scattered git subprocess call sites in
favour of typed calls — **not a functional gap**. Low urgency (the schema's priority
enum is `normal|high`, so it stays `normal`, but treat it as backlog cleanup debt).

### Crisp done-definition (IF pursued)

1. A vendored `crates/octl-cli/src/git/` module (fork-and-own, same provenance +
   attribution discipline as `multiplexer/`) exposing the worktree/branch operations
   the supervisor and merge path actually use — `worktree_remove`, `branch_delete`
   (with the `-d` vs `-D` unmerged-safety distinction preserved), `rev_list_count` /
   ancestry check. Vendor only what call sites exist; no speculative surface.
2. `cleanup.rs` + `run/merge.rs` call the typed wrapper instead of `Command::new(git)`.
   **All five state-integrity invariants preserved** (see repo `CLAUDE.md` → "State
   integrity invariants") — especially the branch-preservation gates and the
   source-relative ancestry check; the wrapper must not soften them.
3. The `create.sh`-side git handling (worktree creation via `workmux add`) may stay on
   the shell for now — replacing spawn-side git is a larger, separate step and is NOT
   required for this issue's done. Scope this to the supervisor/merge git call sites.
4. Green gate + `/llm-review` before landing (touches hot correctness files).

## Out of scope

- The multiplexer slice (done — `@vendor-workmux-multiplexer`).
- Sandbox (dropped — no driver).
- `config.rs` (repo-specific) and `command/` (CLI-only) — never in scope.
- Replacing the `workmux add` spawn path itself; keep `create.sh` for creation.

## Open fork for a human call

Whether to build this at all is a genuine judgment call, deferred here rather than
guessed: (a) build the git-wrapper slice now, (b) keep it open at `low` as tracked
cleanliness debt, or (c) close obsolete and accept scattered git-shelling as the
permanent shape. The multiplexer vendoring already delivered the primary value; the
git wrapper is a smaller, non-blocking cleanup whose payoff is coupling reduction only.
