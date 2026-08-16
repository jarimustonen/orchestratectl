---
created: 2026-08-10
updated: 2026-08-16
type: feature
status: open
priority: high
labels: [defer-0.2.1]
lane: lifecycle
lane_seq: 40
---

# Spinoff blocked on user input at a genuine fork must propagate to the parent agent, not silently block

## Description

## Observed (2026-08-06, `/stint-start` round 3)

A `/worktree-spinoff` (`entirely-faithful-beast`, headless) did its work and ran
`/llm-review`, then discovered the issue's premise was a non-bug and hit a **genuine
decision fork** (merge the modest reviewed slice + spin off the real work, vs. fold the
big API change in now). Instead of resolving it, the agent **stopped at its interactive
prompt and waited ~6 hours** for a human answer that no one was watching for.

- `orchestratectl run wait` reported the run `pending` / `landed: false` the whole time —
  indistinguishable at a glance from a hang or a death.
- The branch had committed, reviewed work; `manifest.updated_at` was frozen near creation;
  the agent process + tmux pane were alive at a `❯` prompt.
- It only resolved because the orchestrator manually read the pane (`tmux capture-pane`)
  and nudged it (`tmux send-keys`) to take one option and call its own `run merge`.

This is a **silent multi-hour stall** of an autonomous run, and it wedged the whole round
until manually noticed. It is distinct from the two existing look-alikes:
`idle-empty-handed-alive-agent-hangs` (ALIVE agent, **0 commits**) and
`agent-skips-run-merge-idle-pending` (agent dropped to an idle shell). Here the agent is
**alive, has committed reviewed work, and is deliberately blocked awaiting user input.**

## Desired behaviour (the real fix — not a workaround)

An autonomous spinoff that genuinely needs a human decision must **not** silently block on
its own stdin. The *need for user input* should become first-class run state that
**propagates (with some delay) up to the parent/orchestrator agent, which can then surface
it to the user.** Sketch:

1. **Child signals "awaiting-user-input" as run state**, not as a blocking stdin read —
   e.g. a durable event/marker (reusing/【extending the `discussion_items` /
   `open_discussions` machinery) carrying the question + options + its recommended default.
2. **The signal propagates to the parent with a delay** (a grace window, so a fork the
   agent resolves itself within a few minutes doesn't page anyone), analogous to the
   `--kind orchestrate` stall hint (`peculiarly-muddled-caption`) but for the
   awaiting-input condition. Consider surfacing it on `run show` / `run list`
   (`awaiting_input: true` / an open-discussion count) and via the `run create --notify`
   hook so the parent session is actively told.
3. **The parent agent surfaces it to the user** (the orchestrator conducting the round
   gets told "run X is waiting on a decision: <question>") instead of having to poll or
   stumble on it 6h later.
4. Meanwhile the child must **not sit blocked on interactive stdin indefinitely** — after
   emitting the awaiting-input signal it should either proceed on a stated best-judgment
   default after a timeout, or submit a **blocked report** (`success:false` +
   `discussion_items`) so the run reaches a terminal/observable state rather than hanging
   `pending` forever.

## Relation to existing issues

- Complements `no-completion-notification-to-parent` and `notify-run-level-summary`
  (parent-notification plumbing) — this adds an *awaiting-input* signal to that channel.
- Sibling to `idle-empty-handed-alive-agent-hangs` + `agent-skips-run-merge-idle-pending`
  but a **third, distinct** alive-agent `pending` shape (committed work + blocked on input).
- The read-time stall-detection pattern from `peculiarly-muddled-caption` is the model for
  the delayed propagation.

## Comments

Filed in place of any handoff "workaround" note: the manual `tmux capture-pane` + nudge
recovery is NOT the intended resolution and is deliberately not being enshrined as
standing guidance — this issue is the fix. Supersedes the previously-considered "spinoff
prompt must forbid interactive blocking" framing (that is point 4 above).

## Decisions

### 2026-08-13T11:10:30Z · @adr-decision-2

DEFER-to-0.2.1: blocked->parent propagation (HIGH) is a missing protocol transition — the self-report plugin makes it trivial. The clean answer is the pi.dev self-report/lease plugin (0.2.1), not the 0.2.0 thin core. Recorded by ADR 0001 (docs/decisions/0001-thin-supervisor-vs-harden.md).
