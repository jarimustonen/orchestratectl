---
created: 2026-08-06
updated: 2026-08-11
type: task
status: in-progress
priority: normal
commits:
- hash: b3fd826
  summary: push_blocked_chunk records effective tier + commit oid
---

# push_blocked_chunk records plan tier not actual promoted tier, omits commit OID

## Description

push_blocked_chunk (crates/octl-cli/src/pipeline/live/mod.rs) records `tier: chunk.tier.wire_name()` (plan-declared) rather than the actual r.tier / run.chunk_tier, and always sets `commit: None` even for a floor-green Built preservation whose exact OID is known. A preserved report can misstate the tier and omits the recoverable commit OID.

Fix: thread r.tier and the Built commit OID into push_blocked_chunk (or build the ChunkReport directly from WaveBuildResult).

Source: /llm-review of entirely-faithful-beast (openai #8, #9). Pre-existing; affects both sequential and concurrent preservation paths.
