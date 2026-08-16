---
created: 2026-06-27
updated: 2026-06-27
type: improvement
closed: 2026-06-27
status: done
priority: normal
epic: orchestratectl-mvp
related: ['@core-path-traversal-id-validation', '@append-typed-node-id']
---

# octl-core: tighten id validators to canonical forms + typed Event ids

## Description

Spin-off from core-path-traversal-id-validation /llm-review (all four models).

DiscussionId/ProposalId currently accept d-/s- + 10-26 lowercase alphanumeric. That is fully path-safe but semantically loose: it accepts ids no generator emits (e.g. d-0123456789 is not RFC4648 base32; d-zzzzzzzzzz is not a ULID). Tighten to the exact union of the two real forms: {26-char Crockford ULID} ∪ {10-char RFC4648 base32 lowercase a-z2-7}. NOTE: current test fixtures use arbitrary lowercase-alnum ids, so this needs coordinated fixture updates.

Also (consensus): type Event.run_id/node_id as RunId/Option<NodeId> so deserializing events.jsonl validates the whole envelope on read (currently the reducer is the only validating boundary). And ergonomics: FromStr + PartialOrd/Ord on the id newtypes; try_exists (fail-closed) instead of Path::exists() in the supervisor dedup check.

Pure hardening/semantics — no known live path-traversal vector remains after the parent issue.

## Resolution

Done. Landed in four commits + a review-fix commit on `id-canonical-form-validation`:

- DiscussionId/ProposalId tightened to the syntactic union {26-char Crockford ULID} ∪ {10-char RFC4648 base32 a-z2-7} via `is_canonical_disc_or_proposal_body`; coordinated repo-wide fixture migration (code + insta snapshots) to canonical id forms.
- `Event.run_id`/`node_id` typed as `RunId`/`Option<NodeId>` — deserializing `events.jsonl` now validates the whole envelope on read; reducer clones the typed ids instead of re-parsing; write side validates via `parse_envelope_node_id` + new `Error::InvalidNodeId`.
- `FromStr` + derived `PartialOrd`/`Ord` on all four id newtypes.

4-model `/llm-review` + `/assess-findings` (see `history/assessment-id-canonical-form-validation.md`):

- **Reversed the supervisor-dedup directive.** The issue text said `try_exists().unwrap_or(true)` ("assume seen, skip"); unanimous review + the empirical trace showed that silently drops a never-emitted item (the report cursor advances regardless, no retry). The reducer is idempotent, so the safe choice is to **propagate** the unknowable existence error (`exists_or_io_err`): the batch aborts before the cursor moves and the report retries — no silent loss, no duplicate. Flagged for confirmation.
- Two clarifications worth recording for future readers: the 26-char Disc/Proposal arm checks charset only (does **not** enforce RunId's first-char ULID bound — RunId/NodeId deliberately untouched), and derived `Ord` is lexical, not numeric (`NodeId` sorts `n-10000 < n-9999`). Both now documented in-code rather than changed.

Deferred: `@append-typed-node-id` — change the core append API to take `Option<&NodeId>` instead of `Option<&str>` (touches ~15 callers; its own pass).
