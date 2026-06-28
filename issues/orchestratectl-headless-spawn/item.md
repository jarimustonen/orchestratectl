---
created: 2026-06-28
updated: 2026-06-28
type: feature
status: fixed
priority: normal
closed: 2026-06-28
commits:
- hash: a76ae97
  summary: 'feat(run): --headless / --tmux-session for run create'
---

# Add --headless / --tmux-session flag to run create

## Description

Feature: support spawning a run without exposing its tmux window in the user's foreground session. The user can opt in to a headless spawn that the multiplexer places into a separate "headless" tmux session (or an explicit `--tmux-session <name>` target). The user can attach later with `tmux attach -t headless` to inspect or supervise, but the day-to-day window list stays clean.

Use case:
- The user is running 5-20 spinoffs in parallel for a campaign. Today every spawn creates a visible tmux window in `default:` — the window list becomes unmanageable. With headless spawn, the windows go to `headless:` and the user only attaches when curious.
- /fan-out users (the typical case for many concurrent units) benefit doubly: a fan-out batch of 20 children pollutes the foreground session today.
- Codex / multi-agent workflows where the user runs orchestratectl in the background.

Status of the underlying layers (verified 2026-06-28):

- **tmux**: native support. `tmux new-session -d -s <name>` creates a detached session; windows added to it are not visible from the user's attached session. `tmux attach -t <name>` to reach them.
- **workmux**: has `--parent-session <name>` flag that targets a specific session for the new window. Has `-b, --background` flag that creates without switching, but the window is still in the user's session — not what we want here. `--parent-session "headless"` IS what we want.
- **orchestratectl**: does NOT currently expose either. `spawn::SpawnRequest` has `layout`, `no_hooks`, `keep_tmux_on_error` — no `parent_session` or `headless`.

Fix direction:

1. Add to `octl_cli::run::RunAction::Create` (clap):
   - `--headless` — boolean. If set, spawn the worker's tmux window in a `headless` session (default name; configurable).
   - `--tmux-session <name>` — string. Explicit session name; overrides `--headless`'s default. Mutually compatible with `--headless` but the explicit name wins.

2. Forward to `spawn::SpawnRequest` via a new optional `parent_session: Option<String>`.

3. In `spawn::run_create_sh` (or whatever wraps the create.sh / workmux call): when `parent_session` is set, append `--parent-session <name>` to the workmux invocation. workmux handles the rest — creates the session if it doesn't exist, attaches the window there.

4. SKILL update: the bundled `worktree-spinoff` / `fan-out` / etc. SKILLs should mention the flag in their "When to use" or "Examples" sections (especially fan-out, which is the obvious win). Update the contract-template Phase 1 first, then mirror.

5. Auto-cleanup interaction: the supervisor's `close-tmux-on-terminal` cleanup must still find and close headless windows. The qualified tmux identity (`socket / session / window_id`) is already recorded in the `node.created` event payload — confirm cleanup uses that, not a default-session assumption.

6. Verification:
   - `orchestratectl run create --kind spinoff --headless --title test --task ...` — window appears in `tmux list-windows -t headless`, not `default`. `tmux attach -t headless` shows it.
   - After auto-cleanup, the headless window is gone.
   - With N >= 5 headless spawns in flight, foreground `tmux list-windows -t default` stays unpolluted.

7. Default behavior remains foreground spawn — opt-in only. Existing users see no change.

Severity: nice-to-have but increasingly important now that the cleanup loop works. Without headless, a campaign with many spawns clutters the user's main tmux unnecessarily — even though everything cleans up at the end, mid-flight visibility is overwhelming.

Possibly small follow-up: a `--detached` flag that uses tmux native detached-session features rather than relying on workmux's `--parent-session`. But starting with workmux delegation is simpler.

Related: `workmux-extract-libs` (if/when raine accepts the split) would let us call multiplexer logic directly from Rust without workmux flag forwarding — even cleaner headless implementation. For now, flag forwarding is sufficient.

## Resolution (2026-06-28)

Implemented in `a76ae97`:

- `run create` gains `--headless` (boolean) and `--tmux-session <name>`
  (string). They resolve to `SpawnRequest.parent_session`
  (`crate::run::create::resolve_parent_session`): explicit name wins and
  implies headless; `--headless` alone yields the default `headless`
  session; neither keeps the unchanged foreground spawn. The name is
  validated (non-empty, no whitespace / `:` / `.` tmux separators).
- `spawn::run_create_sh` forwards it to create.sh as
  `--parent-session <name>`.
- SKILL examples added to `worktree-spinoff` and `fan-out`; help-surface
  snapshots updated.

Auto-cleanup needed no change: `supervise::cleanup` already keys window
kills off the recorded qualified tmux identity (`tmux_session` +
`tmux_window_id`), so it closes a headless window wherever it landed.

**Companion create.sh change (homebase).** create.sh had to learn
`--parent-session`: forward it to `workmux add`, pre-create the target
session detached if missing, discover the window in the *target* session
(not the ambient one), and record `tmux_session = TARGET_SESSION` so
cleanup's qualified identity points at the right session. Committed on
homebase branch `create-sh-tmux-identity` (`7acff49`), stacked on the
qualified-identity commits (`3130460`, `2ea0eca`) that issue
`supervisor-tmux-window-identity` already marks done but that are **not
yet deployed to homebase main**.

**Deploy dependency.** Until `create-sh-tmux-identity` lands on homebase
main (the `~/.claude/skills/worktree/scripts/create.sh` symlink target),
`--headless` forwards `--parent-session` to a deployed create.sh that
rejects it as an unknown flag. This is the same pending-deploy state the
already-merged qualified-identity Rust is in (every spawn currently logs
the back-compat warning because create.sh doesn't yet emit the identity
fields). Deploying that one branch unblocks both.

**Live verification** (against the updated create.sh via `OCTL_CREATE_SH`,
real tmux + workmux + agent):

- `--headless`: window landed in `headless:` (`@113`), absent from
  `default:`; node recorded `tmux_session=headless`,
  `tmux_window_id=@113`, full socket. After `run cancel`, the supervisor
  closed it in ~2s via `tmux kill-window -t @113` (+ worktree/branch
  removed).
- `--tmux-session campaign-x`: window in `campaign-x:` (`@115`), absent
  from `default:` and `headless:`; cleanup closed `@115` the same way.
