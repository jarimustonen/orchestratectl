# `plan.json` — v1 schema draft

The **interface contract** between the spec-node (writes it) and the supervisor
state machine + feature-orchestrator (read it). Designed for current needs,
versioned and forward-compatible. See design.md §9.

## Principles

- `schema_version` gates the whole file. Readers **tolerate unknown fields**
  (forward-compat) and branch on the version.
- **Minimal now, grown deliberately.** No speculative fields. When a spec/verify
  agent needs to express something the schema can't, it **files an improvement
  issue into the orchestratectl repo** (`issuectl new --type improvement`) rather
  than inventing an ad-hoc field — the schema then grows in a later version
  (design principle 4, self-improving tooling).
- **No counters, no thresholds.** Convergence, chunk sizing, and re-verify
  necessity are orchestrator/spec **judgment**, never numbers baked into the file
  (principle 1). The schema records *structure*, not *policy*.

## Shape (v1)

```json
{
  "schema_version": 1,
  "feature": {
    "slug": "user-csv-export",
    "intent": "A signed-in user can download all their own data as a CSV from the account page.",
    "source_branch": "main",
    "integration_branch": "feat/user-csv-export"
  },
  "acceptance": [
    "Downloading from the account page yields a CSV of exactly the user's own records",
    "An unauthenticated request is refused"
  ],
  "chunks": [
    {
      "id": "c1",
      "title": "CSV serializer for a user record",
      "deps": [],
      "tier": "code",
      "brief": "Self-contained, turnkey implementation brief: what to build, where, the interface it must expose, invariants/constraints, and how it plugs into existing code. Rich enough that a cheap model needs no architectural reasoning.",
      "files_touched": ["src/export/csv.rs"],
      "verify_criteria": [
        "round-trips a representative record",
        "empty and unicode fields are escaped correctly",
        "no new clippy warnings"
      ]
    },
    {
      "id": "c2",
      "title": "Account-page export endpoint",
      "deps": ["c1"],
      "tier": "code",
      "brief": "…turnkey brief for the endpoint, referencing c1's serializer interface…",
      "files_touched": ["src/routes/account.rs"],
      "verify_criteria": [
        "GET /account/export returns text/csv for the session user",
        "unauthenticated request → 401"
      ]
    }
  ]
}
```

## Field reference

| Field | Type | Owner | Meaning |
|---|---|---|---|
| `schema_version` | int | spec | schema version; readers branch on it |
| `feature.slug` | string | spec | stable id; names the integration branch |
| `feature.intent` | string | spec (from orchestrator) | **the invariant** — what must exist; the anchor for verify |
| `feature.source_branch` | string | orchestrator | where the feature merges back |
| `feature.integration_branch` | string | orchestrator | `feat/<slug>`, born at run creation |
| `acceptance[]` | string[] | spec | whole-feature product-vs-**intent** checks (verify's top-level bar), distinct from per-chunk criteria |
| `chunks[].id` | string | spec | chunk handle (deps reference it) |
| `chunks[].title` | string | spec | short human label |
| `chunks[].deps[]` | string[] | spec | chunk ids that must land first → the DAG (empty = independent, parallelizable) |
| `chunks[].tier` | enum `code\|mid\|high` | spec | starting model tier; **adaptive promotion** moves it up on repeated verify failure |
| `chunks[].brief` | string | spec | the self-contained implementation brief the code-node consumes |
| `chunks[].files_touched[]` | string[] | spec | hint (scopes the code-node); not a hard constraint |
| `chunks[].verify_criteria[]` | string[] | spec | per-chunk checks the verify-node runs |

## What is deliberately NOT in v1

- No iteration/round counters, token budgets, or size thresholds — policy lives in
  the orchestrator's judgment, not the file.
- No status/progress fields — run state lives in the event log + manifest, not here.
  `plan.json` is the spec's *output*, read-mostly; the supervisor tracks execution
  separately.
- No finding/verdict records — those are node.report assets (discussion_items /
  spinoff_proposals / fix_items), not plan.json.

## Open sub-questions to lock

- Does `tier` belong in `plan.json` (spec decides starting tier) or is it purely an
  orchestrator/runtime concern? Leaning: spec sets a starting hint, orchestrator
  owns promotion.
- Do we need an explicit `re_spec` marker when verify finds a **spec flaw**, or is
  that a node.report finding that re-triggers the spec-node (leaning: the latter —
  keep plan.json a pure spec output).
