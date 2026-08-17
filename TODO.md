# TODO

Session-level plan + handoff. Longer-running planning/design docs live under
`issues/<slug>/{design,plan,breakdown,…}.md`; this file points at issuectl issues
for the actual tracked work.

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

**⚠ KEY LEARNING #NEW (canonical) — the local green gate runs on macOS and is STRUCTURALLY BLIND to a whole failure
class; CI is the only gate that sees it.** `tmux-stub-etxtbsy-flake` is ETXTBSY: Linux refuses to exec a file while any
process holds a write fd to it; **macOS does not enforce this at all**. So the first fix (`sync_all` + `drop` before
`chmod`) passed its worker's full local gate AND the orchestrator's integrated gate on merged `main` (26 suites, 0
failures) — and CI still went red, with **two sibling tests** failing. The real cause was one level deeper: a
**cross-thread fork/exec race** — cargo runs these tests as parallel threads in ONE process, so when thread A holds a
write fd to its stub, thread B's `Command::spawn` forks and the child transiently inherits it; `O_CLOEXEC` closes it
only at *exec*, so a live process holds a write handle during that window. The second spinoff confirmed this and fixed
it structurally (a test-local mutex held from stub creation through the tmux calls), **verified green on
`test (ubuntu-latest)`**. Corollaries: (1) for a platform-specific class, a green local run is **not evidence** —
argue the fix on mechanism and confirm on CI; (2) "the tests pass" did not mean the fix was right, twice in a row.

**⚠ KEY LEARNING #NEW (canonical) — I published v0.2.2 BEFORE CI had reported on the commit it contained.** The release
was unharmed (the defect was test-only), but that was **luck, not process**, and crates.io publishes are permanent
(yank-only). Filed as **`release-gate-on-ci`** (skills, high) with the cheap fix: make CI a hard gate in the release
sequence — `git push && gh run watch "$(gh run list --branch main --limit 1 --json databaseId -q '.[0].databaseId')"
--exit-status && cargo publish …`. `--exit-status` is the load-bearing part: red CI breaks the `&&` chain so no publish
happens. Same discipline as the existing "never `| tail` a command whose exit status gates an `&&` chain" rule.

**⚠ LANE-STRUCTURE FIX (this wrap) — `supervise` was a phantom lane; merged into `lifecycle`.** `lifecycle` is defined
as "everything touching `run/*` or `supervise/*`", so a parallel `supervise` lane was definitionally overlapping. Not a
prediction — **verified from git**: the `supervise` lane's only work (`pi-spinoff-batch`) landed **entirely in
`crates/octl-cli/src/run/create.rs`** (247 lines), **zero files under `supervise/*`**. Its follow-up
`create-idempotency-lease-recovery` inherits that surface, so it moved to `lifecycle` (seq 5, now the lane head).
This is the same shape as the two integrated-main breakages already recorded below; the difference is it was caught
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
warning lives in the issue body instead.

**Triage sweep done at this wrap (`/triage-unlaned-issues`), DAG now 0 unlaned.**
- **Laned:** `intake-bug-orchestratectl-169460ea27e7` (stale pending runs → `lifecycle`) — admitted and **re-scoped**:
  this round's staged-create fix should stop NEW stillborn pendings, so what remains is (a) the ~7 already on disk and
  (b) making a stale pending distinguishable from a live worker in `run list`. Corroborated again: `run list` returned
  ~301KB dominated by old pendings, several from other repos. Plus `add-configurable-agent` (above).
- **Closed:** `intake-bug-orchestratectl-bb9e417520dd` (`node show` wrong arg order → `{}` with exit 0) as
  **cannot-reproduce** — verified against the **running 0.2.2 binary**: it now returns a proper
  `unknown_subcommand_or_flag` envelope with exit 1. Filed against 0.2.0. **Third time this round-shape has paid off:
  check the running binary, not the issue text.**
- **Closed:** `run-wait-false-stillborn-slow-start` as **cannot-reproduce** on Jari's call — but note the precise
  grounds: it **did genuinely occur** when filed (a worker documented 4 spawns, all falsely declared dead, all
  recovering). It is closed because it **did not recur** — 5/5 waits correct this round after the staged-create fix,
  including a 50-min and a 9-min wait. Plausible mechanism (sampling a run before its node existed) but **not verified
  as causal**. Re-file readily if it returns.

**— stint 3 (2026-08-17, v0.2.1) below —**

**✅ (2026-08-17, stint 3).** A **full round + release**: three parallel headless spinoffs, one per
lane, all landed on first spawn with no worker deaths, then **v0.2.1 cut through every channel**. crates.io has
`octl-core 0.2.1` + `orchestratectl 0.2.1`; the `v0.2.1` tag's Release CI is green with **12 assets**; the crates.io
publish job is green. Local binary **0.2.1**, `orchestratectl doctor` **954 ok / 0 warn / 0 fail**. `main` clean.

**What landed (3 units):**
- **`spinoff-report-fields-null`** (skills, high) — the four "reports persist as null" reports were **all read-surface
  errors**, re-verified against the actual run records this round: the node field is `last_report` (not `report`), and
  `run wait` emits `data.runs[]` while `run show` emits `data.<field>`. Fix: `node show` now also exposes
  `data.report`, `run show` exposes it for **single-worker** runs (intentionally `null` for fan-out — read each worker
  with `node show`), and the read-back guidance + a working `jq` probe went into **five** bundled skills
  (`octl-run-overview`, `worktree-spinoff`, `stint-start`, `stint-handoff`, `fan-out`). Additive only — no field
  renamed, no envelope reshaped. The `run wait`/`run show` envelope unification was deliberately NOT done (breaking).
- **`run-create-long-title-stillborn`** (lifecycle) — branch names bounded to workmux's **50-byte** window-name input,
  so the created window name and `create.sh`'s exact lookup can no longer diverge (`tmux-window-not-found` → stillborn
  `pending` run). Deliberately did NOT widen the lookup to a prefix match: a loose match can bind teardown to an
  unrelated window.
- **`cli-canon-version-schemas`** (cli-canon) — canon §10 closed. `version` now carries `supported_schemas` plus a
  named `supported_schemas_by_name` map (envelope/state/config/help/skill), derived from the real schema constants
  rather than a hardcodable literal, and `--json` is a global shorthand.

**Closed without code:** `cli-canon-config` — live re-verification showed §8 (`config path` / `config show` with
per-key `source` provenance) already shipped in 0.2.0; the issue body's "unrecognized subcommand" evidence predated
the stint-1 landing. **Second round running where verifying against the running binary, not the issue text, changed
the answer** — keep doing that before spending a worker.

**⚠ KEY LEARNING #NEW (canonical) — the documented local-deploy command was broken AND silently reported success.**
Two independent defects compounding:
1. `cargo install --path crates/octl-cli --force` **fails** on a 1.85 toolchain: `cargo install` re-resolves deps from
   scratch instead of using the workspace `Cargo.lock`, pulls `time 0.3.55` (requires rustc **1.88**), and dies. The
   `time = 0.3.41` pin from KEY LEARNING #1 protects the *workspace build* but **not** a fresh-resolving `cargo
   install`. **`--locked` is mandatory.**
2. The failure was invisible because the documented chain pipes to `tail`: `cargo install … | tail -3 && skill install
   && doctor` makes the shell see **tail's** exit status, so a failed install marches on and `doctor` reports green
   **about a binary that was never replaced**. On this machine `PATH` then resolved to the Homebrew tap binary, so
   everything looked deployed.
   Both are now fixed in `AGENTS.md` with the reasoning inline. Corollary: never `| tail` a command whose exit status
   gates an `&&` chain, and verify a deploy by `ls -l ~/.cargo/bin/orchestratectl` + `version`, not by `doctor` alone.
   Also: `skill install --force` does NOT refresh the **codex** skill home — that needs `--agent codex --force`
   (doctor showed 14 warns after the bump until it was run).

**⚠ `ossctl` still cannot cut this project's releases — verified this round, not assumed.** `ossctl release plan
--version 0.2.1` seals a plan whose `targets[]` contains **only `orchestratectl`**; `octl-core` is absent, so a cut
would publish the CLI against an `octl-core =0.2.1` that is not yet in the registry. The installed `release cut` also
has no `--bump` (the OSS-RELEASE.md text describes an ossctl 0.5.0 feature). So v0.2.1 was hand-cut the documented
way. `release-rust-workspace-multicrate` in **~/Sources/ossctl** remains the blocker to retiring hand-cut releases.

**◆ ADR 0011 (homebase, Accepted 2026-08-16) draws a binding boundary for this repo — recorded in `AGENTS.md`.**
Supersedes the closed `pi-background-jobs-extension`: we are **not** building our own pi extension. homebase
conditionally adopts pinned **`@aliou/pi-processes@0.10.9`** as its *interactive, session-scoped* pi runtime (gated on
a smoke matrix); the *durable, harness-neutral* runner is separate work (`orx-background-runner`, tracked in homebase,
blocked on that gate). **orchestratectl must not import pi-processes, touch its manager, assume its ids/log paths, or
use its in-process EventBus** — we stay the run-state owner behind `run wait` / `landed` / the JSON contracts. Open
question left for Jari: whether the durable runner contract eventually moves here; recommendation was to leave it in
homebase until the gate opens.

**⏭ NEXT — `multiplexer` lane, head `tmux-stub-etxtbsy-flake` (priority raised to high this wrap).** It made **main CI
red on the v0.2.1 release commit itself** (`multiplexer::tmux::tests::new_session_surfaces_nonzero`, ETXTBSY on
ubuntu): the test writes an executable stub and spawns it while the write handle is still open. Linux-only race, passes
on rerun, so `main` CI is green again — but it will keep reddening release commits until fixed. It is test-only and
disjoint from every other lane, hence its own lane. After that the cheap wins are `skills`
(`spinoff-skill-stale-preview-banner`, `skill-install-force-symlink`) and `cli-canon-help-json`.

**Lanes (6 now).** `skills` (6), `lifecycle` (10), `cli-canon` (1 — two of three closed this round), `surface` (1),
plus two that arrived from parallel sessions: `supervise` (1 — `pi-spinoff-batch`) and `multiplexer` (1, new this
wrap). Note `lifecycle`'s head is still `uncommonly-fuzzy-swing` because **priority outranks `lane_seq`**; it is the
lane's most valuable item but not the cheapest start. **`uncommonly-fuzzy-swing` and `spinoff-report-fields-null` were
deliberately NOT run in parallel this round** — both work the terminal-report surface, exactly the lane-misprediction
trap; that constraint is now gone since the report work landed.

**New follow-up filed by a worker:** `run-branch-name-ulid-entropy` (lifecycle, seq 25) — `derive_branch_name` takes
the first 10 ULID chars, which encode the **millisecond timestamp, not entropy**, so two runs created in the same
millisecond with similar titles can derive the same branch name. Carries a repro, so it clears the filing bar from
stint 2's learning.

**❗ Untriaged intake still waiting (surfaced at this wrap, NOT folded).**
`intake-bug-orchestratectl-169460ea27e7` — *stale pending runs clutter run list and look like live workers*
(`via:agent-homebase-wrapup`, `needs-triage`). **Corroborated directly this session:** `run list` shows **7 `pending`
runs**, oldest from **24 July**, several belonging to other repos entirely — indistinguishable at a glance from live
workers. This actively degrades the signal `/stint-start`'s preflight reads to decide what is unsettled. Recommend
admitting it; it stays `needs-triage` until Jari acks.

**— stint 2 (2026-08-16, triage-only) below —**

**✅ (2026-08-16, stint 2).** A **triage-only stint**: no product code touched, no worktrees
spawned, no release cut. Release state is unchanged from stint 1 (**v0.2.0** everywhere; see the stint-1 block below).
The whole issue queue was swept against ADR 0010 (`open ∧ ¬laned`) and every claim was **verified against current
code**, not taken from the issue text. `main` clean, 0 unpushed. Commits: `abb5cce`, `6d1d230`, `5f367e1`, `8a81025`.

**Result: 39 unscheduled issues → 24 closed, 15 laned** (18 open non-epic issues, all in `issuectl dag`; the 2
remaining `unscheduled` entries are epics, which are containers, not schedulable units).

**`audit-no-user-specifics` arrived at wrap and is laned** (task, priority **high**, filed from project-canon during
this session, commit `33f2279`) — a family-wide sweep for user-specific facts in a public artifact, after 0.1.1/0.2.0
shipped a gh account, a personal repo-root convention, and three private repo names to crates.io as built-in defaults.
Scoped before laning: grepping the shipped artifact hits **19 files, five of them bundled SKILL templates**, so it
went into `skills` (it cannot run in parallel with that lane). **Zero hits under `crates/*/src/`** — unlike
project-canon, this repo does not appear to have leaked user-specifics into built-in defaults; a lead, not a verdict,
since the audit still owes tests and fixtures. The issue carries the full file list and the distinction the auditor
needs: most hits are the repo's *legitimate* public coordinates (GitHub URL, Homebrew tap), not leaks.

**Why so many closed — the load-bearing finding.** Two thirds of the queue was not real work:
- **4 issues were never bugs at all — all four are the same mistake.** `run wait` emits `data.runs[]` (it can wait on
  many runs) while `run show` emits `data.<field>`; and the node projection's field is **`last_report`, not `report`**.
  Reports queried `.data.status` and `.report.summary`, got `null`, and were filed as data loss. Verified against the
  actual runs: every report was intact, summaries and `wrap_up_recommendations` included. Affected
  `node-show-null-report`, `intake-…eb2acb9686cb`, and the high-priority `spinoff-report-fields-null`.
  **The real defect: the bundled skills document how to WRITE a report and never how to READ one back** — `last_report`
  appears in no SKILL file. That is now `spinoff-report-fields-null`, re-scoped, with the verification table attached.
- **11 were LLM review-residue.** Three (`atomic-source-ref`, `run-merge-recovery`, `run-merge-recovery-2`) shared a
  word-for-word 3-line template body whose only content pointed at `history/review-merge-transaction-recovery.md` —
  a file that does not exist (`history/` is gitignored), making them unactionable. The rest needed a crash inside a
  two-write window, a recycled pid on a phased-out file format, an IO error in our own home dir, or a hung git on NFS
  we do not have; two stated **in their own text** that the problem was unconfirmed or unreachable.
- **5 were duplicates**, 3 out of scope, 1 already fixed by `run salvage`, 1 shipped (`--notify`).

**⚠ KEY LEARNING #NEW (canonical) — an automated review pass that files every "deferred residual" as an issue
manufactures a backlog of un-work.** 13 of the 24 closures came from the `/llm-review` + `/assess-findings` cascade.
The failure mode is not bad judgment about severity; it is **filing without content**: a template body pointing at a
machine-local, gitignored review artifact that is gone by the time anyone reads the issue. Rule going forward: a
review residual becomes an issue only if it has **(a) an observed occurrence** or **(b) a self-contained, readable
description**. Never a bare pointer to a `history/` file. This matches `/stint-handoff`'s standing "scrutinise
spin-off quality before folding" discipline — apply it at *filing* time too, not only at folding time.

**⏭ NEXT — start at the `skills` lane, head `spinoff-report-fields-null`.** It is high-priority, cheap (documentation
plus a `report` alias on `node show`, optionally unifying the `run wait` / `run show` envelope shape), and it stops a
recurring drain: four separate false bug reports have now come from that one undocumented read surface. The rest of
that lane is small and independent (`spinoff-skill-stale-preview-banner` — the "NOT IMPLEMENTED" banner is still live
and in **two** skills, not one; `skill-install-force-symlink` — the **dangling**-symlink case is still real,
`path.exists()` follows links; `consult-failure-hard-fail` — kept and re-scoped on Jari's call to prose in four SKILL
templates, phrased generically so the open-source skills stay project-neutral; `stint-skills-drop-intake-specifics`).
The three cheapest wins across the whole DAG are roughly one worktree together.

**Lanes (4, split so no two touch the same hot-file family):** `skills` (5 — `crates/octl-cli/skills/*` + `skill.rs`),
`lifecycle` (9 — everything touching `run/*` or `supervise/*`, sequenced), `cli-canon` (3 — unchanged from stint 1),
`surface` (1 — config only). Note `lifecycle`'s head is `uncommonly-fuzzy-swing` because **priority outranks
`lane_seq`** in `issuectl dag`; that is correct (it is the lane's most valuable item), but it is not the cheapest
start — lower its priority if you want to warm up on `shell-quote-dedup` (now **three** copies, not two).

**🧹 Queue hygiene done this stint.** `issuectl doctor --fix`: 16 `## Notes` → `## Comments`, and `.issuectl/AGENTS.md`
regenerated — it had been missing eight frontmatter fields including **`lane`**, the very field the DAG runs on.
`signal-exit-143-regression` carried status `closed`, a value **not in the schema enum**, so issuectl never classified
it as terminal and it kept surfacing as unlaned; set to `fixed`. Eight pre-issuectl closures had no `closed:` date —
recovered the real dates from git history (all June 2026, 13.–29.6.) rather than stamping a placeholder. `issuectl
doctor` is now clean apart from `arch-supervision-alternatives`'s undeclared `deliverable` key, which is not an error.

**❗ Outside this repo (for Jari).** `~/.claude/skills/triage-bugs/` is left with dangling symlinks after the homebase
rename to `triage-unlaned-issues` (homebase `fd14a240`), and the new skill is not installed into `~/.claude/skills/`
yet — this stint ran its helper straight from the homebase source. Fix on the homebase side.

**— stint 1 (2026-08-16, the v0.2.0 release) below —**

The 0.2 simplification release is fully shipped. This stint ran the
thin-supervisor build and selected safety/robustness follow-ups end-to-end, then cut **v0.2.0** through every channel.
`main` is clean, pushed, tagged `v0.2.0`; crates.io has **`octl-core 0.2.0`** and **`orchestratectl 0.2.0`**;
GitHub Release / cargo-dist assets are published at `https://github.com/jarimustonen/orchestratectl/releases/tag/v0.2.0`;
Release CI and main CI are green. Local binary is **0.2.0** and local deploy/doctor is clean after installing all
skill homes: `orchestratectl doctor` **898 ok / 0 warn / 0 fail**. No new Telegram intake surfaced at handoff.

**What landed this stint (0.2 core):**
- **Thin supervisor model implemented:** A1 worker exit-status shim, A2 OID-based `run merge` transaction recovery,
  A6 typed outcome table, A5 `attention_required` read/wait surface, A3 fenced `run salvage`, and explicit
  `run create --interactive`. The old inference-by-activity/branch/pane success heuristics are gone; `run merge` is
  the only success truth, while negative/manual states are typed and branch-preserving.
- **Release-blocking safety/robustness follow-ups landed:** dirty-worktree and detached-HEAD committed-work preservation
  on non-merge teardown, per-node branch-preserving `run cancel --node`, log-authoritative leaf rollup, typed
  `ReportOrigin` + retired forgeable `via` merge authority, raw-git self-merge false-failed hint, and lenient advisory
  report validation for `run merge --report-file`.
- **pi.dev / skill surface completed for 0.2:** `config path` + `config show`, workmux `pi` preset confirmed obsolete
  because current workmux has built-in `pi`, and bundled `stint-start`/`stint-handoff` descriptions trimmed under
  pi.dev's 1024-char limit with a guard test.
- **0.3 decision recorded:** pi.dev non-blocking waits should be a separate **`pi-background-jobs`** extension/repo,
  not orchestratectl core. Issue `pi-background-jobs-extension` is filed and **deferred** for 0.3.

**⏭ NEXT.** The immediate 0.2 release train is done. Resume with ordinary post-release cleanup and 0.2.1 planning:
start at Lane D `skill-install-force-symlink` unless Jari wants the repo-wide cleanup first. The 0.2.1 supervisor
thread is mostly deferred design/lease/plugin work: pi.dev self-report/heartbeat lease, blocked-on-user-input propagation,
run-create resilience, teardown backpressure/lease hardening, child-run log-authoritative rollup, and config layered
inspection. Do **not** reopen the 0.2 refactor release path unless CI or release verification regresses.

**⚠ Known follow-ups (SUPERSEDED by stint 2's triage — see the block above; kept for context).** This list named
`node-show-null-report`, `count-jsons-swallows-io`, `run-salvage-fresh`, `run-show-landed-git-timeout`, and the
teardown TOCTOU / ref-validation / in-progress-op / backpressure residuals as "intentionally deferred" — all are now
**closed**, most as unactionable or unreachable. Still open and laned: `run-create-long-title-stillborn` (workaround:
short `--title`), `run-show-null-worktree-path`, `config-show-layered-view`, `enforce-run-merge`. A repo-wide cleanup
of stale skill references in `AGENTS.md` is still desired; scheduling lives in `issuectl dag`.

**— historical below (this session's earlier waves + canonical KEY LEARNINGs, still load-bearing) —**

Wave 1 of the architecture re-examination shipped, then a
full **DECISION-1** co-design with Jari in the PO review.
- **v0.1.7 FULLY SHIPPED** (crates.io `octl-core`→`orchestratectl`, `v0.1.7` tag → Release CI green all jobs,
  Homebrew tap 0.1.7): `agent-skips-run-merge-idle-pending` + `ci-docs-bakeoff-registry-link` +
  `doctor-codex-companion-coverage`. CHANGELOG `[Unreleased]`→`[0.1.7]`. Local binary **0.1.7**, `doctor` **763/0**.
- **Wave 1 — 5 headless workers, ALL landed on first spawn, no deaths.** Lane F Phase-1 trio (read-only research):
  `arch-lifecycle-map-rootcause` (→ `analysis.md`, 450 lines — the inference-vs-protocol root cause + 28-issue
  taxonomy), `arch-feature-usage-audit` (→ `feature-audit.md`, 311 lines — 717-run usage evidence, bias-to-cut),
  `arch-supervision-alternatives` (→ `alternatives.md`, 505 lines — thin vs protocol vs FIFO). Plus Lane B pi.dev
  `harness-pi-skill-shim` (pi worker completes research end-to-end) + Lane D `pidev-pi-skill-lifecycle` (pi mirror
  provenance + prune + doctor drift). Integrated gate green (supervise 727/0, all suites 0 failed). `main` clean, 0 unpushed.
- **The 2 research workers skipped their own issue-close** (reports landed, `--status done` not called) → closed as
  orchestration bookkeeping. All 5 issues terminal.

**◆ DECISION-1 DECIDED (2026-08-12, with Jari in the PO review) → `issues/lifecycle-architecture-review/target-state-0.2.md`.**
The feature-audit landed and Jari + the orchestrator did the full DECISION-1 deliberation (every linchpin
code-verified). Outcome (full detail in target-state-0.2.md):
- **Working model confirmed: stint → PO review → stint.** Autonomous spinoffs; review at the round/PO level;
  interactivity reached for occasionally.
- **The reframe:** today's 9 kinds conflate **three orthogonal axes** — *topology* (spinoff/fan-out, stays),
  *how-run* (autonomous vs interactive → an explicit `--interactive` flag, NOT a kind), *workflow* (research/fix/ADR
  → skills/prompt-fragments, NOT kinds). Unifying principle: **"told, not guessed"** (explicit state over inferred).
- **CUT (decided):** `orchestrate`+`orchestrated`, `code`, `bugfix`+`make-skill` (as kinds); `pipeline`+`floor`;
  harness `bakeoff`+`conformance`+`CodeHarness` trait+`aider`+`claude-deepseek` (keep the light claude+pi launcher);
  the mid-run `discussion`/`spinoff-proposal` machinery (keep the terminal-report `discussion_items[]`/`spinoff_proposals[]`).
- **CHANGE:** `spinoff` always headless (remove the non-headless path).
- **KEEP:** spinoff, fan-out, research, technical-decision (≥ as skills), claude+pi launcher, the crash-atomic
  event store + `run merge` + teardown gate, interactivity as a flag. Cutting `code` (after `orchestrate`) empties
  `Lifecycle::Interactive` → the kind-derived lifecycle *inference* collapses (~24 supervisor branch points; the
  accidental complexity `analysis.md` §C.3 named).
- **Design philosophy for the redesign:** clean-slate the MODEL / keep the proven primitives; told-not-guessed;
  usage-scoped not capability-scoped; typed-over-heuristic. **0.2 = the simplification + pi.dev** (one release).

**🧭 THE PIVOT (Jari, 2026-08-12) — STOP patching the lifecycle core; RE-EXAMINE the architecture.** A bug-cluster
analysis of all 44 open issues showed **~57% (and 58% of bugs) concentrate in one subsystem: supervisor / agent
lifecycle / liveness / teardown.** Within it the same root cause recurs — the supervisor **INFERS** a distributed
process's state from indirect signals (`pid × pane × branch × report`), so every new signal-combination is a new
edge case and patching never shrinks the list (the agent-skips fix above *immediately spawned 3 more* cluster-A
follow-ups — textbook). **Jari also flagged: actual usage is NARROW — some options likely aren't needed** (drag).
Response: filed epic **`lifecycle-architecture-review`** + 5 tasks (**Lane F**, now the GLOBAL HEAD) — map+root-cause,
feature-usage/drag audit (HIGH, bias-to-cut), alternatives survey → design session WITH Jari → an ADR
(harden vs re-architect). **Lanes A (26, supervisor core) + E (3, run-show DTO) are ⛔ GATED behind ◆ DECISION-2** —
no new cluster-A/B fixes until the ADR decides each issue's disposition. Non-core lanes (B pi.dev/pipeline, D skill)
proceed in parallel. The full plan (all 47 issues in lanes, ◆ decision points, ⬆ release nodes, next-waves) is the
DAG + Wave plan below.

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

**KEY LEARNING #4 (canonical) — when a subsystem's bugs are COMBINATORIAL, stop patching and review the architecture.**
The supervisor/agent-lifecycle core accreted ~25 open issues because it INFERS a distributed process's state from
indirect signals (`pid × pane × branch × report`); each fix closes one signal-combination and reveals the next
(the agent-skips CPU-clock fix spawned 3 more idle-unmerged follow-ups the same day). A per-bug loop can't shrink a
combinatorial edge-case space — the honest move is to review the model (inference-by-polling vs. protocol/state-machine
where the worker self-reports transitions) and to audit whether narrow real usage even needs all the surface. This is
why Lanes A + E are gated behind the architecture ADR (◆ DECISION-2) instead of being worked head-by-head. Corollary
for triage: a cluster where ">half the open issues share a root cause" is an architecture signal, not a backlog.

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

**RELEASE STATE.** crates.io + GitHub binaries + Homebrew tap all coherent at **0.1.8** (shipped 2026-08-13).
CHANGELOG `[Unreleased]` now **accumulates the v0.2.0 breaking changes** (the pipeline/harness cut, pi-provenance
v3, the snapshot-guard, the in-progress-eligible SKILL realignment) — do NOT patch-release these. **Next release
is v0.2.0 = the DECISION-1 simplification + the pi.dev thread** (Jari's call — one release, no separate 0.3). pi.dev remainder toward
0.2: `workmux-pi-agent-preset`, `config-subcommand`. **Release autonomy (Jari): cut autonomously at the right moments
— DON'T ask, DON'T re-confirm** (release fully autonomous, `main`-push always allowed, `pull→rebase→push` always
allowed — root `AGENTS.md`). **We are RETIRING hand-cut releases** (Jari, 2026-08-12 — fix it, don't document the workaround): the fix is
filed as `release-rust-workspace-multicrate` in **~/Sources/ossctl** (make `ossctl release` handle the
dependency-ordered two-crate publish + version bump + snapshot regen). **Prefer closing that over cutting more by
hand.** Until it lands, v0.1.7 is cut the same TEMPORARY way 0.1.1–0.1.6 were: two-crate order
`octl-core`→`orchestratectl` (pin `=<version>`) — one `release: vX.Y.Z` commit bumping `Cargo.toml` workspace version
+ octl-cli's octl-core pin + CHANGELOG (+ regenerate the restaled `envelope_snapshots__version_{text,json,jsonl}`
insta snapshots, stripping insta's volatile `assertion_line:` header), push, `cargo publish` both, tag `vX.Y.Z` →
Release CI on `hauis`. `hauis`-runner git-400 playbook: `peculiarly-madly-sneeze` (closed).

**✅ DONE 2026-08-13 — the DESIGN SESSION (Lane F Phase 2) — see the LATEST block at top; output `design.md`, DECISION-2 = thin model.**
`arch-redesign-design-session` — a facilitated `/llm-workshop` **WITH Jari** (interactive, not headless). Its input was
**`target-state-0.2.md`** (DECISION-1 outcome + the design philosophy + the 5 open questions) plus the three Phase-1
reports (`analysis.md` / `feature-audit.md` / `alternatives.md`). It settles: workflow packaging (skills vs
prompt-fragments), how far to collapse research/technical-decision, and **the surviving supervisor core's model** —
`alternatives.md`'s fork (thin vs protocol vs FIFO) = DECISION-2 itself → `design.md` → the ADR
(`arch-decision-rearchitect-vs-harden`). **Sequencing:** run DECISION-1's cuts FIRST — they pre-shrink DECISION-2 (many
Lane A/E issues obsolete when their surface is cut); formal per-issue re-triage stays at ◆ DECISION-2 after the ADR.
**Do NOT spawn Lane A / Lane E work** — still ⛔ gated. The design session deserves fresh context, so it is the next
stint's focus. DAG drift-clean at wrap (42 active issues, all in lanes, nothing outside). No worktrees remain;
**`main` clean, 0 unpushed, local binary 0.1.7 (`doctor` 763/0)**.

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

The 5 state-integrity invariants and the `/stint` operating policy (deploy /
green-gate / hot files) live in the root `CLAUDE.md` / `AGENTS.md`. Read them before
touching the reducer, lock layer, or `supervise/`. (The `harness/floor/pipeline` modules
were DELETED 2026-08-14 by `cut-pipeline-floor-harness-heavy` — only the light
`harness/{mod,prompt,select,support}.rs` claude+pi launcher remains.)

## Piialiisan bugiraportit

- [x] 🐛 Auto-land an idle spinoff whose work is committed and merges cleanly — CLOSED wontfix 2026-08-14 (subsumed by the thin-supervisor ADR's manual-finish decision; re-file if a concrete need surfaces). ([`intake-feature-orchestratectl-0c37ae4b9e84`](issues/intake-feature-orchestratectl-0c37ae4b9e84/item.md))
- [x] 🐛 run show --output json surfaces terminal report as "none"; report lives in nodes/n-0001.json .last_report — CLOSED **duplicate** 2026-08-15 of Lane E `node-show-null-report` (same bug; intake adds that `run show`, not only `node show`, is affected — noted on the closed intake body). ([`intake-feature-orchestratectl-302ab43b3efd`](issues/intake-feature-orchestratectl-302ab43b3efd/item.md))
- [x] 🐛 run create timeout can leave a supervisorless pending run with no nodes — CLOSED **duplicate** 2026-08-16 of `run-create-long-title-stillborn`, which is the only one of five run-create-stillbirth reports with a deterministic repro (long `--title` → truncated branch name → `tmux-window-not-found`). ([`intake-bug-orchestratectl-dabe78632044`](issues/intake-bug-orchestratectl-dabe78632044/item.md))
- [x] 🐛 run wait --output json returns null terminal fields for a settled run — CLOSED **cannot-reproduce** 2026-08-16: not a bug. `run wait` emits `data.runs[]` (it can wait on many runs); the probe queried `.data.status`, which does not exist. `.data.runs[0].status` returns everything. ([`intake-bug-orchestratectl-eb2acb9686cb`](issues/intake-bug-orchestratectl-eb2acb9686cb/item.md))
- [x] 🐛 stale pending runs clutter run list and look like live workers — **ADMITTED + LANED** 2026-08-17 (`lifecycle`), re-scoped: the staged-create fix in 0.2.2 covers prevention, so this now owns (a) the ~7 stale pendings already on disk and (b) making a stale pending distinguishable from a live worker in `run list`. ([`intake-bug-orchestratectl-169460ea27e7`](issues/intake-bug-orchestratectl-169460ea27e7/item.md))
- [x] 🐛 node show accepts wrong argument order silently: returns {} with exit 0 — CLOSED **cannot-reproduce** 2026-08-17: verified against the running **0.2.2** binary, which returns a proper `unknown_subcommand_or_flag` envelope with exit 1. Filed against 0.2.0. ([`intake-bug-orchestratectl-bb9e417520dd`](issues/intake-bug-orchestratectl-bb9e417520dd/item.md))
- [ ] 🐛 Piialiisan bugiraportti: run create: per-run worker model override (harness args), without mutat… — jari via Telegram ([`intake-feature-orchestratectl-d0c82ab27c9d`](issues/intake-feature-orchestratectl-d0c82ab27c9d/item.md))
