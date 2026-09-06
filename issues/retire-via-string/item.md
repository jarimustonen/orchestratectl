---
created: 2026-08-15
updated: 2026-08-15
type: improvement
status: done
priority: normal
epic: lifecycle-architecture-review
commits:
- hash: 054dcb2
  summary: 'refactor(via): retire via-string merge authority for typed ReportOrigin'
- hash: e6b9c12
  summary: 'fix(via): apply llm-review findings (docs, layering, tests)'
closed: 2026-08-15
---

# Retire the via string in favor of typed ReportOrigin (reducer + node report)

## Description

Follow-up from the `/llm-review` of `typed-report-origin` (3-model consensus).
That change added a typed `ReportOrigin` and made `supervise::outcome::classify`
prefer it, but deliberately left the legacy `via: "explicit-merge"` string as the
authority for two OTHER surfaces (out of scope for that issue, which scoped
teardown/reducer via-semantics as unchanged):

- The taskfleet-core reducer's confirmed-explicit-merge **adoption against a terminal
  node** (`reducer.rs::report_is_confirmed_explicit_merge`) keys on `via`.
- `run wait`'s `landed`/`merged` **report-marker fallback** (`run/landed.rs`) keys
  on `via` when git cannot verify.

Because `node report` (the only agent-reachable append path) does NOT strip a
caller-supplied `via`, an agent can still put `{success:true, via:"explicit-merge"}`
into its own report and have these two surfaces treat it as a merge — a
**split-brain**: `classify` says `PlainSuccess` (origin=Agent), the reducer/`run
wait` say merged. Teardown is NOT affected (PlainSuccess → SourceRelative preserves
work), so this is observability/robustness, not a teardown-safety bug — the same
reason it was deferred.

## Proposed work

1. Update the reducer's confirmed-merge adoption to prefer `ReportOrigin::RunMerge`,
   falling back to `via` only when the origin field is genuinely ABSENT (parallel to
   `classify`'s gating). Keep legacy on-disk runs decoding unchanged.
2. Update `run/landed.rs`'s report-marker fallback the same way.
3. Once both consumers read the typed origin, strip a caller-supplied `via` in
   `node report`'s `normalize_agent_origin` (and update the `run_wait` test helper
   `settle_run` to fabricate merged state via a path that stamps a real origin, or
   via `run merge`, instead of forging `via` through `node report`).
4. Consider centralizing the stamp (a `build_supervisor_report` helper / an
   append-primitive authority marker) so a future supervisor-synthesized
   `node.report` site cannot forget to stamp `Supervisor` (llm-review finding #9).

See `history/review-typed-report-origin.md` for the full review dispositions.

