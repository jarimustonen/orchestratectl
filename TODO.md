# TODO

Session-level plan + handoff. Longer-running planning/design docs live under
`issues/<slug>/{design,plan,breakdown,…}.md`; this file points at issuectl issues
for the actual tracked work.

---

## 🔄 Continue here (ALOITA TÄSTÄ) — 2026-08-11 (3 reliability fixes landed UNRELEASED on `main`; pi.dev harness migration prioritized as GLOBAL HEAD)

**✅ LATEST (2026-08-11 — read first).** One `/stint-start` round, **B‖D‖E parallel, 3 headless spinoffs, all
reviewed (/llm-review + /assess-findings) + green, landed first spawn (no deaths):** `push-blocked-chunk-tier-and-commit-audit`
✅ (done), `supervisorview-conflates-states` ✅ (done), `skill-companion-codex-layout` ✅ (done). Integrated gate
green (fmt/clippy/`cargo test --workspace`, 0 failures). Local rebuild redeployed, `doctor` 0 fail / 0 warn (707 ok).
**NO RELEASE this round (Jari's call — `/stint-handoff` without release):** the 3 fixes + the version-snapshot CI-red
fix are **UNRELEASED on `main`** on top of shipped **0.1.5**. Next user-facing cut = **v0.1.6** (CHANGELOG
`[Unreleased]` is empty — author entries at cut time; release is fully autonomous per policy). `main` clean.
⚠️ **NOTE: `main` has UNPUSHED commits + unreleased work** — a `git push` keeps origin in sync (always allowed);
publishing them is the v0.1.6 cut whenever warranted.

**What the 3 fixes do:**
- `push-blocked-chunk-tier-and-commit-audit` (Lane B) — `push_blocked_chunk` **and** the crash/panic audit path now
  record the **promoted/effective** tier (not the plan-declared one) + the **commit OID** of committed-but-blocked
  work; threaded the oid through BuildAttempt/ChunkAttempt/WaveBuildOutcome::Blocked; promotion regression test added.
- `supervisorview-conflates-states` (Lane E) — `run show`/`run list` no longer collapse supervisor conditions: a
  wire-level `SupervisorState` enum (**alive | dead | not-recorded | unreadable | unknown**), `alive` kept as a
  back-compat boolean. **Closed a real probe read-then-stat TOCTOU** (the /llm-review panel flagged it unanimously)
  and stopped indeterminate states (Unreadable/Unknown) from driving stillborn/orphaned verdicts (new
  `presumed_working()` predicate) — so an unreadable pid file can't mislead a reattach/cancel decision.
- `skill-companion-codex-layout` (Lane D) — companion resources now install for the **codex flat layout** (shared
  `~/.codex/prompts/_shared/` subdir; per-skill claude-layout companion links rewritten to `_shared/…`), claude
  layout provably byte-for-byte unchanged (`Cow::Borrowed`); drift-guard test suite pins the rewrite invariants.
  Follow-up filed by its review: `doctor-codex-companion-coverage` (Lane D).

**Bonus (CI-red cleared) + hygiene:** two workers refreshed the stale 0.1.4→0.1.5 `version_text` snapshot, which
**fixed the pre-existing main-wide CI red** — closed `version-envelopes-snapshot` (fixed) and its duplicate
`stale-version-envelope-snapshot` (both were the same 0.1.5-bump stale snapshot filed by parallel sessions). Also
removed a stray `envelope_snapshots__version_text.snap.new` the codex worker committed by accident (commit 2ca29ee).

**Why `run-create-harness-flag` (pi.dev) is now the GLOBAL HEAD (Jari 2026-08-11).** Migrate autonomous worker runs
from hardcoded Claude Code to the **pi.dev harness**. The pi adapter already WORKS (`bakeoff-aider-pi-live-fail`
fixed, pi 0.82 live-verified); the only missing piece is wiring `--harness` into `run create` so autonomous kinds
(spinoff/research/…) launch pi-driven workers. See `run-create-harness-flag` for the full spec (flag > env > config
precedence, per-kind default, `run show/list --json` surfacing, skill/agent-tool translation shim).

**KEY LEARNING #NEW (canonical) — "disjoint lanes" is a PREDICTION, not a guarantee; the integrated gate is
non-optional.** The DAG put `supervisor-dies-before-worker-node` in Lane A (supervise/*) and `run-wait-still`
in Lane E (run/*) as parallel-safe. But the supervisor-dies fix, once its real shape emerged, landed in
`run/*` (`run list` + the `RunSummary` DTO + `run show`), NOT supervise/*. Both spinoffs were green in
isolation; INTEGRATED, `main` did not compile (`E0425: stillborn not in scope` — run-wait-still's refactor of
`run/show.rs`'s scan-return tuple removed the `stillborn` binding the supervisor-dies change relied on). The
post-round `cargo test --workspace` on integrated `main` caught it immediately; a small 4th spinoff derived the
bool from the single `stall` source of truth. **Lesson:** a lane assignment predicts *likely*-touched files; a
fix can legitimately land elsewhere. Never skip the integrated gate for "independent" parallel units, and when
two units might both touch the `run show` / `RunSummary` DTO surface, prefer sequencing them.

**KEY LEARNING #1 (canonical) — RUSTSEC-2026-0009 vs MSRV 1.85 is a standing conflict.** The `time` crate's
stack-exhaustion DoS advisory is fixed only in `time ≥0.3.47`, but **every `time ≥0.3.47` requires rustc 1.88**
> our 1.85 MSRV floor. `time` is transitive-only (via `tracing-appender`, log-rotation timestamps — we never
parse untrusted time input, so the advisory is **not exploitable** here). Resolution: **pinned `time` to
`0.3.41`** (keeps MSRV 1.85) **+ a scoped, time-boxed `deny.toml` ignore** of RUSTSEC-2026-0009 documenting the
rationale. **Re-evaluate the ignore if/when MSRV moves to ≥1.88** (then unpin `time` and drop the ignore).
Corollary: bumping a dep to clear a `cargo-deny` advisory can silently blow the MSRV — always re-check the
`msrv (1.85)` job, don't just look at ubuntu.

**KEY LEARNING #2 (canonical) — parallel spinoff waves under saturation kill supervisors before the worker
node — the SURFACING half is now shipped (0.1.5); the RESILIENCE half remains open.** Under heavy FS/CPU
contention (multiple live supervisors, `git index.lock` races, `git worktree remove` + `run list` hitting the
120s timeout) a per-run supervisor can die before/around the first node, leaving a run `pending`/`stalled`,
`node_count=0` (stillborn) or `node_count>0` (orphaned mid-run), 0 useful commits. **As of 0.1.5 both shapes now
SURFACE promptly** (`pending (stillborn)` in `run list`; orphaned mid-run settles in `run wait`/`run show` past a
15-min grace) instead of blocking or looking-healthy — but the underlying *resilience* (making the supervisor not
die under load, or `run create` backpressure/queue when N supervisors already live) is **still open**:
`supervisor-spawn-fails-silently-at-run-create` (#4 load-trigger, investigative), `run-create-back-to-back-no-supervisor`,
plus the backpressure idea. Re-spawn does NOT help while the load persists (dies again); cleanup itself can wedge
(worktree-remove timeout → dir stranded, manual `rm -rf`). Next reliability thread: the resilience half.

**KEY LEARNING #3 (still canonical) — worker deaths are TRANSIENT.** Retry **with harvest** of the recoverable
preserved branch (review → adopt → complete → merge), NOT hand-merge of unreviewed work, NOT base-agent swap.
Heavy-LLM units legitimately take **54–96 min**; a long run is not a hang. (This round: all 8 units landed on
first spawn, no deaths.)

**RELEASE STATE.** crates.io + GitHub binaries + Homebrew tap all at **0.1.5** (fully coherent). CHANGELOG
`[Unreleased]` is now **empty** (everything folded into the dated `[0.1.5]`). **Release autonomy REAFFIRMED by
Jari (2026-08-10): cut releases autonomously at the right moments — DON'T ask, DON'T re-confirm.** This is
already the operating policy (release fully autonomous, `main`-push always allowed, `pull→rebase→push` always
allowed — root `AGENTS.md`); honor it without a permission prompt. The `hauis`-runner git-400 recurrence
playbook is in `peculiarly-madly-sneeze` (closed) if binary/brew 400s again.

**NEXT — resume with `/stint-start`, execute the DAG below.** `GLOBAL HEAD-OF-LINE` is now
**`push-blocked-chunk-tier-and-commit-audit`** (Lane B, F6 — a concrete pipeline audit bug continuing the
wave-audit thread just advanced by `wave-terminal-worker-own-artifact-unaudited`). Practical *actionable*
heads, disjoint + parallel-safe: Lane E **`supervisorview-conflates-states`** + `count-jsons-swallows-io`
(run-show hardening, low-risk), Lane D **`skill-companion-codex-layout`** (layout decision, low urgency),
Lane A **`supervisor-spawn-fails-silently-at-run-create`** (the resilience half of KEY LEARNING #2 —
investigative, no repro; `worker-process-hang` is in-progress/agent-runtime scope, parked;
`peculiarly-cheerful-mine` is **design-first** (LockedRun+append, inv 1-2) → better as `/worktree-code`).
**Lane C stays EMPTY** (workmux vendoring complete). **Lane D still carries `collision: bundled-skill
snapshot`** on `spinoff-skill-stale-preview-banner`. **⚠️ Watch the run-show/DTO collision surface** (KEY
LEARNING #NEW): Lane E run-show items + any Lane A fix that touches `run list`/`run show`/`RunSummary` can
collide — sequence them if both are in a wave. Recompute the head at pick time from live `issuectl` status;
merge the DAG at Phase 0/handoff. No worktrees remain; **`main` clean, 0 unpushed, `v0.1.5` tagged + shipped,
local binary 0.1.5 (`doctor` 0/0).**

---

## Execution DAG (2026-08-12)

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
GLOBAL HEAD-OF-LINE: arch-lifecycle-map-rootcause (Lane F — the ARCHITECTURE RE-EXAMINATION is now the priority thread. ~57% of open issues + 58% of bugs cluster in the supervisor/agent-lifecycle core; hypothesis: the supervisor INFERS a distributed process's state from indirect signals (pid × pane × branch × report) → combinatorial edge cases patching can't shrink. So we STOP patching the core and REVIEW it first.) Phase-1 runs read-only in PARALLEL with arch-feature-usage-audit + arch-supervision-alternatives. Non-core lanes proceed alongside: Lane B pi.dev (harness-pi-skill-shim) + pipeline, Lane D skill. ⚠️ Lane A (supervise core) + Lane E (run/* read surface) are ⛔ GATED behind ◆ DECISION-2 — do NOT spawn new cluster-A/B fixes until the harden-vs-rearchitect ADR lands; the in-flight agent-skips is the LAST one allowed through (→ ⬆ v0.1.7).   ← start here on resume

LANE F — ARCHITECTURE RE-EXAMINATION  (epic: lifecycle-architecture-review)  ★ PRIORITY
Phase 1 (read-only research — PARALLEL-SAFE: disjoint output files under issues/lifecycle-architecture-review/, no code edits):
  ▶ arch-lifecycle-map-rootcause           (high; map the run/supervisor/agent lifecycle end-to-end + taxonomy of the ~24 cluster-A/B issues by signal-combination + root-cause inference-vs-protocol → analysis.md)
    arch-feature-usage-audit               (HIGH — Jari flagged 2026-08-12: actual usage is NARROW, some options likely unneeded. Ground the audit in Jari's REAL use set, then flag every unused kind/flag/subsystem as a removal candidate w/ its drag cost. BIAS TOWARD CUTTING. Suspects: 9 run-kinds, code-pipeline/wave-build, bakeoff, discussions → feature-audit.md; feeds ◆ DECISION-1)
    arch-supervision-alternatives          (survey protocol/state-machine, exit-code+FIFO, event-sourced/lease vs the polling-watchdog → alternatives.md)
Phase 2 (needs all three phase-1 docs):
    arch-redesign-design-session           after arch-lifecycle-map-rootcause, arch-feature-usage-audit, arch-supervision-alternatives (needs the evidence base) — facilitated /llm-workshop WITH Jari; simplest architecture that collapses cluster A → design.md
Phase 3 (THE decision):
    arch-decision-rearchitect-vs-harden    after arch-redesign-design-session (needs the chosen design) — ADR via /worktree-technical-decision; harden vs re-architect; GATES ◆ DECISION-2

LANE A — supervise/agent-lifecycle CORE (cluster A, 23 issues)  ⛔ GATED behind ◆ DECISION-2
(do NOT spawn new fixes here until the ADR decides disposition — listed in full so nothing is outside the DAG)
    agent-skips-run-merge-idle-pending      [IN-FLIGHT — the LAST one allowed through; on landing → ⬆ v0.1.7]
    worker-process-hang                     (in-progress; WHY the pid exits is agent-runtime scope, parked)
    supervisor-stall-detection              (stalled:false through a multi-hour silent hang; run wait 6h default too long)
    supervisor-spawn-fails-silently-at-run-create   (#4 stateful load-trigger; investigative, no repro; RESILIENCE half of KEY LEARNING #2)
    run-create-back-to-back-no-supervisor
    reattach-does-not-bootstrap-crashed-at-creation-run
    child-supervisor-spawn-exhaustion-lifecycle
    cancel-dead-supervisor-recovery
    legacy-pid-identity-check
    autoretry-crash-consistency
    idle-empty-handed-alive-agent-hangs
    watchdog-pane-aware-liveness
    watchdog-tick-verdict-refactor
    teardown-gate-trust-and-lifecycle
    moderately-macabre-self
    peculiarly-cheerful-mine                (orchestrate driver HEARTBEAT/lease; needs LockedRun+append inv 1-2; DESIGN-FIRST)
    uncommonly-fuzzy-swing                  (spinoff blocked-on-user-input must propagate to parent, not silently block)
    no-completion-notification-to-parent
    notify-run-level-summary
    code-run-inject-no-selfmerge
    interactive-merge-audit-marker
    run-salvage-command                     (recover a dead agent's stranded work — the salvage command)
    orchestrate-integration-branch-no-worktree-merge-fails

LANE B — pipeline/* + harness/* (NOT lifecycle core — proceeds in parallel with Lane F)
· pi.dev sub-thread (→ ⬆ v0.2.0):
  ▶ harness-pi-skill-shim                   (pi worker SKILL translation shim; harness/* — unblocked now ci-docs landed)
    workmux-pi-agent-preset                 (workmux pi agent preset for `--harness pi`)
    config-subcommand                       (config path + config show --json; config.rs — pairs w/ the harness config layer)
· pipeline sub-thread:
    dreadfully-dirty-pain                   (mechanical wave-build rebase-and-fix; wave-promotion follow-up)
    practically-exclusive-celery            (meter agent usage before a wave-build worker panic)
    pipeline-hardening
    pipeline-run-create-wiring              collision: create.sh
    pipeline-breaker-inflight-and-opus-metering
    pipeline-drop-primitive-underspecified
    pipeline-tiered-triage                  (in-progress; deferred self-disagreement trigger)

LANE C — workmux vendoring — COMPLETE (empty; landed 2026-08-10)

LANE D — workflow/skill (skill.rs + skill prose; NOT lifecycle core — proceeds)
  ▶ pidev-pi-skill-lifecycle               (skill.rs: pi skill prune-orphans + doctor drift via out-of-band provenance)
    skill-install-force-symlink            (skill.rs: install --force aborts on a pre-existing symlink)
    spinoff-skill-stale-preview-banner     collision: bundled-skill snapshot (octl-spawn-spinoff SKILL.md preview banner — prose fix)

LANE E — run/* read surface (cluster B, run-state DTO)  ⛔ GATED behind ◆ DECISION-2
(part of the lifecycle-state review — hold; listed so nothing is outside the DAG)
    node-show-null-report                   (node show null report after self-merge — report is in nodes/<node>.json last_report)
    run-show-null-worktree-path             (run show null worktree_path/source_branch for a live pending run)
    count-jsons-swallows-io                 (run show count_jsons swallows a filesystem read error as a false 0)

◆ DECISION-1 — after arch-feature-usage-audit lands: decide which unused features/surfaces to DEPRECATE or REMOVE (drag reduction). Jari (2026-08-12) actively expects unused options here — bias toward cutting; may fire EARLY (does not wait for the full ADR). May obsolete Lane A/B issues + spawn removal work.
◆ DECISION-2 — after arch-decision-rearchitect-vs-harden lands (the ADR): disposition of EVERY Lane A + Lane E issue — keep-and-fix / defer / OBSOLETE-as-subsumed / re-scope. This is the "what do we do with the open issues" checkpoint; it GATES Lanes A + E.

⬆ RELEASE v0.1.7 — agent-skips-run-merge-idle-pending (+ already-landed ci-docs-bakeoff-registry-link + doctor-codex-companion-coverage). Cut once agent-skips lands + integrated gate green (clears the CI-red docs job for users).
⬆ RELEASE v0.2.0 — pi.dev harness milestone: harness-pi-skill-shim + workmux-pi-agent-preset + config-subcommand + pidev-pi-skill-lifecycle (on top of 0.1.6's --harness + dual-home). Cut when the pi.dev thread runs one autonomous kind pi start→merge→report end-to-end.
⬆ RELEASE (gated on ◆ DECISION-2) — lifecycle release: bundles the harden fixes OR ships the re-architecture per the ADR; version TBD (0.3.0 if re-architect).
Cadence: release whenever a wave lands shippable user-facing work (operating policy — release often, fully autonomous).
```
<!-- execution-dag:end -->

**Epics (not lane nodes):** `code-pipeline` — parent of the Lane B `pipeline-*` work;
`lifecycle-architecture-review` — parent of **Lane F** (the architecture re-examination).
**Nothing is outside the DAG.** Every active non-epic issue (44 as of 2026-08-12) sits in a
lane above — verified drift-clean by the `comm -3` check. No `deferred`-parked items. The full
open list is `issuectl ls --status open`; `issuectl ls --status open --json | jq length` should
equal the lane-node count.

### Wave plan (next waves — planned into lanes)

- **Wave 1 (immediate, on resume):** cut **⬆ v0.1.7** the moment the in-flight `agent-skips`
  lands + integrated gate is green (clears the CI-red docs job for users). Then spawn **Lane F
  Phase-1 trio in parallel** — `arch-lifecycle-map-rootcause` ∥ `arch-feature-usage-audit` ∥
  `arch-supervision-alternatives` (all `/worktree-research`, **read-only, disjoint output files
  → safe to parallelize despite one lane**) — alongside one **Lane B pi.dev** unit
  (`harness-pi-skill-shim`) and one **Lane D** unit (`pidev-pi-skill-lifecycle`). **NO new Lane A
  / Lane E spawns** (⛔ gated behind ◆ DECISION-2).
- **Wave 2:** once the Phase-1 trio lands, run **Lane F Phase-2** `arch-redesign-design-session`
  — a facilitated `/llm-workshop` **with Jari** (interactive, not headless). Continue the Lane B
  pi.dev + pipeline and Lane D threads in parallel.
- **Wave 3:** **Lane F Phase-3** `arch-decision-rearchitect-vs-harden` → the ADR → **◆ DECISION-2**
  (re-triage all Lane A + Lane E issues: keep / defer / obsolete / re-scope). Cut **⬆ v0.2.0**
  when the pi.dev thread completes end-to-end. Fire **◆ DECISION-1** after
  `arch-feature-usage-audit` (deprecate/remove dead-weight).
- **Wave 4+:** execute the ◆ DECISION-2 outcome (harden the surviving Lane A/E issues, or the
  re-architecture campaign via `/orchestrate`), then the gated lifecycle release.

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
**Round 2 executed 2026-08-10 (A‖B‖E parallel, 3 headless spinoffs):** landed
`supervisor-dies-before-worker-node` (fixed; stillborn surfaced in `run list` — re-scoped, see LEARNING #NEW),
`run-wait-still` (fixed; orphaned-mid-run detection in `run wait`/`run show`, 15-min grace), and
`wave-terminal-worker-own-artifact-unaudited` (done; wave worker's own branch audited across `catch_unwind`).
All 3 landed first spawn, no deaths. **Integrated gate caught a latent collision:** the supervisor-dies fix
actually landed in `run/*` (not supervise/*), colliding with run-wait-still on `run/show.rs` → `main` didn't
compile (`E0425 stillborn`, each green alone). A 4th integration-fix spinoff resolved it (derive the bool from
the single `stall` verdict); full `cargo test --workspace` then green. `comm -3` drift found only the 3 landed
as right-only (0 left-only — no worker-filed issues); dropped all 3. Heads advanced: Lane A ▶
`supervisor-spawn-fails-silently-at-run-create` (worker-process-hang parked in-progress), Lane B ▶
`push-blocked-chunk-tier-and-commit-audit`, Lane E ▶ `supervisorview-conflates-states`; Lane D unchanged.
**Then v0.1.5 FULLY SHIPPED** (crates.io `octl-core`→`orchestratectl`, `v0.1.5` tag → Release CI green 4m36s,
Homebrew tap 0.1.5). Local rebuild redeployed, `doctor` 0 fail / 0 warn (690 ok). No worktrees remain. DAG
driftless at wrap.
**Round executed 2026-08-11 (B‖D‖E parallel, 3 headless spinoffs — NO release this round, Jari's call):**
landed `push-blocked-chunk-tier-and-commit-audit` (done; push_blocked_chunk + crash-audit paths record the
promoted/effective tier + the commit OID of committed-but-blocked work; promotion regression test),
`supervisorview-conflates-states` (done; wire-level `SupervisorState` enum alive|dead|not-recorded|unreadable|unknown
on `run show`/`list`, `alive` kept back-compat; closed a real probe read-then-stat TOCTOU the /llm-review panel
flagged unanimously; indeterminate states no longer drive stillborn/orphaned verdicts), `skill-companion-codex-layout`
(done; codex flat-layout companions install to `~/.codex/prompts/_shared/` w/ link rewrites, claude layout
byte-for-byte unchanged, drift-guard test suite). All 3 reviewed (/llm-review + /assess-findings) + green, landed
first spawn — no deaths. **Bonus:** two workers refreshed the stale 0.1.4→0.1.5 version snapshot, which
fixed the **pre-existing main-wide CI red** — closed `version-envelopes-snapshot` (fixed) + `stale-version-envelope-snapshot`
(duplicate). Removed a stray `envelope_snapshots__version_text.snap.new` a worker committed by accident (commit 2ca29ee).
Integrated gate green (fmt/clippy/`cargo test --workspace`, 0 failures). Local rebuild redeployed, `doctor` 0 fail /
0 warn (707 ok). Dropped the 3 landed; `comm -3` add: `doctor-codex-companion-coverage` (Lane D, worker follow-up).
Heads advanced: Lane B ▶ `dreadfully-dirty-pain` (◀ `run-create-harness-flag` PRIORITIZED for pi.dev), Lane D ▶
`doctor-orphan-companion-files`, Lane E ▶ `count-jsons-swallows-io`; Lane A unchanged. **NO RELEASE cut** — the 3
fixes + snapshot fix are UNRELEASED on `main` on top of shipped 0.1.5; next cut is v0.1.6 (CHANGELOG `[Unreleased]`
still empty — author the entries at cut time). No worktrees remain. DAG driftless at wrap.

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
