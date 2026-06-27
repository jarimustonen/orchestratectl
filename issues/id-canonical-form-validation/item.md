---
created: 2026-06-27
updated: 2026-06-27
type: improvement
status: in-progress
priority: normal
epic: orchestratectl-mvp
related: ['@core-path-traversal-id-validation']
---

# octl-core: tighten id validators to canonical forms + typed Event ids

## Description

Spin-off from core-path-traversal-id-validation /llm-review (all four models).

DiscussionId/ProposalId currently accept d-/s- + 10-26 lowercase alphanumeric. That is fully path-safe but semantically loose: it accepts ids no generator emits (e.g. d-0123456789 is not RFC4648 base32; d-zzzzzzzzzz is not a ULID). Tighten to the exact union of the two real forms: {26-char Crockford ULID} ∪ {10-char RFC4648 base32 lowercase a-z2-7}. NOTE: current test fixtures use arbitrary lowercase-alnum ids, so this needs coordinated fixture updates.

Also (consensus): type Event.run_id/node_id as RunId/Option<NodeId> so deserializing events.jsonl validates the whole envelope on read (currently the reducer is the only validating boundary). And ergonomics: FromStr + PartialOrd/Ord on the id newtypes; try_exists (fail-closed) instead of Path::exists() in the supervisor dedup check.

Pure hardening/semantics — no known live path-traversal vector remains after the parent issue.
