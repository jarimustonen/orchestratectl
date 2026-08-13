# ADR 0001 — Re-architect the lifecycle core to a THIN supervisor (vs. harden the polling-inference model)

- **Status:** Accepted
- **Date:** 2026-08-13
- **Deciders:** Jari (product owner) + the `arch-redesign-design-session` `/llm-workshop` (gemini-3.1-pro, gpt-5.6-sol, deepseek-v4-pro — all "revise": direction sound, negative-case mechanics folded in as A1–A6)
- **Epic:** `lifecycle-architecture-review` (Lane F) · **Phase 3** (the decision point) · **◆ DECISION-2**
- **Supersedes/gated by:** `arch-redesign-design-session` (Phase 2) · **◆ DECISION-1** (`target-state-0.2.md`)
- **Target release:** **0.2.0** (the simplification) + **0.2.1** (the deferred pi.dev self-reporting plugin)

> This ADR **records** a decision already made with Jari in the design session; it does not
> re-open it. Its second job is the **per-issue re-triage** of the gated Lane A + Lane E
> backlog against the decided model (§7). The authoritative design is
> [`issues/lifecycle-architecture-review/design.md`](../../issues/lifecycle-architecture-review/design.md);
> this ADR is its durable, decision-shaped record.

---

## Context

The run/supervisor/agent lifecycle subsystem is the source of ~all open cluster-A/B bugs.
Phase 1 (`analysis.md`, `feature-audit.md`, `alternatives.md`) established the root cause and
the usage evidence; Phase 2 (`target-state-0.2.md` = DECISION-1, then `design.md` = the design
session) decided the target. The two decisions this ADR records:

- **DECISION-1** (already decided, PO review 2026-08-12): the cut/keep/reframe of the 9 kinds
  and the heavy layers (`target-state-0.2.md`).
- **DECISION-2** (this ADR): the shape of the **surviving supervisor core**, and the
  disposition of every gated Lane A + Lane E issue against it.

### The problem (evidence pointers)

**Phase 1 — root cause** (`analysis.md §C`): the supervisor is an *inference engine* that
reconstructs a distributed agent's true state by polling a **cross-product of indirect
proxies** — PID liveness × tmux window/pane presence × git branch-ancestry × worktree
cleanliness × **three activity clocks** × file timestamps — because the agent has exactly one
first-class, optional, lossy way to report: a terminal `node.report`. The edge-case count is
the combinatorial product of proxy states, so **patching one cell exposes its neighbours** and
the open-issue count does not shrink under patching.

- **Observed in the wild** (`analysis.md §C.2.5`, `TODO.md`): landing
  `agent-skips-run-merge-idle-pending` *immediately spawned three more* cluster-A refinements
  (`idle-unmerged-{monotonic-clock,process-tree-cpu,e2e-preservation-test}`) — the hypothesis's
  central prediction, observed.
- **Bucket distribution** (`analysis.md §B`): of 28 open cluster-A/B issues, **~24 are direct
  consequences** of reconstructing distributed state from indirect signals.
- **Essential vs accidental** (`analysis.md §C.3`): the **crash-atomic event store**
  (`applied_seq` / `LockedRun` / `LOCK_SH`) is the one bug-free layer — no open issue targets
  `events.rs`/`lock.rs`/`reducer.rs` correctness. The activity clocks, the git-reconcile
  fallback, the synthetic `merge-reconciled` report, the tmux tri-state, and the supervisor-
  liveness heuristics are **accidental complexity** — the cost of an absent protocol.

**Phase 1 — usage evidence** (`feature-audit.md`, 717 runs): `spinoff` = 83% of all runs / 96%
of the last 120; `fan-out` is a real distinct need; the interactive, orchestrate, pipeline, and
multi-harness surfaces are near-unused. The working model is **stint → PO review → stint**,
with interactivity reached for occasionally.

### The fork (from `alternatives.md`)

1. **Thin model** — `run merge` is the only completion truth; delete the inference.
2. **Protocol model** — the worker self-reports `spawned→working→merging→merged|blocked|failed`
   transitions + renews a lease; PID liveness demoted to a pure crash backstop.
3. **Exit-code + a launcher shim** — the cheap adjunct that helps *either* model.

Both realize "told, not guessed"; they differ in **how much the worker tells** and therefore
in **how much worker discipline** they demand.

---

## Decision

### D1 — The supervisor core is the THIN model

**A unit is DONE iff the worker called `orchestratectl run merge`.** Under the run lock, that
call rebases + merges + appends the durable `explicit-merge` transition — and that append **is**
the completion fact. `run merge` is the **only success-completion truth**.

Everything the current watchdog does to *guess* done-ness is **deleted**:

- the git-reconcile-implies-done probe + the synthetic `merge-reconciled` report,
- the three activity clocks (commit-time / pane-mtime / CPU-rate) + the CPU baseline window —
  the whole idle-unmerged synthesizer,
- the tmux tri-state / streak-gating / pane-aware liveness matrix as a *primary* signal,
- the lifecycle-branched liveness re-basing (there is no `Lifecycle::Interactive` category any
  more — see D3).

The **inference cross-product `pid × pane × branch × clock` is removed**. Liveness survives only
as a **pure crash backstop**, never a primary signal.

The protocol model (self-report + lease) is the *better mechanism* but its cost is worker
discipline — every bundled SKILL must emit transitions at the right points, and a skipped emit
looks like a hang; that is exactly the "capability ahead of use" drag 0.2 removes. It is
**DEFERRED to 0.2.1 as a pi.dev plugin — not cut**: because pi.dev becomes the harness we
actually run, self-reporting lands in *one* place (the harness) instead of being sprinkled
across recipe skills. The lease (the clean answer to the 9-issue supervisor-liveness bucket)
rides in on that plugin.

### D2 — The six hardenings (A1–A6), from the 3-model critique

The critique was unanimous that the direction is sound but the **negative-case mechanics** were
underspecified. All six are part of the decision, not optional polish:

| # | Hardening | Rule |
|---|---|---|
| **A1** | **Launcher-shim true exit status** | Wrap worker launch in a thin shim (`octl-run-worker <run> <node> -- pi …`) that `wait()`s on the child and records its **true exit status** as a durable event under the run lock. Died-vs-finished becomes a **told fact**, not a pid guess. `non-zero/signal → failed`; `exit 0 + explicit-merge → done`; **`exit 0 + no merge → attention-required` (never auto-failed)** — the old safety-net case becomes a visible, resumable state. A shim, not a protocol — no per-SKILL churn. |
| **A2** | **Bounded merge-transaction recovery** | `run merge` spans git refs + the event log and is not atomic across them. `run merge` appends `merge.started{op_id, expected_source_oid, worker_oid}` **before** the git mutation (compare-and-swap on the source ref); on the next lock acquisition the supervisor **finishes or rejects that one known transaction by OID** — a deterministic completion of a recorded transaction, **not** the general git-reconcile heuristic. |
| **A3** | **Fenced manual resume/finish skill** | A human-invoked, `LockedRun`-witnessed skill that the PO drops into a stuck worktree: acquire the run lock, verify pid identity, **fence the prior worker** (SIGTERM), then either drive `run merge` **directly from the worktree's git state** (bypassing the deadlocked agent) or launch one fresh agent — **never alongside a live original**. Refuses terminal nodes; idempotent against a duplicate merge. It never `tmux send-keys`-guesses at a wedged opaque agent. |
| **A4** | **Fixed post-death grace, not an activity clock** | The residual automatic backstop fires only when the shim is *lost* (hard kill of the shim / host death): **process confirmed gone (pid + start-token identity) AND no exit event AND no merge event AND a fixed short grace (~5 s) elapsed → `failed`, preserve branch**. The grace is anchored to the **first confirmed-death observation** (persisted, monotonic, survives supervisor restart) — its only job is to let an in-flight merge/exit append finish. Immediately before appending `failed`, the supervisor **re-reads the log under the exclusive lock**; a merge/exit that landed in the race wins. |
| **A5** | **Bounded visibility + per-node cancel** | The manual model needs eyes without terminalizing. **`run wait --timeout`** returns a distinct **non-terminal `attention-required`** result (does not mutate lifecycle). **`run list` / `run show`** expose pending-age, last-observed pid, worktree path, and a one-line resume hint. **Per-node `run cancel <node>`** (branch-preserving) unblocks a single stuck fan-out child without killing the batch. |
| **A6** | **Typed outcome table** | Negative outcomes are a **typed transition table**, not an interpretation of signal combinations. `run merge` is the only **success** truth — **not** the only **terminal** truth. |

**A6 — the outcome table:**

| Source | Outcome | Teardown? |
|---|---|---|
| `run merge` succeeds (`explicit-merge`) | `done` | yes |
| exit-status event non-zero / signal | `failed` | no (preserve branch) |
| exit 0, no merge event | `attention-required` (non-terminal) | no — manual finish |
| `run cancel` (run or `--node`) | `cancelled` | **no — preserve branch + worktree** (inv. 5) |
| confirmed-death backstop (A4) | `failed` | no (preserve branch) |
| terminal `node.report{blocked}` | `blocked` (non-terminal, manual) | no |

`cancel` **preserves** work (never a teardown authorization) — closing the critique gap that an
implementer might read `cancel` as delete.

### D3 — Engine end-state: 9 kinds → 2 topologies + a flag + recipes

The engine (schema/reducer/supervisor) knows only **topology**; everything else moves out.

| Today's kind | 0.2 disposition | Mechanism |
|---|---|---|
| **spinoff** | ✅ topology | the core; **always headless** |
| **fan-out** | ✅ topology | N parallel independent units — the one multi-node topology |
| **research** | → thin recipe | generic spinoff + `research` recipe |
| **technical-decision** | → thin recipe | generic spinoff + `adr` recipe |
| **bugfix** | → thin recipe | kind arm deleted; recipe kept |
| **make-skill** | → thin recipe | kind arm deleted; recipe kept |
| **code** | ❌ kind removed | value = interactivity → the `--interactive` flag |
| **orchestrate** + **orchestrated** | ❌ removed entirely | multi-feature campaign absorbed by the stint loop |

Net: the supervisor's *default* assumption is an **autonomous single-node worker** (unless
`--interactive`), collapsing the `Lifecycle` enum and ~24 kind-derived branch points.
Interactivity stops being *derived from `Kind`* and becomes **one explicit `--interactive`
flag** (D6). **Fan-out deliberately stays a single run with N nodes** (not N runs — the batch
needs one rollup, one wait, budgeted concurrency); the reducer keeps its **node-set rollup +
per-node teardown** (essential, not kind-derived); only the parent/child *cross-run* DAG
bookkeeping of `orchestrate`/`orchestrated` is deleted.

**Recipes are thin, exec-able skills over ONE generic spinoff engine.** The shared machinery
(spawn / worktree / `run merge` / teardown) lives once in the CLI (Rust), behind verbs; recipes
reference the verbs, never restate them. A recipe loads into the worker's context only when
`exec`'d, and can call `/llm-review` + `/assess-findings`. Adding a recipe = one thin file —
"more recipes is better".

**Also cut (DECISION-1, executed here):** `pipeline` + `floor` (~20k LOC), the heavy `harness`
layer (`bakeoff` + `conformance` + the `CodeHarness` trait + `aider` + `claude-deepseek`), the
`--harness` selection stack (4-level precedence resolver + `manifest.harness`/`harness_source`
provenance + DTO columns), and the mid-run `discussion`/`spinoff` machinery — keeping only the
terminal-report `discussion_items[]`/`spinoff_proposals[]` fields.

### D4 — Harness: pi.dev universal default, claude interactive-only opt-in

- **pi.dev = the universal default** for every run, autonomous and interactive. Autonomous runs
  are always pi.dev.
- **claude = the interactive, non-default opt-in** — reached only under `--interactive`.
- Implemented **as a default + config, not a hard engine coupling** (critique B1): hard-wiring
  "claude ⟺ interactive" into an engine that otherwise knows only topology would itself be a
  capability-scoped constraint. The engine stays topology-only; the harness choice is a
  launcher-level default Jari controls. Product intent is expressed as the default, not enforced
  by a special-case branch.

### D5 — What survives (the essential residue)

- the **crash-atomic event store** — `applied_seq` watermark, `LockedRun` witness, `LOCK_SH`
  reads (keep exactly as is),
- a **terminal-outcome contract** — `explicit-merge` + the new exit-status event (A1) + the
  terminal `node.report` (with `discussion_items[]`/`spinoff_proposals[]` retained as report
  fields),
- the **merge-transaction record** (A2) — the `merge.started` op-log,
- the **merge-assertion teardown gate** — invariant 5, `cleanup.rs` (branch/worktree preserved
  unless a confirmed `run merge`),
- **pid liveness as a pure crash backstop only** (A4), never a primary signal.

### D6 — `--interactive` semantics

A single flag, not a second lifecycle: the supervisor **never** auto-terminalizes and **never**
auto-tears-down; it waits for an explicit `run merge` (→ teardown) or `run cancel`; a **dead pid
is ignored** (the human may have quit/restarted the agent — the ~1.5-day
`agent-died-merge-no-teardown-interactive` case); no crash backstop, no idle net, no timeout.
`doctor` **reports** abandoned interactive runs (pending past a long age, no live pid) with
non-destructive resume/cancel guidance — it flags, it does not terminalize or delete.

### D7 — Migration: clean break + doctor sweep

**No backward compatibility — clean break in 0.2.0.** Justification: single-user internal tool;
every call site (bundled skills + Jari's habits) updates in the same release; a deprecation
window would mean carrying *both* surfaces — the exact dead-weight drag 0.2 removes. `doctor` is
the migration mechanism, in two tiers:

- **Safe to prune (install-surface):** removed-kind skills stranded in `~/.claude/skills/`
  (`/worktree-bugfix`, `/orchestrate`, `/worktree-orchestrated`, `/worktree-make-skill`,
  pipeline skills), orphan companion files, dead `config.toml [harness]` sections, deregistered
  sync rows — removed via an explicit `doctor` prune/fix action.
- **Never destroyed (history data):** `~/.orchestratectl/runs/*` dirs whose kind was removed —
  the very evidence `feature-audit.md` mined (717 runs). `doctor` may **report** them; it must
  not delete them.
- **Read-only legacy decoder (A6, critique):** "no CLI back-compat" does not eliminate on-disk
  compat. 0.2.0 ships a permissive, read-only parser that reads pre-0.2 run dirs into a reduced
  view so `doctor`/`run list` never fault on old data. A bounded read-only parser, **not** a
  compat shim in the hot write path.

---

## Blast radius

- **Deleted (~20k+ LOC):** `pipeline` + `floor`; the heavy `harness` layer (`bakeoff`,
  `conformance`, `CodeHarness` trait, `aider`, `claude-deepseek`); the `--harness` precedence
  stack + provenance fields; the mid-run `discussion`/`spinoff` machinery (events, projections,
  counters, CLI subcommands); the watchdog inference core (`watchdog_tick` activity clocks,
  git-reconcile probe, synthetic `merge-reconciled`, tmux tri-state/streak/pane matrix,
  lifecycle re-basing); the `orchestrate`/`orchestrated` cross-run DAG bookkeeping; the `code`
  kind + its Lifecycle inference.
- **Added / re-shaped:** the launcher shim + exit-status event (A1); the `merge.started`
  op-log + CAS recovery (A2); the fenced resume/finish skill (A3); the fixed-grace backstop
  (A4); `run wait --timeout → attention-required`, `run cancel --node`, richer `run list/show`
  (A5); the typed outcome table (A6); the `--interactive` flag (D6); the `doctor` prune +
  read-only legacy decoder (D7); the thin recipe skills over the generic engine (D3).
- **Untouched (kept verbatim):** the crash-atomic event store (`events.rs`/`lock.rs`/
  `reducer.rs`/`schema.rs` store layer), the merge-assertion teardown gate (invariant 5), the
  notify back-channel (`notify.rs`).
- **Correctness-sensitive files to sequence (never parallelize edits):**
  `crates/octl-core/src/{events,lock,reducer,schema}.rs`, `crates/octl-cli/src/supervise/*`,
  and the to-be-deleted `crates/octl-cli/src/{harness,floor,pipeline}/*`.

## Migration sketch (within 0.2.0, if the big-bang risk bites — see below)

1. **Subtractive cuts first, behind a green integrated gate:** `pipeline`/`floor`, the harness
   heavy layer, the removed kinds, the discussion/spinoff machinery. Largest deletion, most
   bisectable.
2. **Thin supervisor + exit shim (A1/A2/A4/A6):** replace `watchdog_tick` with the exit-status
   read + typed outcome table + bounded backstop + merge-transaction recovery.
3. **Visibility + interactive (A5/D6):** `run wait --timeout`, `run cancel --node`, richer
   `run show`, the `--interactive` flag.
4. **Recipe repackaging (D3/D4):** collapse kinds to recipes over the generic engine; pi.dev as
   default; the fenced resume/finish skill (A3).
5. **Migration tooling (D7):** `doctor` prune + read-only legacy decoder.

Each layer lands under the **integrated green gate** (`cargo test --workspace` on integrated
`main`, per operating policy) so a regression is bisectable to one layer.

---

## Release plan

- **0.2.0 — the simplification.** Engine collapse (D3), thin supervisor (D1/D2), `--interactive`
  (D6), pi.dev-as-default (D4), recipe hybrid (D3), the fenced resume/finish skill (A3), clean
  break + doctor sweep (D7). Breaking CLI change.
- **0.2.1 — the pi.dev self-reporting plugin.** The deferred protocol path: worker transition
  self-reporting + a supervisor/worker lease, built inside pi.dev. Collapses the supervisor-
  liveness bucket if/when it lands.

---

## Consequences

**Positive.** The accidental-complexity buckets (`analysis.md §C.3`) collapse: the activity
clocks vanish, the git-reconcile fallback and synthetic report vanish, the tmux tri-state and
lifecycle re-basing vanish, ~24 kind-derived branch points vanish. Completion becomes a told
fact (`explicit-merge`) or a told exit status (A1) — a lookup, not an inference. The one bug-free
layer (the store) is kept verbatim. The read surface stops re-deriving health.

**Negative / accepted.** The two watched risks below. Plus: the manual resume/finish path (A3)
replaces an *automatic* rescue with a human-invoked one — accepted because stuck runs surface at
PO-review cadence and are made visible by A5 (Jari also wanted a "resume this worktree" command
independently). And the supervisor-liveness bucket is not solved in 0.2.0 — it is explicitly
deferred to the 0.2.1 lease.

### Watched risks (noted, not designed away — Jari's call)

1. **Web-tools default-on = exfil / prompt-injection surface** (critique B2). Every worker,
   including code-fix workers with no web need, can reach the network → data-exfiltration and
   prompt-injection exposure. **Accepted for now** (single-user tool; Jari confirmed "all can
   open web tools"). **What would change the call:** multi-user use, or handling untrusted
   repos/inputs → switch to **recipe-declared static capabilities** (research declares web;
   bugfix does not), which is also cleaner "told-not-guessed".

2. **0.2.0 is a big-bang breaking release** (critique B3). It bundles the supervisor rewrite +
   ~20k-LOC deletion + schema/event removal + harness-default migration + recipe repackaging +
   migration tooling in one release → poor fault isolation. **Accepted** (Jari: "ei haittaa").
   **Mitigation available if it bites:** the layered sequencing above (subtractive cuts →
   thin supervisor → visibility → recipes → tooling), each behind a green integrated gate, so a
   regression bisects to one layer rather than the whole release.

---

## Alternatives considered

- **Harden the polling-inference model (do not re-architect).** The counter-position: the
  essential residue (`analysis.md §C.3`) is real and the crash-atomic store is sound, so
  accidental complexity could be *contained* (`watchdog-tick-verdict-refactor` + fast-tracking
  model-independent fixes) without a rewrite. **Rejected:** `analysis.md §C` shows patching one
  proxy cell reliably exposes its neighbours — the open-issue count does not shrink under
  patching (observed: the agent-skips fix spawned three more). Containment leaves the
  combinatorial surface intact.
- **Protocol model now (worker self-reports + lease, in 0.2.0).** The better mechanism.
  **Rejected for 0.2.0, deferred to 0.2.1:** its cost is worker discipline across every SKILL —
  the "capability ahead of use" drag 0.2 is removing. It lands cleaner as a single pi.dev plugin
  once pi.dev is the default harness (D1).

---

## Per-issue re-triage of Lanes A + E

See **§7 in [`design.md`](../../issues/lifecycle-architecture-review/design.md)** for the
predicted mapping; the **formal, applied dispositions** are recorded here and stamped onto each
issue via `issuectl` (a `## Decisions` note, plus a close-to-`obsolete` or a `defer-0.2.1` /
`keep-0.2` / `rescope-0.2` label). `signal-exit-143-regression` is **carved out** (CI-red,
model-independent, fast-tracked in a parallel worktree this round) and is **left untouched**.

### Legend
- **KEEP-and-fix** — surface survives; fix stays open (model-independent).
- **DEFER-to-0.2.1** — the clean answer is the pi.dev self-report/lease plugin; open, `defer-0.2.1`.
- **OBSOLETE-as-subsumed** — the surface it targets is deleted by the thin model; closed `obsolete`.
- **RE-SCOPE** — survives but re-framed against the new model; open, `rescope-0.2`.

### Lane A (supervise / agent-lifecycle core)

| Issue | Disposition | Rationale |
|---|---|---|
| `signal-exit-143-regression` | **CARVED OUT** | Not gated; fast-tracked in a parallel worktree. Untouched by this ADR. |
| `merge-report-schema-lenience` | **KEEP-and-fix** (fast-track) | The terminal-report contract survives (D5); merge-first-then-validate is model-independent and low-risk. |
| `legacy-pid-identity-check` | **KEEP-and-fix** | PID identity survives as the crash backstop's recycle defense (A4). |
| `no-completion-notification-to-parent` | **KEEP-and-fix** | The notify back-channel survives (D5); multi-child robustness still wanted. |
| `notify-run-level-summary` | **KEEP-and-fix** | Fan-out + notify survive; a run-level multi-node summary applies. |
| `teardown-gate-trust-and-lifecycle` | **RE-SCOPE** | The teardown gate survives (inv. 5), but the report-shape trust decision is re-framed by the typed outcome table (A6) + exit shim (A1). Re-scope to the typed-outcome gate. |
| `run-salvage-command` | **RE-SCOPE** | Becomes the fenced manual **resume/finish skill** (A3) — generalized from dead-branch salvage to live-worktree resume. |
| `idle-unmerged-monotonic-clock` | **OBSOLETE** | The activity-clock synthesizer is deleted (D1). |
| `idle-unmerged-process-tree-cpu` | **OBSOLETE** | Same — the CPU clock is deleted (D1). |
| `idle-unmerged-e2e-preservation-test` | **OBSOLETE** | Tests the synthesized idle-unmerged report path, which is deleted; branch-preservation is now covered by the A6 outcome table (`cancel`/`failed` preserve). |
| `idle-empty-handed-alive-agent-hangs` | **OBSOLETE** | Subsumed by the exit shim (A1: `exit 0 + no merge → attention-required`) + bounded visibility (A5). |
| `watchdog-pane-aware-liveness` | **OBSOLETE** | The tmux pane-aware/tri-state matrix is deleted as a primary signal (D1). |
| `watchdog-tick-verdict-refactor` | **OBSOLETE** | Refactors the `watchdog_tick` inference core, which is deleted (D1). |
| `autoretry-crash-consistency` | **OBSOLETE** | Hardens the agent-died auto-retry synthesizer; under A1/A6 a non-zero exit → `failed` (preserve branch), no retry loop. |
| `interactive-merge-audit-marker` | **OBSOLETE** | Disambiguates `explicit-merge` vs the synthetic `merge-reconciled`; the reconciled path is deleted, so `explicit-merge` is the only success marker. |
| `code-run-inject-no-selfmerge` | **OBSOLETE** | The `code` kind + its SKILLs are removed (D3); interactive runs let the human own the merge (D6). |
| `moderately-macabre-self` | **OBSOLETE** | Reciprocal parent/child adoption is `orchestrate`/`orchestrated` cross-run bookkeeping, which is cut (D3). |
| `child-supervisor-spawn-exhaustion-lifecycle` | **OBSOLETE** | Child-supervisor spawning is the cut orchestrate/orchestrated topology (D3); fan-out is one run with N nodes, no child supervisors. |
| `orchestrate-integration-branch-no-worktree-merge-fails` | **OBSOLETE** | `orchestrate` + its integration-branch machinery are removed (D3). |
| `supervisor-stall-detection` | **DEFER-to-0.2.1** | Supervisor-liveness bucket → the supervisor lease (0.2.1 plugin). |
| `supervisor-spawn-fails-silently-at-run-create` | **DEFER-to-0.2.1** | (HIGH) Supervisor-existence inference; the clean answer is the lease. Severity noted for 0.2.1. |
| `run-create-back-to-back-no-supervisor` | **DEFER-to-0.2.1** | Same supervisor-existence bucket → lease. |
| `reattach-does-not-bootstrap-crashed-at-creation-run` | **DEFER-to-0.2.1** | Same bucket → lease. |
| `cancel-dead-supervisor-recovery` | **DEFER-to-0.2.1** | Dead-supervisor recovery is the lease's job. |
| `peculiarly-cheerful-mine` | **DEFER-to-0.2.1** | An explicit driver heartbeat/lease — the protocol's job, deferred with it. |
| `uncommonly-fuzzy-swing` | **DEFER-to-0.2.1** | (HIGH) `blocked→parent` propagation is a missing protocol transition → the self-report plugin. |

*(`worker-process-hang` is `in-progress`/parked and out of the gated set — WHY the pid exits is
agent-runtime scope, not this subsystem; left as-is.)*

### Lane E (run/* read surface)

| Issue | Disposition | Rationale |
|---|---|---|
| `run-show-null-worktree-path` | **KEEP-and-fix** | A5 *requires* `run show` to expose the worktree path for `attention-required` runs — the fix becomes mandatory, not obsolete. |
| `node-show-null-report` | **KEEP-and-fix** | The terminal-report surface survives (D5); reading `last_report` correctly is a model-independent read bug. |
| `count-jsons-swallows-io` | **RE-SCOPE** | The discussion/spinoff projection counts it guarded are cut (D3); the residual node count should read the authoritative manifest counter, closing the swallowed-IO path (`analysis.md §C.4`). |

**Divergence from `design.md §9`'s prediction, noted:** §9 predicted "most of the run-show DTO
re-derivation (Lane E) → OBSOLETE." That is true of the **read-side health-inference machinery**
(`SupervisorView` five-state, `stalled`/`stillborn`/`orphaned`, landing re-derivation) — but
those tickets (`supervisorview-conflates-states`, `run-wait-still`,
`landing-signal-reliable-after-rebase`) already **landed/closed**. The three *open* Lane E
issues are plain field-read bugs on surfaces the thin model **keeps** (indeed A5 strengthens
`run show`), so they are KEEP/RE-SCOPE, not OBSOLETE.

---

## References

- Design (authoritative): `issues/lifecycle-architecture-review/design.md`
- DECISION-1 frame: `issues/lifecycle-architecture-review/target-state-0.2.md`
- Root-cause analysis: `issues/lifecycle-architecture-review/analysis.md`
- Usage evidence: `issues/lifecycle-architecture-review/feature-audit.md` (717 runs)
- The thin/protocol fork: `issues/lifecycle-architecture-review/alternatives.md`
- Expert thread: group `group_c292390c734842e89c7a1fe0e8ca2f12` (gemini-3.1-pro
  `api_74fe455e631848a0b71ebb3fae300809`, gpt-5.6-sol `api_38f886f00b264d5e8a1fbcd5d2ae36ab`,
  deepseek-v4-pro `api_81a8758a56bb48738e80e0c6b00f5dad`) — all "revise".
- State-integrity invariants: root `CLAUDE.md` "State integrity invariants" (1–5).
