---
created: 2026-08-15
updated: 2026-08-15
type: improvement
status: open
priority: normal
epic: lifecycle-architecture-review
---

# Teardown: validate source_branch / recorded branch as durable refs/heads refs

## Description

Follow-up from /llm-review of detached-head-teardown-commit-loss (openai D2, deepseek D2). The teardown guards use `manifest.source_branch` and `Node.branch` as reachability proofs via `git rev-list --count <source>..<x>`, but only prove the string RESOLVES at that moment — not that it is a durable `refs/heads/*` ref. A malformed/legacy manifest holding a raw OID, a tag, `HEAD`, or a revision expression could make `source..HEAD == 0` classify a worktree Safe even though no durable branch protects the commit after removal. The empty-string case is already fixed (finding A: `manifest_source_branch` filters empty + `Git::rev_list_count` rejects empty endpoints). This tracks the broader hardening: resolve/validate source as `refs/heads/<name>` before trusting it, and normalize `--short` vs full-ref spellings before the recorded-branch equality compare. Whether this is reachable from the current `run create` path is unconfirmed — validate before investing.
