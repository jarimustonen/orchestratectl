---
created: 2026-06-28
updated: 2026-06-28
type: bug
status: fixed
priority: normal
commits:
- hash: 74be081
  summary: add mandatory terminal node report step to 8 autonomous-merge SKILLs
- hash: 1c66dc5
  summary: correct post-report wording; file supervisor-complete-run-on-terminal-report follow-up
closed: 2026-06-28
---

# Autonomous-merge SKILLs do not tell agent to submit terminal node.report

## Description

Symptom: a /worktree-spinoff (or any autonomous-merge worktree spawned via `orchestratectl run create --kind <X>`) finishes its work, merges to main, and then sits idle - the run stays `pending`, the per-run supervisor process keeps polling, and the tmux window does not close. The user sees a "dangling" worktree that looks like it is waiting for their input, when in fact the work is complete.

First observed 2026-06-28 (haukinen) after the skill-bundling-campaign deploy. Run 01kw79n2yv3epts3amfszmv3aa (the supervise-test-teardown-leak fix) merged 4 commits to main, closed its issue with `fixed`, and stopped - but `orchestratectl run show` still shows lifecycle pending, supervisor 72879 still alive 30+ minutes later.

Root cause: the bundled SKILL.md files for `/worktree-spinoff`, `/worktree-code` (and the autonomous siblings: research, make-skill, bugfix, technical-decision, fan-out, orchestrated) instruct the agent to "merge itself back via /worktree-merge". They do NOT instruct the agent to submit a terminal `node report` to orchestratectl. Without that report:

- The supervisor has no signal that the worker is done; it keeps watching.
- `run show` reports `lifecycle: pending` indefinitely.
- The tmux window stays open because nothing tells it to close.
- From the user's view it looks like the agent is stuck or asking for something.

This is a SKILL design gap, not an agent bug - the agent did exactly what the SKILL told it to do.

Fix direction:

1. Verify what verb exists today. `orchestratectl node report --help` is the candidate; if it does not exist, file a separate CLI issue first. From `orchestratectl-overview` the verb is mentioned, so likely it ships - confirm.

2. Add an explicit final step to every autonomous-spawn SKILL.md (8 files: worktree-spinoff, worktree-orchestrated, worktree-research, worktree-make-skill, worktree-bugfix, worktree-technical-decision, fan-out, and worktree-code's autonomous post-merge step) that reads roughly:

   ```
   After `/worktree-merge` succeeds, submit the terminal report:

   orchestratectl node report <node-id> \
     --success true \
     --discuss '[...]' \
     --spinoff-candidates '[...]' \
     --wrap-up '[...]'

   This is mandatory. Without it the supervisor stays alive and the
   run reads `pending` forever.
   ```

   The structure of the report payload should be the same one
   `worktree-orchestrated` already documents (the schema the
   `/orchestrate` driver consumes per the contract template).

3. Confirm what the supervisor does when it receives a final `node.report`:
   - It should mark the node terminal, transition the run lifecycle to `completed`, and exit cleanly itself.
   - It should also kill the tmux window for autonomous kinds (interactive kinds keep it open for the user).
   If the supervisor does not yet do this, a follow-up CLI issue is needed.

4. After the SKILL fix lands, do a real smoke spawn (e.g. another /worktree-spinoff against a small task) and confirm the run reaches `lifecycle: completed`, the supervisor process exits on its own, and the tmux window closes within a few seconds of the merge.

Workaround for already-dangling runs:

```
orchestratectl run cancel <run-id>
```

This synthesizes a terminal `node.report` for the pending node and transitions the run to `cancelled` - cosmetic difference from `completed` but functionally clears the supervisor and unblocks the tmux window. The actual merge already happened, so no work is lost.

Scope: 8 SKILL.md template files + possibly tests that snapshot the SKILL list / catalog. No CLI code change expected unless step 3 reveals the supervisor itself does not react to a terminal `node.report` for autonomous kinds.
