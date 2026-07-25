# TODO

Session-level plan + handoff. Longer-running planning/design docs live under
`issues/<slug>/{design,plan,breakdown,…}.md`; this file points at issuectl issues
for the actual tracked work.

---

## 🔄 Continue here (ALOITA TÄSTÄ) — 2026-07-25 (after the reliability stint)

**One-paragraph state.** A two-round `/stint` closed the **supervisor/lifecycle
reliability cluster** and advanced the **code-pipeline** by one step. The pipeline
**fix loop is now LIVE** (`pipeline run` recovers on floor/verify failure via
`RE_CODE_CHUNK` + `TRIGGER_RE_SPEC`, bounded by a hard iteration cap) on top of the
already-proven walking skeleton. On the reliability side: `run create` now fails
loudly (readiness-pipe confirmation via daemonization double-fork, no more pid-file
poll ambiguity / orphan window), always writes `supervisor.stderr.log`, never
false-reports `work-complete` with no child; the headless tmux-window race is fixed;
`run merge` teardown is fixed at the source (reducer now **adopts** a late
`explicit-merge` report even against a terminal node → supervisor is the sole
teardown actor again, inline CLI reclaim removed); and both aider + pi harness
adapters are live-verified so a full 4-way bakeoff runs. All green, rebuilt locally,
`doctor` 0 fail / 0 warn. The v0.1.0 publish is still **deferred** (maybe merged into
`issuectl` instead).

**NEXT — pick one:**
- **(a) Back to the pipeline (recommended).** With the fix loop landed, the next
  pipeline steps are `pipeline-tiered-triage` (adaptive tier promotion + tiered
  fast-coordinator) and `pipeline-circuit-breakers` (cost/token metering + richer
  breakers — this also absorbs the cosmetic **pi bakeoff cost-column shows `-`**
  gap, pi 0.82 `--mode json` usage shape). New follow-up `pipeline-fix-loop-provenance`
  (provenance-aware chunk rollback) is also filed. → `/stint` or `/worktree-spinoff
  pipeline-tiered-triage`.
- **(b) Finish the last reliability remainders.** The cluster is **safe** but not
  100% cured — see "Reliability remainders still open" below. Highest-value: the
  `supervisor-spawn-fails-silently` **#4 stateful load-trigger** (only the
  confirmation ambiguity was cured this session) and the NEW
  `child-supervisor-spawn-unconfirmed-no-retry` (the child-supervisor analogue of the
  readiness fix — records pid 0 as success, no retry).

### What landed this session (all on `main`, green, deployed)
- **Pipeline fix loop** (`pipeline-fix-loop` ✅ done): `RE_CODE_CHUNK` + `TRIGGER_RE_SPEC`
  + hard iteration bound, reusing the T4 driver primitives; tested.
- **Creation-path reliability**: fail-loud `supervisor_spawn_failed` envelope,
  always-write `supervisor.stderr.log`, no false `work-complete` w/ empty children,
  idempotency-before-spawn (`supervisor-spawn-fails-silently` partial — guards landed);
  **readiness pipe** replaces the pid-file poll (`supervisor-confirm-readiness-pipe` ✅);
  headless tmux-window race (`headless-spawn-tmux-window-race` ✅).
- **Merge teardown**: inline reclaim shipped, then the proper fix —
  `reducer-adopt-explicit-merge` ✅ (reducer adopts late explicit-merge, supervisor sole
  teardown actor, `append_and_apply_*` reports applied-vs-noop, unmerged-work
  preservation gates kept). `merge-skips-teardown` ✅.
- **Bakeoff adapters** (`bakeoff-aider-pi-live-fail` ✅): pi `--` terminator dropped,
  aider commits its leftovers; both live-verified (aider 0.86.2, pi 0.82.0).
- **Test hygiene**: `notify-test-toctou-flake` ✅ (integration-surfaced order-dependent
  flake in `fires_hook_with_completion_env`; poll-for-content fix).

### Reliability remainders still open (safe, not cured)
- `supervisor-spawn-fails-silently-at-run-create` (high) — **#4 stateful load-trigger
  only**; confirmation ambiguity now cured by the readiness pipe. Fails loudly now.
- `run-create-back-to-back-no-supervisor` — no code race isolated; readiness-pipe
  worktree recorded findings in the issue.
- `child-supervisor-spawn-unconfirmed-no-retry` (NEW this session) — child-supervisor
  spawn records pid 0 as success and never retries; the child analogue of the readiness fix.
- `agent-died-merge-no-teardown-interactive` — the watchdog agent-died FALSE POSITIVE
  on long-lived interactive runs is the upstream trigger behind the swallowed-report cases.

### Design of record + how to resume
- Pipeline design: `issues/code-pipeline/{design.md,plan-schema.md,breakdown.md}`.
- `git log --oneline -12`; `git rev-list --count origin/main..main` (human pushes).
- Redeploy if changing CLI/skills: `cargo install --path crates/octl-cli --force &&
  orchestratectl skill install --force && orchestratectl doctor` (expect 0 fail/0 warn).
- Try the pipeline live: seed a throwaway git repo, then `orchestratectl pipeline run
  --intent "…" --source-branch main --repo <dir> --test-cmd "python3 -m unittest
  discover -q" --clippy-cmd "true" --file-scope-slack 2`. Clean any stale
  `$TMPDIR/octl-pipeline/<slug>` first.

---

## Adjacent open backlog (NOT this workstream — orchestratectl-core bugs)

The supervisor/lifecycle reliability cluster was largely closed by the 2026-07-25
stint (see the Continue-here block: `headless-spawn-tmux-window-race`,
`merge-skips-teardown`, `supervisor-confirm-readiness-pipe`,
`reducer-adopt-explicit-merge`, `bakeoff-aider-pi-live-fail`,
`notify-test-toctou-flake` all ✅). **Still open** on the core side:
`supervisor-spawn-fails-silently-at-run-create` (#4 load-trigger only),
`run-create-back-to-back-no-supervisor`, `child-supervisor-spawn-unconfirmed-no-retry`
(new), `agent-died-merge-no-teardown-interactive`,
`landing-signal-reliable-after-rebase`, `worker-process-hang`,
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
