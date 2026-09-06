---
created: 2026-06-28
updated: 2026-06-28
type: bug
status: fixed
priority: normal
commits:
- hash: 1b9a2c6
  summary: 'fix(events): accept orchestrator.decision + discuss.critical kinds'
closed: 2026-06-28
---

# /orchestrate decision-log and pakkopysäytys event kinds rejected by binary

## Description

Symptom: `/orchestrate` skill §5 (decision logging) and §6 (pakkopysäytys) both tell the orchestrator to call `taskfleet event create --kind <X>` where `<X>` is `orchestrator.decision` or `discuss.critical`. The binary's event-kind enum REJECTS both:

```
{"error":{"code":"unknown_event_kind", ...}}
```

The accepted closed-set today is:
```
run.created, run.status, node.created, node.status, node.report,
discussion.opened, discussion.resolved, spinoff.proposed, spinoff.approved,
spinoff.rejected, child.spawned, supervisor.exited, supervisor.reattach-requested
```

Reported 2026-06-28 by an agent running /orchestrate on a real campaign in the deutschpad repo (commit 849d658, skills 0.0.1). The agent worked around it by writing decisions to a plain markdown file (`~/.taskfleet/runs/<id>/decisions.md`), but the SKILL explicitly states "A choice that does not appear in report.yaml is a contract violation" — and there is no path to make that happen with the current binary.

Impact: TWO of /orchestrate's core mechanisms (decision log + pakkopysäytys) are unusable as documented.

Fix direction (pick one, design call):
1. Extend the event-kind enum with `orchestrator.decision` and `discuss.critical`. Simplest. Each gets a reducer entry (probably no projection update — they're append-only audit records). The supervisor's terminal-cleanup logic must not be confused by these (they are not node.report).
2. Add dedicated CLI verbs: `taskfleet decision add ...` and `taskfleet pakkopysaytys open ...`. More explicit; harder for SKILLs to drift from.
3. Reuse existing kinds: e.g. `discussion.opened` for pakkopysäytys (severity field distinguishes critical), plus a new minimal kind or projection field for decisions.

Recommend option 1 — minimal change, keeps SKILL text simple, retains audit symmetry.

Acceptance:
- `taskfleet event create <run-id> --kind orchestrator.decision --from-file <json>` succeeds.
- Same for `discuss.critical`.
- Both events show up in `event tail <run-id>` and `report.yaml` (the final hierarchical report) can read them.
- /orchestrate SKILL.md text continues to work without modification.

Severity: BLOCKING for /orchestrate. The orchestrator runs but cannot honor its own contract.
