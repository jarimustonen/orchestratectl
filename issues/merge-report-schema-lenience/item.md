---
created: 2026-08-12
updated: 2026-08-12
type: feature
status: open
priority: normal
---

# run merge should not block a clean merge on a malformed advisory report field

## Description

## Observed (glasspad stint 2026-08-11/12)
Two independent autonomous spinoff workers, in the same session, wrote their terminal report's `spinoff_proposals[]` with the WRONG field names — `title`/`detail` instead of the schema's `proposed_title` / `proposed_kind` / `rationale`. `orchestratectl run merge --report-file <f>` validates the report BEFORE the merge, so the schema violation (`spinoff_proposals[0].proposed_title must be a non-empty string`) **rejected the whole report and blocked the merge** of already-committed, green, /llm-review'd code. In the B2 case the run sat `pending` (worktree looked stuck) until the worker noticed, rewrote the report, and re-ran `run merge`.

## Why it's a foot-gun
- `spinoff_proposals` is **advisory** (follow-up suggestions the parent may ignore), yet a typo in it blocks the *actual code merge* — the high-value, irreversible-ish operation — on the *lowest-value* part of the payload.
- The field-name mismatch recurred across two different worker agents → the naming (`proposed_title` vs the intuitive `title`) is a predictable trap, not a one-off.

## Proposed
Make `run merge` resilient to a malformed *advisory* report section. Options (pick during triage):
1. **Merge-first, then validate report**: perform the branch merge, then validate/persist the report; a bad advisory field is surfaced as a warning and the offending proposals dropped/flagged, NOT a merge blocker. (The merge is what the caller actually needs; the report is metadata.)
2. **Lenient advisory parsing**: accept common alias keys (`title`→`proposed_title`, `detail`→`rationale`) for `spinoff_proposals[]`/`discussion_items[]`, or drop unparseable proposals with a warning instead of failing the whole call.
3. At minimum, **validate the report file up front with a dedicated `run report validate`-style preflight** the spinoff SKILL can call, and make the error name the exact expected field set inline.

Keep required top-level fields (`success`) strict; only the advisory arrays should degrade gracefully. Do not silently drop a *required* field.

## Repro
```
orchestratectl run merge <run-id> --report-file f.json
# where f.json has spinoff_proposals: [{ "title": "...", "detail": "..." }]
# → error.code schema_violation, merge NOT performed, node stays live/pending
```
