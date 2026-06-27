---
created: 2026-06-27
updated: 2026-06-27
type: improvement
status: open
priority: normal
related: ['@ci-and-lints']
---

# Golden test pinning idempotency-key hash output

_Source: crates/octl-cli/src/idempotency.rs_

## Description

Surfaced by the ci-and-lints multi-model review (history/review-ci-and-lints.md, F14). The idempotency-key derivation (FNV-1a constants + input ordering) has no test pinning its output. A future silent change would break dedup: stored keys stop matching, so a retried run create / event append could double-execute or dedup could fail silently. The ci-and-lints change added digit separators to the FNV constants (verified byte-identical) — exactly the edit a golden test would protect. Add a golden assert_eq! pinning a known (input -> key) mapping with a stable example input.
