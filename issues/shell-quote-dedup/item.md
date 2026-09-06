---
created: 2026-08-15
updated: 2026-08-21
type: improvement
status: untriaged
priority: normal
epic: lifecycle-architecture-review
---

# Dedupe shell_single_quote across run resume hints

## Description

`shell_single_quote` (single-quote shell escaping for copy-paste-safe run ids in resume hints) is duplicated verbatim in `crates/taskfleet-cli/src/run/false_failed.rs` and `crates/taskfleet-cli/src/run/attention.rs`. The two copies can drift.

Surfaced by llm-review (anthropic #10) during the `raw-git-selfmerge-false-failed` review.

**Scope:** extract the helper to one shared location (e.g. `crate::run::shell_quote` or `crate::output`), update both call sites, keep the existing hostile-id unit tests. Trivial, low-risk cleanup; kept out of the lifecycle fix to keep that change narrow.
