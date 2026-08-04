# TODO

Session-level plan + handoff. Longer-running planning/design docs live under
`issues/<slug>/{design,plan,breakdown,…}.md`; this file points at issuectl issues
for the actual tracked work.

---

## 🔄 Continue here (ALOITA TÄSTÄ) — 2026-07-31 (DAG-driven stint; 6 units landed)

**One-paragraph state.** This `/stint` session made the **execution DAG a first-class,
self-maintaining artifact** and then drained three waves through it — **6 units landed on
`main`, all green, integrated-gate verified, `doctor` 0/0**: `capture-agent-pane-by-pane-id`
(agent.log now captured by stable `pane_id`), **`stint-maintains-execution-dag`** (the DAG
convention itself — `/stint` now maintains an issue-derived DAG in this file: DAG owns the
plan, issuectl owns status, stateful-merge self-heal across phases; design in
`issues/stint-maintains-execution-dag/design.md`), `interactive-code-run-self-merged`
(interactive `code` runs gated behind `--confirm-interactive`, can no longer self-merge past
the human review gate), `floor-capture-trust-model` + `floor-capture-hardening-round-2`
(floor evidence capture is now structured-JSON, injection-resistant, fail-closed,
target-qualified, OID-provenance-bound + repo-config-neutralized), and
`agent-skips-run-merge-idle-pending` (supervisor safety-net: an autonomous run that committed
but skipped `run merge` and went idle now terminalizes to recoverable-failed within a bounded
time; interactive exempt). v0.1.0 publish still **deferred**.

**KEY LEARNING (reaffirmed live this session) — worker deaths are TRANSIENT.** The
`agent-skips-run-merge-idle-pending` worker died `agent-died` after committing a 523-line
first cut; the recoverability signal preserved the branch, and a **retry that harvested the
first cut** reviewed + completed + landed it. Retry (with harvest), NOT hand-merge of
unreviewed work, NOT base-agent swap. Heavy-LLM units legitimately take **54–96 min**; don't
mistake a long run for a hang. (Earlier precedent: `pipeline-tiered-triage` died twice at
~13 min, third spawn ran ~54 min and landed.)

**NEXT — execute the DAG below.** `GLOBAL HEAD-OF-LINE` is **`floor-capture-hardening-round-3`**
(Lane B, high — Lane A's remaining heads are investigative/normal). Recompute the actual
head at pick time from live `issuectl` status (`open`/`in-progress`, deps `fixed`/`done`) —
the DAG stores the plan + lane/collision ordering, NOT status. Merge the DAG at Phase 0/7
(drop landed, add active, keep order) per the convention in
`crates/octl-cli/skills/stint/SKILL.template.md`.

The superseded orphan worktree/branch `wt/01kym6a7bz-idle-pending-safetynet` from the
`agent-skips-run-merge-idle-pending` death has been **removed** (2026-07-31; its work was
harvested + landed via the retry, and its uncommitted test fragment was verified subsumed by
main's more comprehensive coverage). No worktrees remain. `main` was pushed to `origin`.

---

## Execution DAG (2026-08-01)

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
GLOBAL HEAD-OF-LINE: supervisor-spawn-fails-silently-at-run-create (Lane A, only remaining high — but investigative/no-repro; practical actionable heads are Lane B/C/D/E below)   ← start here on resume

LANE A — supervise/* + reducer/schema (create.sh, run/spawn.rs, capture.rs)
    worker-process-hang                      (in-progress; now unblocked — capture landed; but WHY pid exits is agent-runtime scope)
    supervisor-spawn-fails-silently-at-run-create   (high; #4 stateful load-trigger only — no repro, investigative)
    idle-empty-handed-alive-agent-hangs             (follow-up of idle-unmerged net — empty-handed alive-agent variant)
    watchdog-tick-verdict-refactor                  (follow-up of idle-unmerged net — watchdog tick refactor)
    watchdog-pane-aware-liveness                    (follow-up of A1 pane_id capture)
    code-run-inject-no-selfmerge                    (follow-up of interactive-code — code-inject the no-self-merge rule)
    interactive-merge-audit-marker                  (follow-up of interactive-code — audit marker for human-confirmed merge)
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
  ▶ pipeline-fix-loop-rollback-hardening     (fix-loop-provenance follow-up: transactional/audit hardening of the deferred review items)
    pipeline-parallel-chunks                 (DAG scheduler)
    pipeline-hardening
    pipeline-run-create-wiring               collision: create.sh   (shares w/ Lane A capture)
    pipeline-breaker-inflight-and-opus-metering
    pipeline-drop-primitive-underspecified
    pipeline-tiered-triage                   (in-progress; deferred self-disagreement trigger)

LANE C — workmux vendoring (fully independent)
  ▶ workmux-extract-libs   (now unblocked — vendored tree landed via vendor-workmux-multiplexer)

LANE D — workflow/skill (skill prose + skill registry; sequence, touches bundled-skill catalog)
  ▶ skill-install-prune-deregistered       (skill.rs: skill install leaves de-registered bundled skills stranded in ~/.claude/skills — needs provenance-safe prune or doctor orphan check)
    doctor-skill-companion-sync            (skill.rs: doctor skill.sync should also verify companion resource files like AGENTS-EXECUTION-DAG.md, not just SKILL.md)
    skill-companion-codex-layout           (skill.rs: companion resources are claude-only; codex flat layout unsupported — both filed by the split-stint worker)

LANE E — run/* CLI surface (touch run/*, not supervise core; lower collision, still sequence)
  ▶ landing-signal-reliable-after-rebase
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
**Reconciled again 2026-07-28:** dropped landed `interactive-code-run-self-merged` (fixed)
+ `floor-capture-trust-model` (done); inserted follow-ups filed by those workers —
`floor-capture-hardening-round-2` (Lane B head), `code-run-inject-no-selfmerge` +
`interactive-merge-audit-marker` + `watchdog-pane-aware-liveness` (Lane A),
`triage-bugs-stint-inprogress-ownership-conflict` (Lane D head).
**Wave 3 reconcile 2026-07-28:** landed `floor-capture-hardening-round-2` (→ round-3 filed,
Lane B head) + `agent-skips-run-merge-idle-pending` (fixed; 1st attempt agent-died, retry
harvested the recoverable first cut and landed it). New Lane A follow-ups filed by the retry:
`idle-empty-handed-alive-agent-hangs`, `watchdog-tick-verdict-refactor`.
**Orphan cleaned 2026-07-31:** the superseded dead worktree/branch
`wt/01kym6a7bz-idle-pending-safetynet` was removed (work landed via the retry; uncommitted
test fragment verified subsumed by main's coverage). No worktrees remain.

### What landed in the PRIOR (T6 + resilience) session — historical reference (all on `main`, green, `doctor` 0/0)
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
