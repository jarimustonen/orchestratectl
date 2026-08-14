---
created: 2026-08-14
updated: 2026-08-14
type: task
status: in-progress
priority: high
epic: lifecycle-architecture-review
labels: [architecture]
commits:
- hash: 95d9c25
  summary: core cut — kinds + mid-run discussion/spinoff machinery removed
- hash: b72e7d8
  summary: CLI cut — modules, dispatch, lifecycle inference, tests
- hash: b1b6b6d
  summary: skills + snapshots
- hash: bcb9147
  summary: changelog + doc cleanup; obsoleted bundled-orchestrate-skill
- hash: aa8925b
  summary: review fixes — enforce read-only Kind::Unknown (guard + fail-closed data_kind) + tests
---

# 0.2 subtractive cut: remove run kinds (code/orchestrate/orchestrated/bugfix/make-skill) + mid-run discussion/spinoff machinery

## Description

## Description

Second subtractive cut of the 0.2 simplification (ADR `docs/decisions/0001-thin-supervisor-vs-harden.md`, Migration sketch step 1; DECISION-1 `target-state-0.2.md`). Follows the landed pipeline/floor/harness-heavy cut (`cut-pipeline-floor-harness-heavy`). This is the **riskier** of the subtractive cuts — it touches `supervise/*` (the kind-derived lifecycle inference collapses once `Lifecycle::Interactive` empties) **and** the bundled skill set — so it is **sequenced solo**, not run in parallel with any Lane D skill.rs work, and it lands behind a green integrated gate.

**Breaking CLI change → v0.2.0-bound.** Clean break, no back-compat (single-user internal tool; ADR §D7). `doctor` is the migration mechanism for stranded install-surface; on-disk run history for removed kinds must be **reported, never deleted** (ADR §D7 — the 717-run evidence corpus).

## Scope (this cut only)

**Remove the run kinds** `code`, `orchestrate`, `orchestrated`, `bugfix`, `make-skill`:
- Delete these variants from the `Kind` enum (`crates/octl-core/src/schema.rs`) and every match arm / CLI arg / dispatch that only exists to serve them.
- Collapse the now-dead kind-derived lifecycle **inference** in `crates/octl-cli/src/supervise/*` — with `code` gone, `Lifecycle::Interactive` empties; remove the inference branches that fan out on the removed kinds (analysis.md §C.3 named this the accidental complexity). Keep the surviving `spinoff` / `fan-out` / `research` / `technical-decision` topologies working.
- Remove the corresponding bundled skills: `/worktree-code`, `/orchestrate`, `/worktree-orchestrated`, `/worktree-bugfix`, `/worktree-make-skill` (and any companion files), plus the `bundled-orchestrate-skill` surface. Update the `/worktree` router so it no longer routes to removed variants.

**Remove the mid-run discussion / spinoff-proposal machinery:**
- Delete the mid-run `discussion` / `spinoff-proposal` event kinds, reducer projections, CLI verbs, and supervisor handling that drive them **during** a run.
- **KEEP** the terminal-report fields `discussion_items[]` / `spinoff_proposals[]` (they ride the terminal `node.report`, per DECISION-1 / target-state-0.2.md) — this cut removes only the *mid-run* machinery, not the terminal report surface.

## Constraints / invariants

- Honor the 5 state-integrity invariants (root `AGENTS.md` / `CLAUDE.md`) — do not touch the reducer/lock/event append discipline except to delete now-dead event kinds through the proper `LockedRun` + `append_and_apply_*` paths.
- On-disk run dirs of removed kinds: `doctor` may **report**, must **not** delete (ADR §D7). A read-only permissive decoder so `doctor`/`run list` never fault on legacy data is acceptable if needed, but is not the core of this cut — prefer keeping it minimal.
- Obsoletes the `bundled-orchestrate-skill` issue (the `/orchestrate` skill is removed here).

## Green gate + review

- `cargo fmt --all`, `cargo clippy --workspace --all-targets` (no NEW warnings), `cargo test --workspace` (green). Refresh any insta snapshots the surface removal restales (CLI-surface change → the insta snapshot loop, `crates/octl-cli/CLAUDE.md`).
- This is production code + a breaking change: run `/llm-review` (+ `/assess-findings`) before merging.
- After merge, the integrated gate re-runs on `main`.

## References

- ADR: `docs/decisions/0001-thin-supervisor-vs-harden.md` (§D7 migration, Migration sketch step 1)
- DECISION-1: `issues/lifecycle-architecture-review/target-state-0.2.md`
- Design: `issues/lifecycle-architecture-review/design.md`
- Prior cut: `cut-pipeline-floor-harness-heavy` (done 2026-08-14)
