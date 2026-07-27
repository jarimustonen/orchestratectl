---
created: 2026-07-27
updated: 2026-07-27
type: bug
status: open
priority: normal
related: ['@stint-maintains-execution-dag']
---

# triage-bugs and stint disagree on who sets a fix-now issue to in-progress

## Description

Surfaced by the `/llm-skill-review` cross-skill lens during the
`stint-maintains-execution-dag` work (not caused by it — pre-existing).

`/stint` Phase 1 and `/worktree-spinoff` agree that the **worker** owns the
issue-status lifecycle: it sets `in-progress` on its first commit. `/stint` is explicit —
"Do not set `--status in-progress` here — the spinoff owns the issue lifecycle … Setting
it now races with the worker."

But `/triage-bugs`' own fix-now disposition instruction (in its SKILL body) tells the
**caller** to run `issuectl update <slug> --status in-progress` before spawning. An agent
that follows `/triage-bugs`' text sets `in-progress` too early — the issue then looks like
it has a live worker when none exists yet, and the caller's write can race the worker's
own first-commit status write.

## Impact

Cosmetic-to-moderate: a fix-now issue can show `in-progress` with no worktree, and the
caller/worker double-write to the same issue file can conflict. It does not corrupt the
DAG (which stores no status), but it muddies the "is this being worked?" signal the DAG's
head-of-line computation reads from issuectl.

## Fix

Make the ownership boundary consistent across the two skills — the clean split is:

- `/triage-bugs`: `needs-triage` → `triaged` only (no status mutation on fix-now).
- `/stint`: disposition labels + dependency/lane metadata only (already correct).
- worker: issue status `open` → `in-progress` → `fixed` (already correct).

Concretely: remove the `issuectl update <slug> --status in-progress` step from
`/triage-bugs`' fix-now disposition, and stop its lifecycle prose from claiming the caller
owns the `→ in-progress` transition.

## Notes

- `/triage-bugs` is bundled under `crates/octl-cli/skills/triage-bugs/` — a bundled-skill
  change ⇒ redeploy + insta snapshot loop.
