# TODO

Session-level plan + handoff. Longer-running planning/design docs live under
`issues/<slug>/{design,plan,breakdown,…}.md`; this file points at issuectl issues
for the actual tracked work.

---

## 🔄 Continue here (ALOITA TÄSTÄ) — 2026-08-10 (one round + v0.1.4 FULLY SHIPPED; CI was red-for-days → now green)

**✅ LATEST (2026-08-10 session — read first).** One `/stint-start` round: **8 units landed on `main`, all
reviewed+green, all self-merged (no worker deaths).** The planned 3 were `run-wait-stillborn-run-not-detected`
✅ (fixed; `run wait`/`run show` flag a stillborn run — dead supervisor, 0 nodes — as `stalled` and return
promptly instead of blocking the full timeout), `pipeline-hard-failure-carries-report` ✅ (done; hard
`PipelineError` carries a report so `cmd_run` exits non-zero AND surfaces the `branch_preserved` inv-5 audit),
and `run-show-json-null-fields` ✅ (fixed; `run show --output json` no longer all-null — supervisor lifted to
`.data` top level, run-list row flattened in). **Then a CI-health thread became the substantive work:** a
worker's mail-sweep caught that **`main`'s CI had been RED for several days** and nobody had noticed →
`ci-red-main-deny-docs` ✅ (fixed; repaired `octl-core`/`octl-cli` broken intra-doc links + the RUSTSEC-2026-0009
`cargo-deny` advisory). Fixing that surfaced two more: **a REAL macOS bug** `merge-lock-flock-not-portable-macos`
✅ (fixed; `merge.sh` used `flock` which is absent on stock macOS — the primary platform — so the worktree-merge
lock was silently broken; replaced with a portable atomic `mkdir` mutex, same 600s/serialization semantics), and
an **MSRV-vs-advisory conflict** (below). Plus `dry-run-projection-parity-flake` ✅ (fixed; a subtle
inode-recycle write-detection bug in the test — atomic temp+rename frees an inode CI's busy FS recycles, so an
inode-only diff missed a real `manifest.json` rewrite → fingerprint by `(inode, mtime, size)`; 100/100 green at
16 threads; reducer itself confirmed correct) and `workmux-extract-libs` ✅ (done; user-approved — vendored a
typed `crates/octl-cli/src/git/` wrapper, `run/merge.rs` + `supervise/cleanup.rs` production git calls routed
through it, all 5 invariants preserved). **`orchestratectl` 0.1.4 is now FULLY SHIPPED across all three
channels:** crates.io (`octl-core` then `orchestratectl`), GitHub Release **v0.1.4** (aarch64-mac +
x86_64/aarch64-linux binaries + installer), Homebrew tap at **0.1.4**
(`brew upgrade jarimustonen/orchestratectl/orchestratectl`). Release workflow all-green (no `hauis` git-400
recurrence). `main` clean, 0 unpushed, `v0.1.4` tagged, no worktrees remain, local binary **0.1.4** (`doctor`
0 fail / 0 warn, 687 ok). crates.io token was present this round (no SOPS re-pull needed).

**KEY LEARNING #1 (canonical) — RUSTSEC-2026-0009 vs MSRV 1.85 is a standing conflict.** The `time` crate's
stack-exhaustion DoS advisory is fixed only in `time ≥0.3.47`, but **every `time ≥0.3.47` requires rustc 1.88**
> our 1.85 MSRV floor. `time` is transitive-only (via `tracing-appender`, log-rotation timestamps — we never
parse untrusted time input, so the advisory is **not exploitable** here). Resolution: **pinned `time` to
`0.3.41`** (keeps MSRV 1.85) **+ a scoped, time-boxed `deny.toml` ignore** of RUSTSEC-2026-0009 documenting the
rationale. **Re-evaluate the ignore if/when MSRV moves to ≥1.88** (then unpin `time` and drop the ignore).
Corollary: bumping a dep to clear a `cargo-deny` advisory can silently blow the MSRV — always re-check the
`msrv (1.85)` job, don't just look at ubuntu.

**KEY LEARNING #2 (NEW, canonical) — parallel spinoff waves under saturation kill supervisors before the
worker node.** Filed + reproduced **3× today** (in a *3dbear-monorepo* parallel `/stint` wave, not this repo):
`supervisor-dies-before-worker-node` — under heavy FS/CPU contention (multiple live supervisors, `git index.lock`
races, `git worktree remove` + `run list` hitting the 120s timeout) the per-run supervisor dies *before* creating
node `n-0001`, leaving the run `pending`/`stalled=true`, `node_count=0`, a base-main worktree with **0 commits**.
**Re-spawn does NOT help if the load persists** (dies again immediately); cleanup itself can wedge (worktree-remove
timeout → dir stranded, needed manual `rm -rf`). Mitigations proposed in the issue: (a) supervisor retry/backoff on
worker-node creation, (b) `run create` fail-fast on a non-alive supervisor (no pending-lie), (c) `run create`
backpressure/queue when N supervisors already live. **Validation bonus:** this round's shipped stillborn-detection
fix is exactly what surfaced these as `stalled=true` (previously they'd have blocked the full timeout). Sibling:
`run-wait-still` (the `node_count>0`, supervisor-died-*mid*-run variant still blocks — my fix only covered
`node_count==0`). Both Lane A/E; see DAG.

**KEY LEARNING #3 (still canonical) — worker deaths are TRANSIENT.** Retry **with harvest** of the recoverable
preserved branch (review → adopt → complete → merge), NOT hand-merge of unreviewed work, NOT base-agent swap.
Heavy-LLM units legitimately take **54–96 min**; a long run is not a hang. (This round: all 8 units landed on
first spawn, no deaths.)

**RELEASE STATE.** crates.io + GitHub binaries + Homebrew tap all at **0.1.4** (fully coherent). CHANGELOG
`[Unreleased]` is now **empty** (everything folded into the dated `[0.1.4]`). Operating policy unchanged
(release fully autonomous, `main`-push always allowed, `pull→rebase→push` always allowed — root `AGENTS.md`).
The `hauis`-runner git-400 recurrence playbook is in `peculiarly-madly-sneeze` (closed) if binary/brew 400s again.

**NEXT — resume with `/stint-start`, execute the DAG below.** `GLOBAL HEAD-OF-LINE` is now
**`supervisor-dies-before-worker-node`** (Lane A) — the freshest high-value reliability bug, **3× reproduced
under load today**, directly relevant to parallel spinoff waves (relates to the still-open
`supervisor-spawn-fails-silently-at-run-create` #4-load-trigger). Practical *actionable* heads, disjoint +
parallel-safe: Lane B **`wave-terminal-worker-own-artifact-unaudited`** (F4 — a worker that commits then
panics leaves its OWN branch unaudited), Lane E **`run-wait-still`** (the `node_count>0` supervisor-died-mid-run
variant of the stillborn fix — actionable), Lane D `skill-companion-codex-layout` (decide-the-layout, low
urgency). Lane A's `peculiarly-cheerful-mine` (orchestrate driver heartbeat) is a **design-first** candidate
(needs LockedRun+append, inv 1-2) — better as `/worktree-code`. **Lane C is now EMPTY** (workmux vendoring
fully complete — multiplexer + git-wrapper both landed). **Lane D still carries `collision: bundled-skill
snapshot`** on `spinoff-skill-stale-preview-banner`. Recompute the head at pick time from live `issuectl`
status; merge the DAG at Phase 0/handoff. No worktrees remain; **`main` clean, 0 unpushed, `v0.1.4` tagged +
shipped, local binary 0.1.4 (`doctor` 0/0).**

---

## Execution DAG (2026-08-10)

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
GLOBAL HEAD-OF-LINE: supervisor-dies-before-worker-node (Lane A — NEW, 3× reproduced under load today; freshest high-value reliability bug. Other actionable heads: Lane B wave-terminal-worker-own-artifact-unaudited, Lane E run-wait-still)   ← start here on resume

LANE A — supervise/* + reducer/schema (create.sh, run/spawn.rs, capture.rs)
  ▶ supervisor-dies-before-worker-node       (NEW; supervisor dies BEFORE creating node n-0001 under FS/CPU saturation → run pending/stalled, 0 nodes, 0 commits; reproduced 3× today in a 3dbear parallel wave; re-spawn does NOT help under sustained load. Fix: retry/backoff on node creation, run-create fail-fast on non-alive supervisor, or backpressure/queue. Relates to supervisor-spawn-fails-silently-at-run-create)
    worker-process-hang                      (in-progress; now unblocked — capture landed; but WHY pid exits is agent-runtime scope)
    supervisor-spawn-fails-silently-at-run-create   (high; #4 stateful load-trigger only — no repro, investigative)
    peculiarly-cheerful-mine                 (orchestrate driver HEARTBEAT/lease — generalizes the shipped read-time stall hint to the 4 shapes it can't catch; needs LockedRun+append (inv 1-2); follow-up of peculiarly-muddled-caption; DESIGN-FIRST candidate)
    moderately-macabre-self                  (verify reciprocal parent/child relationship before cross-run supervisor ops; typed-selector review follow-up; STUB — needs scoping)
    uncommonly-fuzzy-swing                   (spinoff blocked on USER INPUT at a genuine fork must propagate to the parent agent (with delay) → surfaced to user, not a silent multi-hour block; round-3 finding; relates to no-completion-notification-to-parent + notify-run-level-summary)
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
  ▶ wave-terminal-worker-own-artifact-unaudited (F4 — a worker that commits then panics/errors leaves its OWN branch unaudited; pre-existing)
    push-blocked-chunk-tier-and-commit-audit (F6 — push_blocked_chunk records plan-declared tier not the promoted one + omits commit OID; pre-existing)
    dreadfully-dirty-pain                    (carry stale wave-build diff + findings into rebase-and-fix re-brief; wave-promotion follow-up)
    practically-exclusive-celery             (meter agent usage spent before a wave-build worker panic; wave-promotion follow-up)
    pipeline-hardening
    pipeline-run-create-wiring               collision: create.sh   (shares w/ Lane A capture)
    pipeline-breaker-inflight-and-opus-metering
    pipeline-drop-primitive-underspecified
    run-create-harness-flag                  (feature: `run create --harness` promotes the pi adapter (+ others) from bakeoff into real runs; touches harness/* — collision: create.sh w/ Lane A)
    pipeline-tiered-triage                   (in-progress; deferred self-disagreement trigger)

LANE C — workmux vendoring — COMPLETE (empty; multiplexer + git-worktree wrapper both vendored & landed 2026-08-10)

LANE D — workflow/skill (skill prose + skill registry; sequence, touches bundled-skill catalog)
  ▶ skill-companion-codex-layout           (skill.rs: companion resources are claude-only; codex flat layout unsupported — both filed by the split-stint worker)
    doctor-orphan-companion-files           (skill.rs: doctor should also detect ORPHAN companions — files a prior binary installed but this binary no longer bundles; doctor-skill-companion-sync follow-up)
    skill-install-force-symlink            (skill.rs: install --force aborts on a pre-existing symlink — refused_overwrite; prune/handle the stale symlink first)
    spinoff-skill-stale-preview-banner     collision: bundled-skill snapshot (octl-spawn-spinoff SKILL.md still carries a "NOT IMPLEMENTED" preview banner — prose fix)

LANE E — run/* CLI surface (touch run/*, not supervise core; lower collision, still sequence)
  ▶ run-wait-still                           (NEW; `run wait` STILL blocks on an orphaned run with node_count>0 when the supervisor died MID-run — the sibling the stillborn fix did NOT cover (that fix handled node_count==0). run-wait/supervisor/reliability)
    supervisorview-conflates-states          (NEW; run show/list SupervisorView conflates absent/dead/unreadable/unprobed supervisor states — run-show follow-up)
    count-jsons-swallows-io                  (NEW; `run show` count_jsons silently returns 0 on a filesystem read failure — should surface the IO error, not a false 0; run-show follow-up)
    run-salvage-command
    orchestrate-integration-branch-no-worktree-merge-fails
```
<!-- execution-dag:end -->

**Epic (not a lane node):** `code-pipeline` — parent of the Lane B `pipeline-*` work.
**Adjacent backlog / deferred:** none currently parked. (`peculiarly-madly-sneeze` — the `hauis`
runner git-400 that blocked binary/brew releases — was root-caused and **closed** this session; see
the Continue-here banner.) The full open list is `issuectl ls --status open`.

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
**Round executed 2026-08-05 (B‖C‖D parallel, then E):** landed `pipeline-parallel-chunks` (fixed;
concurrent DAG-wave scheduling, opt-in `--max-build-concurrency`, /llm-review caught + fixed an
invariant-5 leak) and `doctor-skill-companion-sync` (fixed; companion presence+sync check). Re-scoped
`workmux-extract-libs` (kept open — multiplexer already vendored; narrowed to the git-worktree-wrapper
remainder). Dropped the 2 landed; added worker follow-ups `immoderately-dirty-cushion` (Lane B, tier
promotion in wave builds) + `doctor-orphan-companion-files` (Lane D, orphan-companion detection).
`landing-signal-reliable-after-rebase` (Lane E) landed after D cleared the bundled-skill-snapshot
collision (CLI rebase-robust `landed` flag via git cherry patch-id + ancestry net; stint/spinoff docs
no longer rely on `merge-base --is-ancestor`). Integrated gate green (fmt/clippy/`cargo test
--workspace` all pass on integrated main); local rebuild redeployed, `doctor` 0 fail / 0 warn (the new
`skill.sync.stint-start.AGENTS-EXECUTION-DAG.md` companion check passes live). All 4 units landed on
first spawn — no worker deaths. No worktrees remain.
**Handoff reconcile 2026-08-06:** `comm -3` found 1 left-only (`peculiarly-muddled-caption`, filed by a
parallel session) → added to Lane A; 0 right-only. Heads advanced (all round issues terminal): Lane B
head → `pipeline-provenance-durable-refs`, Lane D head → `skill-companion-codex-layout`, Lane E head →
`cancel-run-already-terminal-error-class`. Then v0.1.1 shipped + release-autonomy policy change (see
Continue-here). DAG driftless at wrap.
**Round executed 2026-08-06 (A‖B‖E parallel, 3 headless spinoffs, ~48 min):** landed
`cancel-run-already-terminal-error-class` (fixed; `run cancel` on a terminal run is now a USER
error exit 1 + array `expected` hint), `peculiarly-muddled-caption` (fixed; read-time `stalled`
hint on `run show`/`run list` for undriven `--kind orchestrate` drivers, 12-min grace, no
reducer/schema touch), `pipeline-provenance-durable-refs` (done; durable `refs/pipeline/prov/…`
pinned before rebuild reset + teardown-gated cleanup; /llm-review applied). All 3 landed on first
spawn — no worker deaths. Integrated gate: `cargo test --workspace` surfaced a PRE-EXISTING
parallel-execution flake `snapshot_is_one_invocation_per_socket_regardless_of_node_count` (green
in isolation/single-thread/retry; no round code touched watchdog) → filed `immoderately-irate-north`
(Lane A). Dropped the 3 landed; added worker follow-up `peculiarly-cheerful-mine` (Lane A, driver
heartbeat) + the flake. Local rebuild redeployed, `doctor` 0 fail / 0 warn (545 ok). No worktrees
remain. **Then v0.1.2 SHIPPED** (crates.io `octl-core`→`orchestratectl`, `v0.1.2` tag → Release CI).
**Round 2 executed 2026-08-06 (A‖B‖E parallel, 3 headless spinoffs, ~66 min):** landed
`immoderately-irate-north` (fixed; de-flaked the watchdog invocation-count test — isolation-safe
counting + lock audit; confirmed: `cargo test --workspace` green across 2+ full runs, flake test
passes every time), `immoderately-dirty-cushion` (done; wave-build exhaustion re-queues to a
sequential drain for tier promotion + per-worker `catch_unwind` preserving siblings, inv-5;
/llm-review applied), `run-paths-typed-selector-split` (done; typed `run_paths_exact` +
sealed `RunSelector`, prefix resolved only at CLI verb entry, internal paths exact-only;
4/4 review consensus applied). All 3 landed first spawn — no deaths. Integrated gate green
(1159 passed, 0 failed). Dropped the 3 landed; added 5 worker follow-ups — Lane A
`moderately-macabre-self` + `wildly-glorious-food`, Lane B `entirely-faithful-beast` +
`dreadfully-dirty-pain` + `practically-exclusive-celery`. Local rebuild redeployed, `doctor`
0 fail / 0 warn (553 ok). CHANGELOG `[Unreleased]` carries the wave-promotion fix (no release
cut this round — internal/hardening + one opt-in-path fix; batch into the next user-facing cut).
No worktrees remain. **Then v0.1.3 FULLY SHIPPED** (crates.io 0.1.2→0.1.3; the binary/brew release
had been blocked by a leaked stale `actions/checkout` `http.extraheader` in the `hauis` runner's
GLOBAL git config → HTTP 400; root-caused + fixed on `hauis` (`git config --global --unset-all` the
extraheader + insteadof leaks), re-ran the v0.1.3 Release → all green; closed `peculiarly-madly-sneeze`).
**Round 3 executed 2026-08-06 (A‖B‖E parallel, 3 headless spinoffs):** landed `wildly-glorious-food`
(done; supervisor logs/quarantines CORRUPT persisted child ids vs benign missing-run skip),
`run-wait-timeout-unit-required` (done; `run wait --timeout` accepts a bare integer as seconds — kills
the silent-instant-exit failure mode), and `entirely-faithful-beast` (done). **Lane B paused ~6h at a
genuine fork** — the worker did the work + /llm-review, but review found the issue's premise (data-loss
on hard error) was a **non-bug** (teardown never deletes chunk branches), and it then blocked ~6h on user
input awaiting a scope decision. Resolved this once manually (option 1): land the reviewed/green modest
inv-5 robustness improvement (hard error DOMINATES the wave terminal — reverts a panic-wins regression
that hid infra failures via Ok/exit-0; co-occurring panic surfaced via `PipelineError::with_note`) + spin
off the real audit work; the worker completed its own `run merge`. Integrated gate green (1164 passed, 0
failed; round-2 watchdog de-flake holding). Dropped the 3 landed; added 3 Lane B follow-ups the worker
filed — `pipeline-hard-failure-carries-report` (F5, the genuine inv-5 audit fix),
`wave-terminal-worker-own-artifact-unaudited` (F4), `push-blocked-chunk-tier-and-commit-audit` (F6). No
worktrees remain. **The spinoff-blocked-on-user-input stall is a real lifecycle gap → filed as
`uncommonly-fuzzy-swing` (Lane A): the need-for-input must propagate to the parent agent (with a delay)
so it surfaces to the user, instead of a silent multi-hour block. Not a workaround — the fix.**

**Round executed 2026-08-10 (one round, sequenced across lanes, 8 units, all landed first spawn — no deaths):**
Planned 3: `run-wait-stillborn-run-not-detected` (fixed), `pipeline-hard-failure-carries-report` (done),
`run-show-json-null-fields` (fixed). Then the CI-health thread: `ci-red-main-deny-docs` (fixed; docs+deny —
main had been red for days, caught by a worker mail-sweep), `merge-lock-flock-not-portable-macos` (fixed; real
macOS bug, flock→portable mkdir lock), MSRV restore (pinned `time=0.3.41` + time-boxed `deny.toml` ignore of
RUSTSEC-2026-0009, kept MSRV 1.85 — see KEY LEARNING #1), `dry-run-projection-parity-flake` (fixed; inode-recycle
write-detection → mtime+size fingerprint), `workmux-extract-libs` (done; user-approved typed git wrapper).
Dropped the 4 that were IN the DAG and landed (`run-wait-stillborn-run-not-detected`, `run-show-json-null-fields`,
`pipeline-hard-failure-carries-report`, `workmux-extract-libs`); the other 4 landed units were filed+fixed
within the session so never entered the DAG. **Lane C emptied** (workmux vendoring complete). `comm -3` drift
found 4 left-only new issues → added: `supervisor-dies-before-worker-node` (Lane A, NEW GLOBAL HEAD — 3× repro
under load), `run-wait-still` + `supervisorview-conflates-states` + `count-jsons-swallows-io` (Lane E, run-wait /
run-show follow-ups). Lane B head → `wave-terminal-worker-own-artifact-unaudited`. **Then v0.1.4 FULLY SHIPPED**
(crates.io `octl-core`→`orchestratectl`, `v0.1.4` tag → Release CI all-green, Homebrew tap 0.1.4). Integrated
gate + full CI green (all 7 jobs) before publish. Local rebuild redeployed, `doctor` 0 fail / 0 warn (687 ok).
No worktrees remain. DAG driftless at wrap.

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
