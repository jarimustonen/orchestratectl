# TODO

Session-level plan + handoff. Longer-running planning/design docs live under
`issues/<slug>/{design,plan,breakdown,…}.md`; this file points at issuectl issues
for the actual tracked work.

---

## 🔄 Continue here (ALOITA TÄSTÄ), 2026-08-16 (**v0.2.0 SHIPPED; NEXT = post-release cleanup / 0.2.1 planning**)

**✅ LATEST (2026-08-16: read first).** The 0.2 simplification release is fully shipped. This stint ran the
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

**⚠ Known follow-ups / not release blockers:** `run-create-long-title-stillborn` remains open (workaround: short
`--title`); Lane E read-surface bugs (`node-show-null-report`, `run-show-null-worktree-path`,
`count-jsons-swallows-io`) remain; `config-show-layered-view`, `run-salvage-fresh`, `enforce-run-merge`,
`run-show-landed-git-timeout`, teardown TOCTOU/ref-validation/in-progress-op/backpressure, and A2 recovery residuals
are intentionally deferred. A repo-wide cleanup is still desired for stale skill references in `AGENTS.md`; scheduling already lives in `issuectl dag`.

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
- [ ] 🐛 Piialiisan bugiraportti: run create timeout can leave a supervisorless pending run with no nodes — jari via Telegram ([`intake-bug-orchestratectl-dabe78632044`](issues/intake-bug-orchestratectl-dabe78632044/item.md))
- [ ] 🐛 Piialiisan bugiraportti: run wait --output json returns null terminal fields for a settled run — jari via Telegram ([`intake-bug-orchestratectl-eb2acb9686cb`](issues/intake-bug-orchestratectl-eb2acb9686cb/item.md))
