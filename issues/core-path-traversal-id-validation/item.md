---
created: 2026-06-12
updated: 2026-06-27
type: improvement
status: in-progress
priority: normal
epic: orchestratectl-mvp
---

# octl-core: validate IDs / typed ID newtypes

## Description

Spin-off from state-schema-crate review (gemini #13 / gpt-5.5 #13). paths.rs::node/discussion/spinoff accept raw &str IDs; a malformed or attacker-influenced ID with '/' or '..' can write outside the run directory. Add typed NodeId / DiscussionId / ProposalId / RunId newtypes with parse-time validation (charset + length + prefix), and have all path helpers + reducer consumers take the newtypes.
