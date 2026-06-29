---
created: 2026-06-28
updated: 2026-06-29
type: bug
status: fixed
priority: high
closed: 2026-06-29
---

# run create --headless crashes create.sh with unknown-flag --parent-session

_Source: src/run/create + create.sh_

## Description

orchestratectl run create --headless (and presumably --tmux-session) forwards `--parent-session <name>` to create.sh, but create.sh rejects it:

```
{"code":"unknown-flag","message":"Unknown flag: --parent-session","expected":"--type, --agent, --layout, --no-hooks, --keep-tmux-on-error, --agent-startup-timeout"}
```

orchestratectl then wraps this as `create_sh_error_create_sh_unparseable` and exits 2, so the spawn fails entirely. Repro: `orchestratectl run create --kind orchestrated --headless ...` during the /orchestrate smoke test (2026-06-28). The --headless/--tmux-session feature (commit 1418fa6) added the flag on the Rust side but create.sh does not accept `--parent-session`. Either teach create.sh the flag or rename to what create.sh expects (`--parent-session` vs the workmux session arg). Found during /orchestrate end-to-end smoke test.

## Fix (2026-06-29)

**Root cause / location.** The issue assumed `create.sh` is bundled in this repo. It is not — `create.sh` is owned by the `homebase` repo
(`dotfiles/src/.claude/skills/worktree/scripts/create.sh`) and symlinked into `~/.claude/skills/worktree/scripts/create.sh`. `orchestratectl skill install` ships the `SKILL.template.md` files only, never `create.sh`. So the real fix landed in homebase; orchestratectl already forwarded the flag correctly.

**homebase `create.sh` changes** (commit in the homebase repo — must be pushed separately):
1. Arg parser now accepts `--parent-session <name>` and `--parent-session=<name>`; added to the `unknown-flag` `expected:` list.
2. Session discovery: when `--parent-session` is set, it overrides the `tmux display-message` auto-discovery (which fails when the parent process runs outside tmux). The named session is created detached if absent (`tmux new-session -d -s "$NAME"`, idempotent) and verified with `tmux has-session`.
3. The flag is forwarded to `workmux add --parent-session <name>` so the new window actually lands in that session (window-mode targeting).
4. A missing-tmux check was hoisted so `--parent-session` fails cleanly with `no-tmux` instead of a confusing later error.

**orchestratectl changes** (this branch): the Rust side (`run/create.rs::resolve_parent_session`, `run/spawn.rs`) already forwarded `--parent-session` for headless/`--tmux-session` spawns and omitted it for foreground — verified. Added two integration tests in `tests/spawn_all_kinds.rs`: `headless_forwards_parent_session_to_create_sh` (argv-recording fixture asserts `--parent-session headless` reaches create.sh) and `foreground_omits_parent_session_flag`.

**Smoke (2026-06-29).** Real `run create --kind spinoff --headless --tmux-session octl-hl-smoke` succeeded — no `unknown-flag`; window placed in `octl-hl-smoke`; `run show` reached `pending`; `run cancel` removed the worktree + branch with no leaked supervisor.

**Follow-up.** After `run cancel` of a headless spawn the tmux *window* lingered in the detached session (worktree + branch were removed). The deployed `create.sh` predates the qualified-identity stdout fields (`tmux_session`/`tmux_window_id`/`tmux_socket`), so the supervisor falls back to bare-name matching and cannot reliably remove a window in a non-current session. Tracked as a spin-off proposal.
