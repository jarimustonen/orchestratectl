# `plan.json` — v2 schema draft (post-panel)

The **interface contract** the spec-node writes and the supervisor + orchestrator
read. Immutable per revision, versioned, provenance-bearing. See design.md §4, §7,
§13.

## Principles

- **`schema_version` gates the file, with real compatibility semantics.** Readers
  **reject unsupported major versions** and **reject undeclared required fields**;
  tolerant reading is limited to genuinely additive *optional* fields. Not "ignore
  everything unknown."
- **Immutable per revision.** A fix or re-spec writes `plan.v(N+1).json`; the prior
  revision is never overwritten. Each chunk attempt and verify report records the
  exact `plan_rev` (and `intent_rev`) it consumed.
- **Intent is referenced, not embedded.** `intent_rev` points at the
  orchestrator-owned `intent.md`; the plan cannot redefine intent.
- **Structure, not policy.** No counters/thresholds/budgets (those are orchestrator
  judgment + supervisor circuit-breakers). The schema records *what to build and
  how to check it*.
- **Governed evolution.** A gap → a structured schema-gap event → deduplicated,
  reviewed proposal → versioned schema. Never "agent asks → field added."

## Shape (v2)

```json
{
  "schema_version": 2,
  "plan_rev": 1,
  "intent_rev": 3,
  "feature": {
    "slug": "user-csv-export",
    "source_branch": "main",
    "integration_branch": "feat/user-csv-export"
  },
  "baseline": {
    "ref": "feat/user-csv-export@fork",
    "test_passlist_hash": "sha256:…",
    "clippy_warnings_hash": "sha256:…"
  },
  "acceptance": [
    {
      "kind": "check",
      "desc": "signed-in user downloads their own data as CSV end-to-end",
      "run": "cargo test --test e2e account_csv_export"
    },
    {
      "kind": "assertion",
      "desc": "the CSV contains exactly the user's own records and nothing else"
    }
  ],
  "chunks": [
    {
      "id": "c1",
      "title": "CSV serializer for a user record",
      "deps": [],
      "tier": "code",
      "brief": "Turnkey, self-contained implementation brief; rich enough that a cheap model needs no architectural reasoning.",
      "files_touched": ["src/export/csv.rs", "src/export/csv_test.rs"],
      "checks": [
        { "desc": "round-trips a representative record", "run": "cargo test export::csv::roundtrip" },
        { "desc": "empty + unicode fields escaped", "run": "cargo test export::csv::escaping" }
      ],
      "assertions": [
        "serializer API matches how the endpoint (c2) expects to call it"
      ],
      "requires_tests": true
    },
    {
      "id": "c2",
      "title": "Account-page export endpoint",
      "deps": ["c1"],
      "tier": "code",
      "brief": "…endpoint brief, referencing c1's serializer interface…",
      "files_touched": ["src/routes/account.rs", "src/routes/account_test.rs"],
      "checks": [
        { "desc": "GET /account/export returns text/csv for the session user", "run": "cargo test routes::account::export_ok" },
        { "desc": "unauthenticated → 401", "run": "cargo test routes::account::export_authn" }
      ],
      "assertions": [],
      "requires_tests": true
    }
  ]
}
```

## Field reference (v2 changes in **bold**)

| Field | Type | Owner | Meaning |
|---|---|---|---|
| `schema_version` | int (major) | spec | reader rejects unsupported majors |
| **`plan_rev`** | int | spec | immutable revision; chunk attempts reference it |
| **`intent_rev`** | int | orchestrator | the intent revision this plan targets (intent lives in `intent.md`) |
| `feature.slug/source_branch/integration_branch` | string | orchestrator/spec | as v1 (intent field removed — now referenced) |
| **`baseline`** | object | supervisor | snapshot at `feat/<slug>` fork; verify + floor diff against it |
| **`acceptance[]`** | object[] | spec | whole-feature intent gate; each is `check` (executable) or `assertion` (LLM-judged); **≥1 must be a `check`** |
| `chunks[].id/title/deps/tier/brief` | — | spec | as v1 (`deps` = DAG; `tier` = starting hint, orchestrator owns promotion) |
| `chunks[].files_touched[]` | string[] | spec | **now a merge-time constraint** (supervisor rejects out-of-scope merges beyond slack), not just a hint |
| **`chunks[].checks[]`** | object[] | spec | executable per-chunk checks; **≥1 required**. Shape: `desc` (the **general goal** — always present, human+LLM readable), `run` (a **flexible shell command**), optional `cwd` (a **safe repo-relative** dir — same guard as `files_touched`; omit for the worktree root, a bare `.` is rejected), optional `expect_exit` (an exit code `0..=255`; default 0). Precision available, not forced (owner decision 2026-07-23). |
| **`chunks[].assertions[]`** | string[] | spec | LLM-judged criteria (additive, above the floor) |
| **`chunks[].requires_tests`** | bool | spec | if true, supervisor blocks a merge that added/modified no tests |

## Recorded separately (NOT in plan.json)

- **Provenance per chunk attempt / verify report:** `plan_rev`, `intent_rev`,
  harness name+version, model+params, prompt/template version, base+result commit,
  check results, usage/tokens — in the event log, for causal replay.
- **Run/execution state, findings, decisions:** event log + node.report assets.
- **Credentials / routing config:** runtime config + execution events, never here.

## Open sub-questions to lock during build

- ~~Exact `check.run` contract~~ **RESOLVED (owner, 2026-07-23):** flexible — a check
  carries `desc` (general goal, always) + `run` (flexible shell command) + optional
  `cwd` / `expect_exit` (default 0). Precision available, not forced. Neither a rigid
  struct nor bare text. Issue `plan-check-run-contract` implements this. `cwd` is held
  to the `files_touched` repo-relative safety guard (the floor gates possibly-adversarial
  code-node output), and `expect_exit` is bounded to the shell range `0..=255`.
- DAG-diff algorithm for `plan.vN → v(N+1)`: which completed chunks revert to
  PENDING when their deps or briefs change.
- Whether `baseline` hashes live in `plan.json` or a sibling `baseline.json`
  (leaning sibling — it's supervisor-owned, not spec output).
