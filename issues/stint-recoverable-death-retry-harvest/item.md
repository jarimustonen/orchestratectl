---
created: 2026-07-31
updated: 2026-08-01
type: improvement
status: done
priority: normal
related: ['@stint-maintains-execution-dag', '@run-salvage-command', '@agent-death-strands-recoverable-work']
closed: 2026-08-01
---

# Encode recoverable-worker-death → retry-with-harvest into the /stint skill

## Description

Conductor tactic that surfaced live during the 2026-07-31 DAG-driven /stint and is currently only recorded in TODO.md's KEY LEARNING + handoff prose. It is orchestrator BEHAVIOUR, so per the D1 philosophy (`stint-maintains-execution-dag`) it belongs in the skill itself, not scattered in CLAUDE.md.

## What to encode
Add explicit guidance to the /stint skill (Phase 3 Orchestrate, near 'Sync with run wait; verify landing from git' and the 'do not commit a dead worker's work' rule) for the RECOVERABLE-DEATH case:

- When a spawned worker dies (`agent-died`) but `taskfleet run wait`'s `recoverable_work` shows committed, cleanly-merging commits on a **preserved** branch, the conductor must NOT hand-merge that unreviewed work from the orchestrator session.
- Instead **re-spawn a fresh worktree whose brief points at the preserved branch** and instructs it to review → adopt (cherry-pick / re-apply) → complete the green gate + /llm-review → merge. I.e. **retry-with-harvest**, not hand-merge, and not a base-agent swap.
- Worker deaths are TRANSIENT (the retry usually lands); heavy-LLM units legitimately run 54–96 min, so a long run is not a hang.
- Once the harvested work lands via the retry, the superseded preserved branch/worktree is an **orphan** safe to remove — a deliberate, human-overseen cleanup (relates to `run-salvage-command`).

## Where it lives / related
- Primary home: `crates/taskfleet-cli/skills/stint/SKILL.template.md` (Phase 3). Possibly also a shorter note in `worktree-spinoff`'s death-handling section.
- Relates to `agent-death-strands-recoverable-work` (the recoverability SIGNAL that makes harvest possible — landed) and `run-salvage-command` (the intended salvage command; retry-with-harvest is today's manual stand-in until it ships).
- Lane D (workflow/skill) in the execution DAG.
- Bundled-skill change ⇒ redeploy + insta snapshot loop.
