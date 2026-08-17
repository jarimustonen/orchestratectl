# TODO

Session-level plan + handoff. Longer-running planning/design docs live under
`issues/<slug>/{design,plan,breakdown,…}.md`; this file points at issuectl issues
for the actual tracked work. Standing rules and canonical learnings live in the
root `AGENTS.md` (operating policy + state-integrity invariants) — this file
holds only the **active handoff** and a **compact stint archive**.

---

## 🔄 Continue here (ALOITA TÄSTÄ), 2026-08-17 (**v0.3.0 SHIPPED + CI GREEN — NEXT = `add-configurable-agent` ALONE, or `skills` head `audit-no-user-specifics`**)

**✅ (2026-08-17, stint 5 — this handoff).** Full round + release: **v0.3.0 cut through every channel** — and for the
first time through the round's own new publish gate. 5 headless spinoffs (3 planned units + 2 CI-red fixes), **all
landed first-spawn, no worker deaths**. crates.io has `octl-core 0.3.0` + `orchestratectl 0.3.0`; the `v0.3.0` tag's
Release CI and crates-publish CI both **green**; main CI green on the tagged commit. Local binary **0.3.0**,
`orchestratectl doctor` **1011 ok / 0 warn / 0 fail** (all three skill mirrors). `main` clean and pushed.

**What landed (3 planned units, all 3 lane heads):**
- **`release-gate-on-ci`** (skills, high) — `publish-crates.yml` now runs the FULL main-CI gate (fmt, snapshots,
  clippy, tests both platforms, msrv, docs, deny) **plus a tag↔manifest version-match check** before any publish step;
  a tag on a red or mismatched commit can no longer publish. `OSS-RELEASE.md` + `AGENTS.md` document the tag-triggered
  flow; no doc anywhere still instructs a local `cargo publish`. **Proven live by this very release.**
- **`create-idempotency-lease-recovery`** (lifecycle, high) — pre-publication idempotency reservations now carry a
  durable creator lease (pid + start-time, staleness-bounded): a keyed retry returns the published original, reclaims
  provably-dead staging atomically, or fails closed when unverifiable. Keyed parent-edge (`child.spawned`) read repair
  included. Follow-up `recover-unkeyed-child-publication` was filed by the worker and **closed `wontfix` on Jari's
  call** (cosmic-ray-class window; fan-out already mandates per-unit keys; consequence is bookkeeping, not data loss —
  mechanism documented in the closed issue).
- **`config-show-layered-view`** (surface; was stale-labelled `deferred`, un-deferred at round start) — `config show`
  is now a layered, tolerant inspection surface (config schema **v2**): raw file/env/default layers per key incl.
  `[harness.per_kind]` visible under env shadowing, per-row `valid`/`validation_error` instead of dying on the value
  being debugged, file-layer validity independent of ambient env, `--show-secrets` warning via the JSON `warnings`
  envelope. Execution-path strictness unchanged.

**⚠ TWO CI-RED INCIDENTS, both caught by CI after a green local+integrated gate, both fixed same-round (the release
waited for green each time):**
1. **`ci-red-release-mode-injection`** — CI tests run `--release`; the `OCTL_TEST_*` injection hooks in `run/create.rs`
   are (deliberately) `cfg!(debug_assertions)`-gated, so a new lease-recovery test asserting on injection passed the
   debug-mode local gate and failed both CI platforms. Fix: injection-dependent tests are skipped in release builds
   (hooks stay debug-only — a production binary must never honor a test kill switch). **New blind-spot axis: the local
   gate is debug-mode; CI is the only release-mode gate.**
2. **`etxtbsy-cross-module-stub-race`** — the THIRD ETXTBSY occurrence. Stint 4's mutex serialized only the fake-tmux
   family, but `run/merge.rs`, `git/repo.rs`, `supervise/capture.rs`, `supervise/cleanup.rs` also write exec stubs in
   the same unit-test process; fork-inheritance leaks a write fd across module boundaries (hit
   `multiplexer::tmux::tests::new_session_surfaces_nonzero` even though it HELD the tmux mutex). Structural fix:
   **CI test jobs (ci.yml + publish-crates.yml) now run cargo-nextest, process-per-test** — the whole class is gone,
   not re-mutexed; doctests preserved via a separate step. *(The stint-4 "kept HERE only" macOS/Linux ETXTBSY learning
   is now superseded by this structural fix + the AGENTS.md release-gate rules; archive has the pointer.)*

**Release-mechanics notes (mechanics only — the rules live in `AGENTS.md`):** the gated tag push worked as designed
(the first tag attempt was correctly withheld on red CI, twice); `gh run watch` dies on transient GitHub 504/429s, so
follow a failed watch with an explicit `gh run view --json conclusion` re-check before concluding red — and a 429
downloading a CI *action* (cargo-deny job, run 32041657942) is infra noise, not our failure. Also: the default
`skill install --force` covers claude+pi only — after a version bump run `--agent codex` too or doctor shows 13 codex
sync warns.

**⏭ NEXT — `add-configurable-agent` ALONE (surface lane head, in-progress: design.md v2 is ready to implement), or the
`skills` head `audit-no-user-specifics`.** `add-configurable-agent` is cross-cutting (config + `harness::select` +
run-create) — **do not run it in parallel with any `lifecycle` unit** (see the steer blocks below, which are its
working brief). `audit-no-user-specifics` (high, guards a public-artifact leak class) has now been head-adjacent for
four stints — if the profile work doesn't start this round, do it. Cheap parallel partner for either: the `lifecycle`
head `uncommonly-fuzzy-swing` (blocked-on-user-input propagation) — but NOT alongside `add-configurable-agent`.

**Lanes (3):** `lifecycle` (11, head `uncommonly-fuzzy-swing`), `skills` (5, head `audit-no-user-specifics`),
`surface` (1: only `add-configurable-agent`, in-progress). Realistic parallelism 2–3 units/round; only 2 if the
profile work runs (it excludes `lifecycle`).

**⚠ `add-configurable-agent` does NOT fit the lane model — read before scheduling it.** Jari's own feature request
(named agent profiles: `expert`/`standard`/`implementer`/`secure`, with fallbacks + config layering). Laned `surface`
(now that lane's only item, `in-progress` — design.md v2 is done, implementation is next), but it is
**genuinely cross-cutting**: config surface **and**
`harness::select` **and** the run-create path (accepting a profile, recording resolved profile/model/fallback in run
metadata). Run-create is `lifecycle` territory. **Do not run it in parallel with any `lifecycle` unit.** The schema has
a `collision` field for exactly this, but it is **not implemented in `issuectl update`** and no issue uses it, so the
warning lives in the issue body instead (filed upstream as `intake-feature-issuectl-769ae85ab662`).

**◆ DESIGN STEER ON `add-configurable-agent` (Jari, at this wrap) — capability names are the interface, raw model IDs
are at most an escape hatch.** The system should be driven by a small set of capability tiers (roughly *ultra-capable /
capable / fast / security-conscious*), configurable but shipping **sensible defaults for both the role set and the
mapping**, so it works with no config file present. Consequence: the issue's `expert`/`standard`/`implementer`/`secure`
examples are **capability tiers, not Jari's fleet baked in** — the model IDs there are illustrative config, never
built-in defaults (same leak class `audit-no-user-specifics` exists to catch). **Open, NOT decided:** whether to also
expose raw `--model` / `--effort` flags — Jari is explicitly unsure. Recommendation recorded on the issue: build the
capability layer first, add raw flags only if a concrete need survives it. Note the merged intake's escalation case
(terra gave up twice, sol finished in one pass) *argues for* the tier framing — "retry one tier up" is portable where
two hardcoded vendor IDs are not.

**◆ AND `secure` IS ORTHOGONAL TO THAT LADDER — a safety constraint, not a naming quibble (Jari, same wrap).** The
other roles differ in *capability*; `secure` differs in **data residency**: it runs **locally**, so the payload never
leaves the machine, which is what makes it safe for personal data and API-key **contents**. It is deliberately *low*
capability — accepted cost, not a defect. Two axes (`fast < capable < ultra-capable` × `local | remote`), not four
rungs. The consequence that matters: **a `secure` profile must NEVER fall back to a remote model.** A fallback that
silently escalates to the cloud would exfiltrate exactly the data the choice was protecting, precisely when things are
already failing. Fallback chains must stay inside the same residency class, and exhausting them must **fail with an
actionable error rather than degrade**. "Retry one tier up" is a capability-axis move and must not cross the residency
axis. It also breaks the issue's capability-driven auto-selection: capability follows task *difficulty*, residency
follows *what data the task touches* — and a credentials task is usually trivial, so a difficulty-ranking planner will
never pick `secure` correctly. The design needs an explicit task-sensitivity signal. Recommendation recorded on the
issue: model residency as a **machine-checkable profile attribute** (e.g. `local: true`), not as one role's name, so
the fallback resolver can enforce it instead of relying on the implementer remembering which role is special.

**`intake-feature-orchestratectl-d0c82ab27c9d` closed duplicate into `add-configurable-agent`**, content transcribed
first. It contributed three requirements the feature issue lacked: (1) the per-run override is the **primitive** and a
valid **MVP slice that may land first**; (2) the resolved profile/model/fallback must be **recorded on the manifest and
shown by `run show`**; (3) it must replace a genuinely unsafe workaround — pi reads its model from the **global**
`~/.pi/agent/settings.json`, so per-spawn selection today means rewriting that file and restoring it (racy under
concurrent spawns, easy to leave mutated). Any design still requiring global-settings mutation has not solved it. pi
accepts `--model "provider/id:<thinking>"` on its CLI, so passthrough is viable.

**Still open from the stint-4 triage sweep:** `intake-bug-orchestratectl-169460ea27e7` (lifecycle tail) — stale
pending runs clutter `run list` (~301KB, several from other repos). Re-scoped at that wrap: the staged-create fix
stops NEW stillborn pendings, so what remains is (a) cleaning the ~7 already on disk and (b) making a stale pending
distinguishable from a live worker in `run list`.

---

## Scheduling

Canonical scheduling lives in `issuectl` frontmatter (`lane:`, `lane_seq:`, `blocked_by:`, `collision:`). Do not maintain a markdown DAG or adjacent backlog in this file.

Use these views instead:

```bash
issuectl dag
issuectl dag --json
issuectl ls --status open
issuectl ls --status in-progress
```

`TODO.md` is only the session handoff and project notes; issue bodies and `issuectl dag` are the source of truth.

---

## Invariants + operating policy

The **7 state-integrity invariants** and the stint operating policy (release
mechanics, deploy, green gate + integrated gate, hot files, standing learnings)
live in the root `AGENTS.md`. Read them before touching the reducer, the lock
layer, or `supervise/`, and before any release action.

---

## Stint archive (compact — durable facts only)

Full narratives live in git history of this file; canonical rules extracted from
these stints are in `AGENTS.md`.

- **Review session (2026-08-17, after stint 4).** Fable-driven repo review + doc cleanup, parallel to the
  `add-configurable-agent` design session (design.md v2). `AGENTS.md` rewritten for consistency; `README.md` rewritten
  against 0.2.2 reality; stints 1–3 compressed into this archive. Code: `cut-plan-module` (dead 2013-line
  `octl-core/src/plan.rs` removed — the breaking entry that made the next release 0.3.0) + `harness-pi-default`
  (built-in default flipped to `pi` per ADR 0001 D4). Epics `code-pipeline` + `lifecycle-architecture-review` closed.
- **Stint 4 (2026-08-17, v0.2.2).** Full round + release + a caught mistake: 4 parallel spinoffs (all first-spawn),
  v0.2.2 cut, then CI caught one fix incomplete and a 5th spinoff finished it. Landed: `pi-spinoff-batch` (staged
  atomic run-create publication — the load-bearing fix), `cli-canon-help-json` (§14, clap-derived help envelope),
  `tmux-stub-etxtbsy-flake` (took two spinoffs; its tmux-family mutex later proved too narrow — the class was killed
  structurally in stint 5 by nextest process-per-test in CI), `spinoff-skill-stale-preview-banner` +
  `skill-install-force-symlink`. Lane fix: phantom `supervise` lane merged into `lifecycle` (verified from git;
  `lifecycle` deliberately NOT split despite depth). Origin of the "DO NOT `cargo publish` locally — the tag push IS
  the publish" rule (promoted to `AGENTS.md` after v0.2.2 was luckily-unharmed published before CI reported). Triage
  sweep: 2 closed cannot-reproduce (incl. `run-wait-false-stillborn-slow-start` — did occur when filed, did not recur
  after staged-create; re-file readily), stale-pendings intake laned.
- **Stint 3 (2026-08-17, v0.2.1).** Full round + release, 3 parallel spinoffs, all landed first-spawn.
  Landed: `spinoff-report-fields-null` (report read-back surface + docs in five skills — the four "null report" bugs
  were all read-surface errors), `run-create-long-title-stillborn` (branch names bounded to workmux's 50-byte
  window-name input), `cli-canon-version-schemas` (§10: `supported_schemas` in `version`). Closed without code:
  `cli-canon-config` (already shipped in 0.2.0). Origin of the `--locked`-is-mandatory + never-`| tail` deploy rules
  (now in `AGENTS.md`). Verified: `ossctl release plan` still can't cut this two-crate workspace
  (`release-rust-workspace-multicrate` in ~/Sources/ossctl remains the blocker). ADR 0011 (homebase) boundary recorded
  in `AGENTS.md`: no pi-processes dependency.
- **Stint 2 (2026-08-16, triage-only).** 39 unscheduled issues → 24 closed, 15 laned; whole queue verified against
  current code. Two-thirds of the queue was not real work: 4 "bugs" were the same report-read-surface mistake, 11 were
  LLM review-residue with template bodies pointing at gitignored `history/` files, 5 duplicates. Origin of the
  filing-bar and verify-against-running-binary rules (now in `AGENTS.md`). Queue hygiene: `issuectl doctor --fix`,
  recovered real close-dates from git. `audit-no-user-specifics` arrived and was laned (skills lane): grep of the
  shipped artifact hits 19 files, five of them bundled SKILL templates; zero hits under `crates/*/src/`.
  *Left for Jari, outside this repo:* `~/.claude/skills/triage-bugs/` had dangling symlinks after the homebase rename
  to `triage-unlaned-issues`; fix on the homebase side if not already done.
- **Stint 1 (2026-08-16, v0.2.0).** The 0.2 simplification shipped end-to-end: thin supervisor (A1 exit-status shim,
  A2 OID-based merge-transaction recovery, A6 typed outcome table, A5 `attention_required`, A3 fenced `run salvage`,
  explicit `--interactive`), the teardown work-preservation guards, `config path`/`config show`, and the kind/heuristic
  cuts. Everything is documented as invariants 5–7 + the ADR (`docs/decisions/0001-thin-supervisor-vs-harden.md`).
- **Pre-0.2 (2026-07 → 2026-08-15).** The pivot: bug-cluster analysis showed ~57% of open issues concentrated in the
  supervisor/lifecycle subsystem with one root cause (state INFERRED from `pid × pane × branch × report`), so patching
  was stopped and the `lifecycle-architecture-review` epic ran instead — three research worktrees
  (`analysis.md`/`feature-audit.md` with 717-run usage evidence/`alternatives.md`), DECISION-1 (cut/keep/reframe, with
  Jari), a facilitated design session → `design.md`, DECISION-2 (thin model) → the ADR. v0.1.5–v0.1.8 shipped along the
  way. Durable residue: (a) "a subsystem whose bugs are combinatorial needs an architecture review, not more patches";
  (b) supervisor deaths under spawn saturation — the *surfacing* half shipped in 0.1.5 and the staged-create fix (0.2.2)
  removed the main trigger; the remaining resilience work is tracked as `create-idempotency-lease-recovery` + the
  stale-pendings issue; (c) the disjoint-lanes/integrated-gate and worker-deaths-transient learnings, both promoted to
  `AGENTS.md`.

---
