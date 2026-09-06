---
created: 2026-06-29
updated: 2026-06-29
type: bug
status: fixed
priority: high
closed: 2026-06-29
---

# SKILL docs steer agents to lifecycle for completion polling — real terminal field is manifest.status

## Description

Reported by a deutschpad-session Claude running autonomous worktree orchestration on 2026-06-29 (taskfleet 0.0.1, commit 119a13e). Bug report at `/tmp/taskfleet-bug-report-progress-polling.md`.

The deployed `worktree-spinoff` SKILL's "Following progress" section and `taskfleet-run-overview`'s field model tell agents to **branch on `lifecycle`** with terminal values **`completed | failed | cancelled`**. The CLI does not match either claim:

- `Lifecycle` enum (`crates/taskfleet-core/src/schema.rs`) is `Autonomous | Interactive` — a category, not a progress state. It never transitions to a terminal value.
- `Status` enum is `Pending | Running | Done | Failed | Cancelled` — the actual progress field. Terminal values are `Done | Failed | Cancelled`, NOT `completed`.

Net effect: an agent that follows the documented contract writes a poller that never fires, because the field+value combination it watches for does not exist anywhere in the envelope.

`taskfleet-overview/SKILL.template.md:113` is the only SKILL that gets this right ("`data.manifest.status`, never `lifecycle`, to tell whether work is done"). Every other SKILL contradicts it.

## Affected lines

- `taskfleet-run-overview/SKILL.template.md:65, 79–83, 98, 103, 112–117` — fundamental field-model error; the worst offender (calls `lifecycle` "authoritative" and `status` "do not branch on its text").
- `taskfleet-spawn-spinoff/SKILL.template.md:69, 78` — claims runs start in `lifecycle: pending`.
- `worktree-bugfix/SKILL.template.md:254`, `worktree-make-skill/SKILL.template.md:132`, `worktree-research/SKILL.template.md:135` — speak of `lifecycle: pending` / `lifecycle: completed`.
- `worktree-orchestrated/SKILL.template.md:170, 293`, `fan-out/SKILL.template.md:128–129`, `orchestrate/SKILL.template.md:215, 445` — child-lifecycle wording (these mostly mirror real event kinds, so verify before mass-renaming).

## Fix

1. Rewrite `taskfleet-run-overview/SKILL.template.md`'s field model: `lifecycle = Autonomous | Interactive` (run category), `status = Pending | Running | Done | Failed | Cancelled` (terminal = `Done | Failed | Cancelled`). Branch on `status`; the "do not branch on its text" sentence is wrong — drop it.
2. Update every SKILL that mentions `lifecycle: pending|running|completed|failed|cancelled` to use `status:` with the right values.
3. Re-deploy SKILLs (`taskfleet skill install --force`) and verify `doctor` is clean.
4. Optional follow-on: `taskfleet run wait <id>` as a first-class blocking primitive so agents don't hand-roll pollers — open as separate issue.
