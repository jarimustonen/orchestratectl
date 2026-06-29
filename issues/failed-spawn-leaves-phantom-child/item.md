---
created: 2026-06-28
updated: 2026-06-29
type: bug
status: fixed
priority: normal
related: ['@headless-parent-session-rejected']
closed: 2026-06-29
---

# Failed create.sh leaves a phantom child run + child.spawned event on parent log

_Source: src/run/create_

## Description

When `run create --kind orchestrated` fails inside create.sh (e.g. the --headless/--parent-session crash, see related issue), orchestratectl has ALREADY emitted a `child.spawned` event on the parent run AND created a 0-node phantom child run that sits in `pending` forever.

Observed during /orchestrate smoke test (2026-06-28): failed --headless attempt produced child.spawned at parent seq 4 and phantom run 01kw7q48g1v2k15pnb2av4fvy3 (orchestrated, pending, 0 nodes). The orchestrator had to manually `run cancel` it.

Expected: a create.sh failure should be transactional — either no child run + no child.spawned event, or the child.spawned/run record should be marked failed automatically. As-is, the parent DAG bookkeeping is polluted and the operator must hand-clean. Found during /orchestrate end-to-end smoke test.
