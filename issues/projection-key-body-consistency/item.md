---
created: 2026-06-27
updated: 2026-06-28
type: improvement
status: in-progress
priority: normal
epic: orchestratectl-mvp
related: ['@core-path-traversal-id-validation']
---

# octl-core: verify projection id key matches file body

## Description

Spin-off from core-path-traversal-id-validation /llm-review (gpt-5.5).

read_node/read_discussion/read_spinoff validate the embedded id newtype is well-formed but do not verify it equals the requested filename key — a valid nodes/n-0002.json placed at nodes/n-0001.json would be returned as n-0002, and a later write_node would write a different file than was read. Likewise write_* do not verify the object's run_id == paths.run_id (now feasible since run_id is a typed RunId).

Add: read_* reject when body id != requested key; write_* reject when run_id != paths.run_id; a CorruptProjection Error variant for machine-actionable diagnostics. Not a path-traversal vector (keys are already validated newtypes) — projection-integrity / corruption detection.
