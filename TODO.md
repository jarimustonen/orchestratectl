# TODO

Session-level plan + handoff. Longer-running planning/design docs live under
`issues/<slug>/{design,plan,breakdown,…}.md`; this file points at issuectl issues
for the actual tracked work.

---

## 🔄 Continue here (ALOITA TÄSTÄ) — 2026-07-27 (after the T6 + resilience waves)

**One-paragraph state.** Four `/stint` waves this session took the **code-pipeline
through the full T6 layer** (fix-loop + tiered-triage + circuit-breakers all LIVE)
AND built an **agent-agnostic worker-resilience layer**. The pipeline now: recovers on
floor/verify failure (`RE_CODE_CHUNK` + `TRIGGER_RE_SPEC`, bounded), routes decisions
through a fast coordinator with only consequential ones hitting the Opus decider +
adaptive `PROMOTE_TIER`, and is bounded by deterministic supervisor-owned
circuit-breakers (cost/token tally, wall-time, process-count, storage,
repeated-failure). On the reliability side the whole creation→run→merge→teardown path
is hardened (readiness-pipe spawn confirmation, child-supervisor state machine +
bounded retry, reducer adopts late explicit-merge, interactive watchdog false-positive
fixed) and — the headline — **intermittent worker deaths now self-recover**: an
empty-handed `agent-died` **auto-retries** (bounded), a death that committed clean work
is **flagged recoverable**. All green, rebuilt locally, `doctor` 0 fail / 0 warn. v0.1.0
publish still **deferred**.

**KEY LEARNING — worker deaths are TRANSIENT, not deterministic.** The
`pipeline-tiered-triage` agent died `agent-died` at ~13 min twice, then a third
identical spawn ran ~54 min and landed. Autonomous `agent-died` = a GENUINE claude-pid
exit (not a watchdog false-positive; autonomous is correctly pid-authoritative). The
answer was resilience (auto-retry + salvage), NOT switching base agents — a Codex ADR
was considered and **shelved** (revisit only if deaths become frequent). Heavy-LLM
worker units legitimately take **54–96 min**; don't mistake a long run for a hang.

**NEXT — execute the DAG below.** The three earlier options (a diagnostic-gap fix,
b pipeline tail, c reliability remainders) are resolved into the dependency DAG in
the next section. Start at the heads; the `GLOBAL HEAD-OF-LINE` is
**`capture-agent-pane-by-pane-id`** (Lane A). Pick the next unit by recomputing the
head-of-line from live `issuectl` status — the DAG stores the plan, not status.

---

## Execution DAG (2026-07-27)

Scheduling PLAN — source of truth for lane + order; **issuectl is authoritative for
STATUS** (never copied here). Lanes = hot-file families; within a lane ≤1 live worktree at
a time, in the listed order; across lanes heads run in parallel unless they share a
`collision:` file. **Merge** this at Phase 0/7 (drop landed, add active, keep existing
order) — don't regenerate. `▶` = head-of-line snapshot — **re-compute from issuectl at
pick time** (`open`/`in-progress`, not `deferred`, deps `fixed`/`done`). `after <slug>
(needs …)` = logical `blocked_by` mirror. `collision: <file>` = touches another lane's hot
file (spawn-time exclusion). `[wip]` = a worktree currently has it (don't spawn again).
Convention: `crates/octl-cli/skills/stint/SKILL.template.md` → *Execution DAG*.

<!-- execution-dag:begin -->
```
GLOBAL HEAD-OF-LINE: supervisor-spawn-fails-silently-at-run-create (Lane A, high)   ← start here on resume

LANE A — supervise/* + reducer/schema (create.sh, run/spawn.rs, capture.rs)
    worker-process-hang                      (in-progress; now unblocked — capture landed; but WHY pid exits is agent-runtime scope)
    supervisor-spawn-fails-silently-at-run-create   (high; #4 stateful load-trigger only — no repro, investigative; stepped past for a cleaner repro-able high)
  ▶ interactive-code-run-self-merged [wip]          (high; concrete repro — interactive run self-merged past the review gate)
    agent-skips-run-merge-idle-pending              (high; supervisor safety-net + reducer)
    child-supervisor-spawn-exhaustion-lifecycle
    run-create-back-to-back-no-supervisor
    reattach-does-not-bootstrap-crashed-at-creation-run
    autoretry-crash-consistency
    cancel-dead-supervisor-recovery
    legacy-pid-identity-check
    teardown-gate-trust-and-lifecycle
    no-completion-notification-to-parent
    notify-run-level-summary

LANE B — pipeline/* + floor/* + harness/*
  ▶ floor-capture-trust-model [wip]          (high; floor/, disjoint from A)
    pipeline-fix-loop-provenance
    pipeline-parallel-chunks                 (DAG scheduler)
    pipeline-hardening
    pipeline-run-create-wiring               collision: create.sh   (shares w/ Lane A capture)
    pipeline-breaker-inflight-and-opus-metering
    pipeline-drop-primitive-underspecified
    pipeline-tiered-triage                   (in-progress; deferred self-disagreement trigger)

LANE C — workmux vendoring (fully independent)
  ▶ vendor-workmux-multiplexer
    workmux-extract-libs   after vendor-workmux-multiplexer (needs vendored tree)

LANE D — workflow/skill (skill prose, not product code)
    (empty — stint-maintains-execution-dag landed 2026-07-27)

LANE E — run/* CLI surface (touch run/*, not supervise core; lower collision, still sequence)
  ▶ idempotency-key-allowed-duplicate-run
    landing-signal-reliable-after-rebase
    run-cancel-accept-unambiguous-prefix
    run-wait-timeout-unit-required
    run-salvage-command
    orchestrate-integration-branch-no-worktree-merge-fails
```
<!-- execution-dag:end -->

**Epic (not a lane node):** `code-pipeline` — parent of the Lane B `pipeline-*` work.
**Adjacent backlog / deferred:** none currently parked; the full open list is
`issuectl ls --status open`.

**Parallelism rule of thumb:** ≤1 live worktree per lane. Cross-lane, several heads can run
at once — e.g. Lane A + B + C heads — except a head carrying `collision: <file>` must not
spawn while another worktree touching that file is live. **Migrated 2026-07-27** to the
issue-derived DAG convention (slug identity, `collision:` tags, `[wip]` from issuectl).
**Reconciled 2026-07-27 (this stint):** dropped landed `capture-agent-pane-by-pane-id`
(fixed) + `stint-maintains-execution-dag` (done); closed the stale-status
`agent-died-merge-no-teardown-interactive` (git-verified landed, issuectl never closed)
and dropped it; cleared dead `[wip]` tags (no live worktrees).

### What landed this session (all on `main`, green, deployed — `doctor` 0/0)
- **Pipeline T6 complete:** `pipeline-fix-loop` ✅, `pipeline-tiered-triage` ✅ (in-progress:
  one deferred trigger), `pipeline-circuit-breakers` ✅ (+ pi 0.82 `--mode json` cost-column
  fix folded in).
- **Creation/spawn:** `supervisor-confirm-readiness-pipe` ✅, `headless-spawn-tmux-window-race` ✅,
  `child-supervisor-spawn-unconfirmed-no-retry` ✅ (pid-0 state machine + bounded retry);
  `supervisor-spawn-fails-silently` guards ✅ (partial).
- **Merge/teardown:** `merge-skips-teardown` ✅, `reducer-adopt-explicit-merge` ✅.
- **Watchdog/death resilience:** `agent-died-merge-no-teardown-interactive` ✅ (interactive
  liveness = tmux-window authoritative), `agent-death-strands-recoverable-work` ✅
  (recoverability signal), `autoretry-agent-died-worker` ✅ (bounded auto-retry),
  `capture-agent-output-to-run-dir` ✅ landed (**but capture ineffective — see NEXT (a)**).
- **Adapters + hygiene (prev wave):** `bakeoff-aider-pi-live-fail` ✅, `notify-test-toctou-flake` ✅.

### Reliability remainders still open
- `supervisor-spawn-fails-silently-at-run-create` (high) — **#4 stateful load-trigger only**
  (confirmation ambiguity cured; fails loudly now).
- `run-create-back-to-back-no-supervisor` — no code race isolated.
- `agent-skips-run-merge-idle-pending` — agent ends session without calling `run merge`; run
  stuck `pending`, work committed-unmerged (a supervisor safety-net is the fix).
- `child-supervisor-spawn-exhaustion-lifecycle` — after CHILD_SPAWN_MAX_ATTEMPTS the child tail
  stays open → parent polls forever; needs a terminal-child event.
- `worker-process-hang` (in-progress) — WHY the claude pid exits is agent-runtime, out of
  orchestratectl scope; capture (NEXT a) is the diagnosis path.
- `capture-agent-output-to-run-dir` pane-id follow-up, `run-salvage-command` (option 2 of salvage).

### Design of record + how to resume
- Pipeline design: `issues/code-pipeline/{design.md,plan-schema.md,breakdown.md}`.
- `git log --oneline -20`; `git rev-list --count origin/main..main` (human pushes; ~25 unpushed).
- Redeploy if changing CLI/skills: `cargo install --path crates/octl-cli --force &&
  orchestratectl skill install --force && orchestratectl doctor` (expect 0 fail/0 warn).
- **Integrated gate** (root AGENTS.md): after a multi-worktree round re-run
  `cargo test --workspace` on integrated `main` before deploy — per-worktree green ≠
  integrated green.
- Try the pipeline live: seed a throwaway git repo, then `orchestratectl pipeline run
  --intent "…" --source-branch main --repo <dir> --test-cmd "python3 -m unittest
  discover -q" --clippy-cmd "true" --file-scope-slack 2`. Clean any stale
  `$TMPDIR/octl-pipeline/<slug>` first.

---

## Adjacent open backlog (NOT this workstream — orchestratectl-core bugs)

The supervisor/lifecycle reliability cluster is now **largely cured** across the
2026-07-25→27 stints (creation-path, teardown, child-supervisor, interactive watchdog,
recoverability + bounded auto-retry all ✅ — see the Continue-here block). **Still open**
on the core side: `supervisor-spawn-fails-silently-at-run-create` (#4 load-trigger only),
`run-create-back-to-back-no-supervisor`, `agent-skips-run-merge-idle-pending`,
`child-supervisor-spawn-exhaustion-lifecycle`, `worker-process-hang` (in-progress),
`capture-agent-output-to-run-dir` (pane-id follow-up),
`landing-signal-reliable-after-rebase`,
`orchestrate-integration-branch-no-worktree-merge-fails`,
`reattach-does-not-bootstrap-crashed-at-creation-run`, `notify-run-level-summary`,
`no-completion-notification-to-parent`, plus the v0.2 carry-overs
(`cancel-dead-supervisor-recovery`, `legacy-pid-identity-check`,
`teardown-gate-trust-and-lifecycle`, `vendor-workmux-multiplexer`,
`workmux-extract-libs`). `issuectl ls --status open` for the live list.

---

## Invariants + operating policy

The 5 state-integrity invariants and the `/stint` operating policy (deploy /
green-gate / hot files) live in the root `CLAUDE.md` / `AGENTS.md`. Read them before
touching the reducer, lock layer, `supervise/`, or the pipeline modules
(`harness/`, `floor/`, `pipeline/`).
