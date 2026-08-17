# TODO

Session-level plan + handoff. Longer-running planning/design docs live under
`issues/<slug>/{design,plan,breakdown,…}.md`; this file points at issuectl issues
for the actual tracked work. Standing rules and canonical learnings live in the
root `AGENTS.md` (operating policy + state-integrity invariants) — this file
holds only the **active handoff** and a **compact stint archive**.

---

## 🔄 Continue here (ALOITA TÄSTÄ), 2026-08-17 (**v0.2.2 SHIPPED + CI GREEN — NEXT = `skills` lane, head `release-gate-on-ci` or `audit-no-user-specifics`**)

**✅ LATEST (2026-08-17, stint 4 — read first).** A **full round + release + a caught mistake**: four parallel headless
spinoffs (one per lane), all landed on first spawn, no worker deaths; **v0.2.2 cut through every channel**; then CI
caught that one of the four fixes was **incomplete**, and a fifth spinoff finished it. crates.io has `octl-core 0.2.2`
+ `orchestratectl 0.2.2`; the `v0.2.2` tag's Release CI is **green**, the crates.io publish job is **green**, and
**main CI is green on all 8 jobs, both platforms**. Local binary **0.2.2**, `orchestratectl doctor` **997 ok / 0 warn
/ 0 fail**. `main` clean.

**What landed (5 units, 5 issues closed):**
- **`pi-spinoff-batch`** (supervise→lifecycle, high) — the load-bearing fix. `run create` now **stages** the prompt and
  durable projections outside the public run tree while `create.sh` blocks on workmux/tmux/harness startup, and
  atomically publishes only after a live worker PID **and** `node.created` are durable. So a client timeout under a
  concurrent batch can no longer leave a successful-looking `pending` manifest with zero nodes. Worker filed
  `create-idempotency-lease-recovery` for the residual (hard-kill mid-create leaves reclaimable staging state).
- **`cli-canon-help-json`** (cli-canon) — canon §14 closed. `--help --json` now emits the clap-**derived** help
  envelope (not a hand-maintained literal), and malformed output selectors are rejected. That lane is now empty.
- **`tmux-stub-etxtbsy-flake`** (multiplexer) — see the KEY LEARNING below; took **two** spinoffs.
- **`spinoff-skill-stale-preview-banner` + `skill-install-force-symlink`** (skills, one worker, two commits) — stale
  "NOT IMPLEMENTED" preview guidance removed; forced install now replaces a **dangling** symlink (non-following
  metadata, since `path.exists()` follows links).

**⚠ KEY LEARNING (kept HERE only, Jari's call — see note) — the local green gate runs on macOS and is STRUCTURALLY
BLIND to a whole failure class; CI is the only gate that sees it.** `tmux-stub-etxtbsy-flake` is ETXTBSY: Linux refuses
to exec a file while any process holds a write fd to it; **macOS does not enforce this at all**. So the first fix
(`sync_all` + `drop` before `chmod`) passed its worker's full local gate AND the orchestrator's integrated gate on
merged `main` (26 suites, 0 failures) — and CI still went red, with **two sibling tests** failing. The real cause was
one level deeper: a **cross-thread fork/exec race** — cargo runs these tests as parallel threads in ONE process, so
when thread A holds a write fd to its stub, thread B's `Command::spawn` forks and the child transiently inherits it;
`O_CLOEXEC` closes it only at *exec*, so a live process holds a write handle during that window. The second spinoff
confirmed this and fixed it structurally (a test-local mutex held from stub creation through the tmux calls),
**verified green on `test (ubuntu-latest)`**. Corollaries: (1) for a platform-specific class, a green local run is
**not evidence** — argue the fix on mechanism and confirm on CI; (2) "the tests pass" did not mean the fix was right,
twice in a row. *Note: Jari's call at this wrap was NOT to promote this into `AGENTS.md` — the machines move to Linux
shortly, so the blind spot resolves itself; kept here because it explains why the fix took two spinoffs.*

**⚠ Release-mechanics learning promoted to `AGENTS.md`.** v0.2.2 was published from a local `cargo publish` before CI
had reported on the commit — luck, not process. The finding: `publish-crates.yml` already publishes both crates from
CI, tag-triggered, so local publishing was redundant all along. The full rule ("DO NOT `cargo publish` locally — the
tag push IS the publish", the gated tag-push one-liner, and the `release-gate-on-ci` hole) now lives in `AGENTS.md`'s
operating policy; read it there before any release.

**⚠ LANE-STRUCTURE FIX (this wrap) — `supervise` was a phantom lane; merged into `lifecycle`.** `lifecycle` is defined
as "everything touching `run/*` or `supervise/*`", so a parallel `supervise` lane was definitionally overlapping. Not a
prediction — **verified from git**: the `supervise` lane's only work (`pi-spinoff-batch`) landed **entirely in
`crates/octl-cli/src/run/create.rs`** (247 lines), **zero files under `supervise/*`**. Its follow-up
`create-idempotency-lease-recovery` inherits that surface, so it moved to `lifecycle` (seq 5, now the lane head).
This is the same shape as the two integrated-main breakages in the archive below; the difference is it was caught
**before** a spawn. **`lifecycle` is deliberately NOT split** despite depth 12 — the `run/*` vs `supervise/*` split has
already failed once here (`supervisor-dies-before-worker-node`), so it stays the sequential spine and parallelism comes
from the other lanes.

**⏭ NEXT — `skills` lane.** Its head is `release-gate-on-ci` (filed this wrap), with `audit-no-user-specifics` right
behind it — **both high, both guarding something irreversible** (crates.io permanence vs. leaking user-specific facts
into a public artifact), and `audit-no-user-specifics` has now been head-adjacent for three stints. Pick either; they
are one lane so they sequence anyway. Cheap parallel partners: `surface`/`config-show-layered-view` and the
`lifecycle` head `create-idempotency-lease-recovery`.

**Lanes (3 now — `multiplexer` and `cli-canon` both drained this round and vanished; lanes derive from issue
frontmatter, so no cleanup was needed).** `lifecycle` (12), `skills` (5), `surface` (2). Realistic parallelism is
therefore **3 units/round**, which matches what actually ran. **`cli-canon` will return** as the canon grows (§19+) —
reuse the name.

**⚠ `add-configurable-agent` does NOT fit the lane model — read before scheduling it.** Jari's own feature request
(named agent profiles: `expert`/`standard`/`implementer`/`secure`, with fallbacks + config layering). Laned `surface`
seq 20, sequenced after `config-show-layered-view`, but it is **genuinely cross-cutting**: config surface **and**
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

**Triage sweep done at this wrap (`/triage-unlaned-issues`), DAG now 0 unlaned.**
- **Laned:** `intake-bug-orchestratectl-169460ea27e7` (stale pending runs → `lifecycle`) — admitted and **re-scoped**:
  this round's staged-create fix should stop NEW stillborn pendings, so what remains is (a) the ~7 already on disk and
  (b) making a stale pending distinguishable from a live worker in `run list`. Corroborated again: `run list` returned
  ~301KB dominated by old pendings, several from other repos. Plus `add-configurable-agent` (above).
- **Closed:** `intake-bug-orchestratectl-bb9e417520dd` (`node show` wrong arg order → `{}` with exit 0) as
  **cannot-reproduce** — verified against the **running 0.2.2 binary**: it now returns a proper
  `unknown_subcommand_or_flag` envelope with exit 1. Filed against 0.2.0.
- **Closed:** `run-wait-false-stillborn-slow-start` as **cannot-reproduce** on Jari's call — but note the precise
  grounds: it **did genuinely occur** when filed (a worker documented 4 spawns, all falsely declared dead, all
  recovering). It is closed because it **did not recur** — 5/5 waits correct this round after the staged-create fix,
  including a 50-min and a 9-min wait. Plausible mechanism (sampling a run before its node existed) but **not verified
  as causal**. Re-file readily if it returns.

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

## Piialiisan bugiraportit

- [x] 🐛 Auto-land an idle spinoff whose work is committed and merges cleanly — CLOSED wontfix 2026-08-14 (subsumed by the thin-supervisor ADR's manual-finish decision; re-file if a concrete need surfaces). ([`intake-feature-orchestratectl-0c37ae4b9e84`](issues/intake-feature-orchestratectl-0c37ae4b9e84/item.md))
- [x] 🐛 run show --output json surfaces terminal report as "none"; report lives in nodes/n-0001.json .last_report — CLOSED **duplicate** 2026-08-15 of Lane E `node-show-null-report` (same bug; intake adds that `run show`, not only `node show`, is affected — noted on the closed intake body). ([`intake-feature-orchestratectl-302ab43b3efd`](issues/intake-feature-orchestratectl-302ab43b3efd/item.md))
- [x] 🐛 run create timeout can leave a supervisorless pending run with no nodes — CLOSED **duplicate** 2026-08-16 of `run-create-long-title-stillborn`, which is the only one of five run-create-stillbirth reports with a deterministic repro (long `--title` → truncated branch name → `tmux-window-not-found`). ([`intake-bug-orchestratectl-dabe78632044`](issues/intake-bug-orchestratectl-dabe78632044/item.md))
- [x] 🐛 run wait --output json returns null terminal fields for a settled run — CLOSED **cannot-reproduce** 2026-08-16: not a bug. `run wait` emits `data.runs[]` (it can wait on many runs); the probe queried `.data.status`, which does not exist. `.data.runs[0].status` returns everything. ([`intake-bug-orchestratectl-eb2acb9686cb`](issues/intake-bug-orchestratectl-eb2acb9686cb/item.md))
- [x] 🐛 stale pending runs clutter run list and look like live workers — **ADMITTED + LANED** 2026-08-17 (`lifecycle`), re-scoped: the staged-create fix in 0.2.2 covers prevention, so this now owns (a) the ~7 stale pendings already on disk and (b) making a stale pending distinguishable from a live worker in `run list`. ([`intake-bug-orchestratectl-169460ea27e7`](issues/intake-bug-orchestratectl-169460ea27e7/item.md))
- [x] 🐛 node show accepts wrong argument order silently: returns {} with exit 0 — CLOSED **cannot-reproduce** 2026-08-17: verified against the running **0.2.2** binary, which returns a proper `unknown_subcommand_or_flag` envelope with exit 1. Filed against 0.2.0. ([`intake-bug-orchestratectl-bb9e417520dd`](issues/intake-bug-orchestratectl-bb9e417520dd/item.md))
- [x] 🐛 run create: per-run worker model override (harness args), without mutating global pi settings — CLOSED **duplicate** 2026-08-17 of `add-configurable-agent`, content transcribed first (per-run override as the MVP primitive, manifest/`run show` recording, and the racy global-settings workaround it must replace). ([`intake-feature-orchestratectl-d0c82ab27c9d`](issues/intake-feature-orchestratectl-d0c82ab27c9d/item.md))
