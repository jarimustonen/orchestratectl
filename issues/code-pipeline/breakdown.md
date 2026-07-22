# Code Pipeline — breakdown (epic → tasks, dependencies, critical path)

Sequenced from the v2 design (`design.md`) + panel non-negotiables. Each task
becomes a child issue when scheduled. Nothing here flips the default coding path
until T12; the pipeline ships **behind per-run config** and matures through shadow
+ canary (design §14).

## Tasks

| # | Task | Depends on | Why here |
|---|---|---|---|
| **T0** | **`CodeHarness` adapter interface + result protocol + conformance suite** (aider as the first adapter) | — | Foundational: the supervisor consumes only `ChunkResult`; everything downstream binds to this seam. Do NOT let a spike crown a harness (design §10). |
| **T1** | Tier→binding config + `run create --model` plumbing (surface `DEEPSEEK_API_KEY`/router creds to the code-node env) | T0 | Lets a node run on a chosen tier via an adapter; creds out of `plan.json`. |
| **T2** | `plan.json` v2 schema + validator (checks/assertions, immutable `plan_rev`, `intent_rev`, DAG validity, safe paths) | — | The stage contract; can be built in parallel with T0/T1. |
| **T3** | Deterministic floor: baseline snapshot at `feat/<slug>` fork + supervisor-enforced gates (tests/clippy vs baseline, file-scope at merge, test-gaming detection) | T2 | The panel's #1 demand; the mechanical oracle the autonomous loop needs (design §4). Expands supervisor capability (it runs checks). |
| **T4** | Inverted control loop: supervisor owns the event loop; orchestrator = stateless fn returning typed action primitives; structured decision envelopes | T0, T2 | Removes the control-loop contradiction + context-exhaustion risk (design §2). |
| **T5** | Staged supervisor state machine: spec→code→verify execution; **supervisor-side chunk merge** + deterministic merge-conflict protocol; DAG-diff on re-spec (`plan.vN→v(N+1)`) | T1, T3, T4 | The mechanical pipeline executor (design §6, §7). |
| **T6** | Resource circuit-breakers + **cost/token instrumentation** (per-node metering, real-time query, kill-switch) | T4 | Deterministic safety distinct from quality judgment (design §9). First sub-step: confirm/ build per-node usage metering. |
| **T7** | Verify stage: floor-aware verify, checks vs assertions, adversarial double-verify, triage auditability (record dismissed findings), FIX-must-reverify, promote-on-self-disagreement | T3, T5 | Verify as advisory layer above the floor (design §8). |
| **T8** | Skills: spec-brief authoring, orchestrator triage (the Opus stateless fn), code-node adapter briefs; intent.md authoring | T4, T5 | The LLM-facing prose contracts. |
| **T9** | Router-to-Claude-Code adapter (option A) **qualified behind `CodeHarness`** (upgrade-resilience, tool-call compat, failure attribution, version pinning) | T0 | Independent; an alternative adapter, not on the critical path. Evaluate **pi.dev** here too. |
| **T10** | Observability + provenance (causal IDs, plan/intent revision hashes, model/harness/prompt versions, transcripts) + repair/resume tooling | T4, T5 | Threads through; required before autonomous auto-merge is enabled. |
| **T11** | Staged-rollout wiring: per-run engine selector, shadow mode (plan+verify, no auto-merge), canary by kind/repo, legacy engine retained + rollback | T5, T7, T10 | Reversible deployment (design §14). |
| **T12** | Flip `code`/`spinoff`/`bugfix` defaults to the pipeline — only after measured stability | T11 | The "always how coding is done" end state. |

## Critical path

```
T0 ─┬─► T1 ─┐
    │       ├─► T5 ─┬─► T7 ─┐
T2 ─┴─► T3 ─┤       │       ├─► T11 ─► T12
    └─► T4 ─┘       ├─► T8  │
             T4 ─► T6 ──────┤
             (T4,T5) ─► T10 ┘

T9 (router adapter) — parallel, off the critical path.
T10 (observability) — starts with T4/T5, must land before T11 enables auto-merge.
```

Longest chain: **T0/T2 → T3/T4 → T5 → T7 → T11 → T12.**

## Immediate next (this session or next)

1. **T0** — write the `CodeHarness` trait + `ChunkRequest`/`ChunkResult` + the
   conformance-suite skeleton, with **aider** as the first conforming adapter.
2. In parallel, **T2** — turn `plan-schema.md` v2 into a checked-in schema + validator.

These two are unblocked and unlock the rest. T9 (router / pi.dev) can run whenever
without blocking.

## Owner decisions still open (design §15)

- **D1** principle-1 re-scope (applied) — confirm.
- **D2** passive post-merge rollup (applied) — keep/drop.
- **D3** trivial-task graceful-collapse vs hard skip-spec escape (collapse applied).
