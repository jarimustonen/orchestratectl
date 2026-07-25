# TODO

Session-level plan + handoff. Longer-running planning/design docs live under
`issues/<slug>/{design,plan,breakdown,…}.md`; this file points at issuectl issues
for the actual tracked work.

---

## 🔄 Continue here (ALOITA TÄSTÄ) — 2026-07-25

**One-paragraph state.** The **code-pipeline** (spec→code→verify, model-tiered) is
the active workstream. Its **walking skeleton is LIVE and proven end-to-end**:
`orchestratectl pipeline run` took a real feature from intent to a merged, tested
result — **Opus** specced + verified, **claude-deepseek** wrote the code, the
**deterministic floor** gated every merge, and it auto-merged correct code
(factorial demo: all floor gates green, `Ran 2 tests — OK`). All foundation +
skeleton landed behind the seam / as an additive command; **default coding paths
are unchanged**. `main` is pushed. The v0.1.0 publish is **deferred** (maybe never;
may instead be merged into the `issuectl` tool).

**NEXT — do this first: option (a), `pipeline-fix-loop`.** Build the fix loop so the
pipeline is resilient when code doesn't land right the first time (verify/floor
failure → `RE_CODE_CHUNK` with findings; the T4 driver scaffold + typed primitives
already exist). This is what turns the happy-path walking skeleton into something
production-worthy. → `issuectl show pipeline-fix-loop`, then `/worktree-spinoff
pipeline-fix-loop` (or `/stint` and plan the round).

### Where the pipeline stands (all on `main`)
- **Foundation (behind the seam):** `CodeHarness` contract + adapters
  (aider / claude / claude-deepseek / pi) + timeout/cancellation; `plan.json` v2
  types + validator + checked-in JSON Schema; **deterministic floor** (baseline
  snapshot + gates: checks/regression/clippy/test-gaming/file-scope); **T4
  inverted-loop scaffold** (tiered orchestrator = fast coordinator + Opus decider,
  typed action primitives, decision envelopes).
- **Live:** `orchestratectl pipeline run --intent … --source-branch …` — additive
  command; spec[Opus] → code[claude-deepseek] → floor-gate → verify[Opus] → merge.
- **Bakeoff:** `orchestratectl harness bakeoff --brief <file>` compares the 4 agent
  loops. Measured 2026-07-24: **claude + claude-deepseek pass**; aider + pi failed
  their first run (adapter/config hardening needed only if a full 4-way compare is
  wanted).
- **Design of record:** `issues/code-pipeline/{design.md,plan-schema.md,breakdown.md}`
  — read `design.md` for the architecture + all owner decisions (bold-to-live,
  fast-coordinator/Opus-decider, floor as the guardrail).

### Deferred pipeline backlog (all filed)
`pipeline-fix-loop` ← NEXT (a) · `pipeline-tiered-triage` · `pipeline-circuit-breakers`
· `pipeline-parallel-chunks` · `pipeline-run-create-wiring` · `pipeline-hardening`
(incl. stale `$TMPDIR/octl-pipeline/<slug>` workdir handling — a known papercut) ·
`pipeline-drop-primitive-underspecified` · `floor-capture-trust-model`. Optional:
harden aider/pi adapters + capture tokens/cost in the bakeoff.

### How to resume
1. `git log --oneline -8` (pipeline commits on main); `git rev-list --count
   origin/main..main` → expect `0` (pushed).
2. Redeploy if you'll run it: `cargo install --path crates/octl-cli --force &&
   orchestratectl doctor`.
3. Try it live: seed a throwaway git repo, then
   `orchestratectl pipeline run --intent "…" --source-branch main --repo <dir>
   --test-cmd "python3 -m unittest discover -q" --clippy-cmd "true"
   --file-scope-slack 2`. Clean any stale `$TMPDIR/octl-pipeline/<slug>` first.
4. Start the fix-loop work (a).

---

## Adjacent open backlog (NOT this workstream — orchestratectl-core bugs)

Parallel `/stint` sessions filed a cluster of core supervisor/lifecycle bugs
(`supervisor-spawn-fails-silently-at-run-create`, `run-create-back-to-back-no-supervisor`,
`headless-spawn-tmux-window-race`, `merge-skips-teardown`,
`landing-signal-reliable-after-rebase`, `worker-process-hang`,
`agent-died-merge-no-teardown-interactive`, `orchestrate-integration-branch-no-worktree-merge-fails`,
`reattach-does-not-bootstrap-crashed-at-creation-run`, `notify-run-level-summary`,
`no-completion-notification-to-parent`) plus the earlier v0.2 carry-overs
(`cancel-dead-supervisor-recovery`, `legacy-pid-identity-check`,
`teardown-gate-trust-and-lifecycle`, `vendor-workmux-multiplexer`,
`workmux-extract-libs`). These harden the very spawn/merge machinery the pipeline
rides on — worth a triage pass, but separate from the pipeline build. `issuectl ls
--status open` for the live list.

---

## Invariants + operating policy

The 5 state-integrity invariants and the `/stint` operating policy (deploy /
green-gate / hot files) live in the root `CLAUDE.md` / `AGENTS.md`. Read them before
touching the reducer, lock layer, `supervise/`, or the pipeline modules
(`harness/`, `floor/`, `pipeline/`).
