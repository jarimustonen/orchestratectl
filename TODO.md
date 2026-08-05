# TODO

Session-level plan + handoff. Longer-running planning/design docs live under
`issues/<slug>/{design,plan,breakdown,…}.md`; this file points at issuectl issues
for the actual tracked work.

---

## 🔄 Continue here (ALOITA TÄSTÄ) — 2026-08-05 (v0.1.0 SHIPPED + operating-policy change)

**One-paragraph state.** This was a **release + policy session** (no DAG round, no worktrees spawned).
Headline: **`orchestratectl` 0.1.0 is PUBLISHED** — crates.io (`octl-core` + `orchestratectl`), a GitHub
Release `v0.1.0` (aarch64-mac + x86_64/aarch64-linux binaries + installer), and a **per-tool Homebrew
tap**: `brew install jarimustonen/orchestratectl/orchestratectl` (verified working). Distribution now
matches issuectl/ossctl; homebase's dotfiles install hook was migrated to the per-tool tap (committed +
pushed). Along the way: generated + approved `OSS-RELEASE.md`; finalized `CHANGELOG.md` (folded
`[Unreleased]` into `[0.1.0] - 2026-08-04`); and fixed a release-blocking runner bug — the `hauis` mac
build failed 3× at `actions/checkout` because the runner's `~/actions-runner/.env` set
`GIT_CONFIG_GLOBAL`; removed it + documented the gotcha in `dist-workspace.toml` (issue
`release-mac-checkout-git-config-global`, done). (The prior stint's 13-unit / 4-round work is in git
history + the DAG reconcile notes below.)

**OPERATING-POLICY CHANGE (2026-08-05, canonical — see root `AGENTS.md`).** (1) **Release often** — cut a
release whenever something production-ready lands; don't batch. (2) **Pushing `main` is now always
allowed (no ask)**, deliberately overriding the global "never push without being asked" default for this
repo; the `pull → rebase → push` sequence can be run anytime. Only the two irreversible/public release
steps stay behind the `/oss-release` approval boundary: `cargo publish` to crates.io and pushing a
`vX.Y.Z` release tag (fires the public binary + Homebrew CI release).

**KEY LEARNING (from prior stints, still canonical) — worker deaths are TRANSIENT.** Retry
**with harvest** of the recoverable preserved branch (review → adopt → complete → merge), NOT
hand-merge of unreviewed work, NOT base-agent swap. Heavy-LLM units legitimately take **54–96
min**; a long run is not a hang. (This tactic is now encoded in `/stint-start` Phase 3 via
`stint-recoverable-death-retry-harvest`.) NOTE: every worker this session landed cleanly on the
first spawn — no deaths — but the discipline holds.

**Cross-repo this session:** homebase dotfiles hook migrated to the per-tool tap
(`dotfiles/{setup.d/orchestratectl.sh, setup.d/brew-trust.sh, src/brew-packages.txt}`) — committed +
pushed on homebase `main`. Two `ossctl` release-engine bugs filed + committed in **~/Sources/ossctl**
(`release-list-abandon-not-implemented`, `release-cut-multi-target-ecosystem`; unpushed there — ossctl
push is the human's call, that repo keeps the global default).

**NEXT — resume with `/stint-start`, execute the DAG below.** `GLOBAL HEAD-OF-LINE` is
**`supervisor-spawn-fails-silently-at-run-create`** (Lane A, only remaining high — but
investigative/no-repro; #4 stateful load-trigger only). Practical *actionable* heads, all
disjoint and parallel-safe: Lane B `pipeline-parallel-chunks`, Lane C `workmux-extract-libs`
(**reassess scope first** — the multiplexer slice already landed via `vendor-workmux-multiplexer`;
this may be a re-scope-or-close), Lane D `doctor-skill-companion-sync`, Lane E
`landing-signal-reliable-after-rebase` (**carries `collision: bundled-skill snapshot`** — it edits
stint-start + worktree-spinoff templates, so do NOT run it parallel with a Lane D worktree, now
including the new `spinoff-skill-stale-preview-banner` which also touches the bundled-skill snapshot).
Recompute the head at pick time from live `issuectl` status; merge the DAG at Phase 0/7 per
`crates/octl-cli/skills/stint-start/AGENTS-EXECUTION-DAG.md`. No worktrees remain; **`main` clean and
pushed (0 unpushed)** — pushing is now always allowed (no ask, see the policy change above).

---

## Execution DAG (2026-08-05)

Scheduling PLAN — source of truth for lane + order; **issuectl is authoritative for
STATUS** (never copied here). Lanes = hot-file families; within a lane ≤1 live worktree at
a time, in the listed order; across lanes heads run in parallel unless they share a
`collision:` file. **Merge** this at Phase 0/7 (drop landed, add active, keep existing
order) — don't regenerate. `▶` = head-of-line snapshot — **re-compute from issuectl at
pick time** (`open`/`in-progress`, not `deferred`, deps `fixed`/`done`). `after <slug>
(needs …)` = logical `blocked_by` mirror. `collision: <file>` = touches another lane's hot
file (spawn-time exclusion). `[wip]` = a worktree currently has it (don't spawn again).
Convention: `crates/octl-cli/skills/stint-start/AGENTS-EXECUTION-DAG.md` (shared reference
`/stint-handoff` also links to; the old monolith `/stint` skill was split 2026-08-04).

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
  ▶ pipeline-parallel-chunks                 (DAG scheduler)
    pipeline-provenance-durable-refs         (fix-loop-rollback follow-up: durable per-chunk provenance refs)
    pipeline-hardening
    pipeline-run-create-wiring               collision: create.sh   (shares w/ Lane A capture)
    pipeline-breaker-inflight-and-opus-metering
    pipeline-drop-primitive-underspecified
    pipeline-tiered-triage                   (in-progress; deferred self-disagreement trigger)

LANE C — workmux vendoring (fully independent)
  ▶ workmux-extract-libs   (now unblocked — vendored tree landed via vendor-workmux-multiplexer)

LANE D — workflow/skill (skill prose + skill registry; sequence, touches bundled-skill catalog)
  ▶ doctor-skill-companion-sync            (skill.rs: doctor skill.sync should also verify companion resource files like AGENTS-EXECUTION-DAG.md, not just SKILL.md)
    skill-companion-codex-layout           (skill.rs: companion resources are claude-only; codex flat layout unsupported — both filed by the split-stint worker)
    skill-install-force-symlink            (skill.rs: install --force aborts on a pre-existing symlink — refused_overwrite; prune/handle the stale symlink first)
    spinoff-skill-stale-preview-banner     collision: bundled-skill snapshot (octl-spawn-spinoff SKILL.md still carries a "NOT IMPLEMENTED" preview banner — prose fix)

LANE E — run/* CLI surface (touch run/*, not supervise core; lower collision, still sequence)
  ▶ landing-signal-reliable-after-rebase   collision: bundled-skill snapshot (edits stint-start + worktree-spinoff templates → sequence vs Lane D)
    cancel-run-already-terminal-error-class  (run cancel on a terminal run: distinct error class; run-cancel-prefix follow-up)
    run-paths-typed-selector-split           (typed run-id selector split-out; run-cancel-prefix follow-up)
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
**Reconciled 2026-08-04 (4-round split stint):** dropped 13 landed slugs across 4 rounds
(R1 floor-r3/vendor-workmux/idempotency/triage-ownership · R2 selfmerge-race/fixloop-provenance/
retry-harvest · R3 merge-terminal/plan-schema-v3 · R4 skill-prune/pipeline-rollback/run-cancel-prefix)
plus the `split-stint-start-handoff` refactor (/stint → /stint-start + /stint-handoff). Added the
worker-filed follow-ups: `plan-schema-v3-provenance-required` (landed), `pipeline-fix-loop-rollback-hardening`
(landed), `pipeline-provenance-durable-refs`, `skill-install-prune-deregistered` (landed),
`doctor-skill-companion-sync`, `skill-companion-codex-layout`, `cancel-run-already-terminal-error-class`,
`run-paths-typed-selector-split`. Lane D (skill machinery) refilled; DAG driftless at wrap.
**Reconciled 2026-08-05 (release/policy session — no DAG round):** no lanes executed; `comm -3`
drift check found 2 left-only, 0 right-only → added the two newly-filed skill bugs to Lane D
(`skill-install-force-symlink`, `spinoff-skill-stale-preview-banner`), no drops; date refreshed.
Headline non-DAG work: v0.1.0 shipped (crates.io + per-tool Homebrew tap) and the operating-policy
change (release-often; `main`-push now always allowed).

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
