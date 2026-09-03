---
created: 2026-09-03
updated: 2026-09-03
type: improvement
status: done
priority: normal
provenance: other
provenance_detail: Taskfleet implementation run
source_ref: orchestratectl:01m1khdj7f4j4zdfb4cvstf8z1/task
originating_run: 01m1khdj7f4j4zdfb4cvstf8z1
originating_run_kind: spinoff
closed: 2026-09-03
closed_by: codex
---

# Conform skill installer to canon section 15

## Description

Bring Taskfleet's bundled skill catalog and installer into conformance with project-canon 0.8.0 §15. Claude, pi, and Codex must be explicit first-class targets; the default and `all` must select all three; catalog JSON must advertise the complete capability/layout contract; installation must support `--target`, `--dry-run`, explicit `--force`, and no-clobber behavior. Codex prompts must be self-contained while Claude/pi retain native resource trees.

## Acceptance Criteria

- [x] Claude, pi, and Codex are explicit first-class install targets, with `all` as the default.
- [x] Catalog JSON advertises selection, layout, safety, and version metadata.
- [x] `--target`, `--dry-run`, no-clobber, and explicit `--force` semantics are tested.
- [x] Codex prompts are self-contained while Claude and pi retain native Agent Skill trees.
- [x] Project-canon 0.8.0 §15 and the repository green gate pass.

## Verification

Run focused installer tests, project-canon 0.8.0 doctor/review against the local release binary, and the repository's full green gate. Update bundled guidance and repository documentation in the same implementation commit.

## Resolution

### 2026-09-03T13:03:49Z · @codex

Implemented and verified against project-canon 0.8.0: runtime review §15 passes on `./target/release/taskfleet`; the full repository green gate and isolated release-binary install probes pass.
