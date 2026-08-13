# Design — the 0.2 simplification (Lane F Phase 2 output)

**Status:** DESIGNED with Jari in the `arch-redesign-design-session` (2026-08-12/13),
facilitated `/llm-workshop`, **hardened by a 3-model expert critique** (gemini-3.1-pro,
gpt-5.6-sol, deepseek-v4-pro — all "revise": direction sound, negative-case mechanics
underspecified; findings A1–A6 below folded in). This document settles the **5 open
questions** in `target-state-0.2.md §5` — including **DECISION-2** (the surviving
supervisor core's model). It is the input to the ADR `arch-decision-rearchitect-vs-harden`
and to **◆ DECISION-2's** per-issue re-triage of the gated Lane A + Lane E backlog.

Grounded in the four Phase-1 docs: `analysis.md` (inference is the defect),
`feature-audit.md` (717-run usage evidence), `alternatives.md` (the thin/protocol
fork), `target-state-0.2.md` (DECISION-1 cut/keep/reframe).

**Design philosophy carried in (from `target-state-0.2.md §4`):** told-not-guessed;
clean-slate the model, keep the proven primitives; usage-scoped not capability-scoped;
typed/provable over heuristic. Overriding steer this session: **"simple, working is
what we're after."**

---

## 1. What was decided (the five open questions)

| # (target-state §5) | Question | Decision |
|---|---|---|
| 3 | Surviving supervisor core (= DECISION-2) | **Thin model (Option D)** + one minimal automatic crash-backstop + a **manual resume/finish skill**. Protocol (A+C) **deferred to 0.2.1 as a pi.dev plugin**, not cut. |
| 1 | Workflow packaging (skills vs fragments) | **Hybrid: thin, exec-able recipe skills over ONE generic spinoff engine.** Shared machinery lives once (in the CLI/Rust), never restated in prose. |
| 2 | How far to collapse `research`/`technical-decision` | Fully collapse at the engine level — both become **thin recipes**. **Web tools default-on for all workers** (no per-run tool-flag, no `research` engine path). Keep them as **two distinct recipes** ("more recipes is better"). |
| 4 | `--interactive` supervisor semantics | `--interactive` = supervisor **never** auto-terminalizes or auto-tears-down; waits for explicit `run merge`/`run cancel`; **dead pid ignored**. Human owns the whole lifecycle. |
| 5 | Migration / back-compat | **No back-compat — clean break in 0.2.0.** `doctor` sweeps stale **install-surface** artifacts; **run history is preserved untouched** (at most reported). |

Plus two cross-cutting decisions made this session:

- **Harness rule.** pi.dev is the **universal default** (autonomous *and* interactive).
  **claude is reachable only via `--interactive`, and even there is a non-default
  opt-in.** Autonomous runs are always pi.dev.
- **Release sequencing.** **0.2.0** = the simplification + pi.dev-as-default +
  manual resume skill. **0.2.1** = the pi.dev self-reporting plugin (the protocol path,
  built inside the harness, not sprinkled across SKILLs).

---

## 2. The supervisor core — the thin model (DECISION-2)

**Thesis (from `alternatives.md` Recommendation 1): the cure is *less* mechanism.**
The supervisor stops inferring the agent's state from a cross-product of proxies
(pid × pane × branch × report × three activity clocks). There is exactly **one
completion truth**, and it already exists and is durable:

> **A unit is DONE iff the worker called `orchestratectl run merge`** — which, under the
> run lock, rebases + merges + appends the durable `explicit-merge` transition. That
> append **is** the completion fact.

Everything the current watchdog does to *guess* done-ness is **deleted**:

- the git-reconcile-implies-done probe + synthetic `merge-reconciled` report,
- the three activity clocks (commit-time / pane-mtime / CPU-rate) and the CPU baseline
  window — the whole idle-unmerged synthesizer,
- the tmux tri-state / streak-gating / pane-aware liveness matrix as a *primary* signal,
- the lifecycle-branched liveness re-basing (there is no `Lifecycle::Interactive`
  category any more — see §4).

### 2.1 The completion signal: a real exit status, not a pid guess (A1)

**Critique finding (unanimous):** with only "pid gone", a worker that finished, exited
`0`, and forgot `run merge` is *indistinguishable* from a segfault — so a naive backstop
would auto-`fail` the exact `agent-skips-run-merge-idle-pending` case the design claims to
handle manually. The fix is already in `alternatives.md` Rec 3 (the "pairs with either
model" adjunct): **wrap worker launch in a thin launcher shim** (`octl-run-worker <run>
<node> -- pi …`) that `wait()`s on the child and records its **true exit status** as a
durable event under the run lock. This is a shim, not a protocol — no per-SKILL churn.

The supervisor then reads a *told fact*, not a guess:

- **non-zero exit / killed by signal** → `failed` (preserve branch, invariant 5).
- **exit 0 AND an `explicit-merge` event exists** → `done` + teardown.
- **exit 0 AND no merge event** → **stay `pending` / `attention-required`** — the worker
  finished but skipped `run merge`; hand to the manual finish skill (§2.2). **Not
  auto-failed** — the old safety-net case becomes a visible, resumable state, not a
  wrong terminal verdict.

### 2.1a The residual crash backstop + its bounded grace (A4)

The shim covers clean and signalled exits. The residual automatic rule covers only the
case where the shim itself is lost (supervisor never saw the exit event — hard kill of
the shim, host death):

> **process confirmed gone (pid + start-token identity) AND no exit event AND no merge
> event AND a fixed post-death grace elapsed → `failed`, preserve branch + worktree.**

The grace is **not** an activity clock (critique A4): it is a *fixed short window* (≈5 s)
anchored to the **first confirmed-death observation** (persisted, monotonic, survives
supervisor restart), whose only job is to let an in-flight merge/exit append finish.
Immediately before appending `failed`, the supervisor **re-reads the event log under the
exclusive lock** — if a merge or exit event landed in the race window, it wins. PID
identity uses the existing start-time/recycle defense (kept from today, `analysis.md`
Bucket 1) so a recycled pid can't silently disable the backstop.

### 2.1b Merge-transaction crash consistency (A2)

**Critique finding (unanimous):** `run merge` spans two durability domains — git refs and
the event log — and is not atomic across them. A crash after the git merge but before the
`explicit-merge` append leaves the work *merged on the source branch* with *no merge
event* → a false `failed`. The deleted git-reconcile probe used to catch this. The thin
model keeps **one narrow, deterministic recovery** (not the general reconcile): `run merge`
appends `merge.started{op_id, expected_source_oid, worker_oid}` **before** the git mutation
and updates the source ref with compare-and-swap semantics; on the next lock acquisition
the supervisor **finishes or rejects that one known transaction** by OID — no heuristic
branch inference, just completing a transaction it already recorded.

### 2.2 The manual resume/finish skill — ownership-fenced, agent-bypassing (A3)

The thin model deliberately drops the *automatic* rescue of "finished but idle-unmerged".
Its replacement is a **human-invoked skill** the PO drops into a stuck worktree — but,
per the unanimous critique, it must not "talk to" a wedged opaque agent (fragile
`tmux send-keys` guessing, useless against a stdin-blocked or CPU-looping process) and
must not spawn a second writer into the same worktree. Its contract:

- **Trigger:** manual — Jari, in the stint → PO-review → stint loop, sees an
  `attention-required` run (§2.5) and invokes the skill against that worktree.
- **Mechanism (fenced):** a `LockedRun`-witnessed operation — acquire the run lock,
  verify pid identity, then **fence the prior worker** (SIGTERM the stuck agent) and
  either (a) drive `run merge` **directly from the worktree's current git state**,
  bypassing the deadlocked agent, or (b) launch one fresh agent to continue — never
  alongside a live original. Refuses already-terminal nodes; idempotent against a
  duplicate merge.
- **Why manual is enough:** stuck runs surface at PO-review cadence *and* are made
  visible by §2.5 (not silently) — Jari noted independent demand for a "resume this
  worktree" command anyway. This is the never-shipped `run-salvage-command` (Bucket 3),
  generalized from dead-branch salvage to live-worktree resume.

### 2.3 What survives from today (the essential residue, `analysis.md §C.3`)

- the **crash-atomic event store** — `applied_seq` watermark, `LockedRun` witness,
  `LOCK_SH` reads (the one bug-free layer; keep exactly as is),
- a **terminal-outcome contract** — `run merge`'s durable `explicit-merge` transition
  + the new **exit-status event** (§2.1) + the terminal `node.report` (with
  `discussion_items[]` / `spinoff_proposals[]` retained as report fields; the mid-run
  discussion/spinoff *machinery* is cut per DECISION-1),
- the **merge-transaction record** (§2.1b) — `merge.started` op-log for deterministic
  merge-crash recovery,
- the **merge-assertion teardown gate** — invariant 5, `cleanup.rs` (branch/worktree
  preserved unless a confirmed `run merge`),
- **pid liveness as a pure crash backstop only** (§2.1a), never a primary signal.

### 2.5 Bounded visibility — the manual model needs eyes (A5)

The manual rescue only works if a stuck run is *visible without polling and without
terminalizing it* (unanimous critique — otherwise `run wait` never returns, the PO-review
cadence that is supposed to discover the jam is itself blocked, and one stuck fan-out
child starves the whole batch):

- **`run wait --timeout`** returns a distinct **non-terminal `attention-required`**
  result (does *not* mutate lifecycle) when a run is alive but has made no terminal
  transition past a bound — so a caller unblocks and surfaces it instead of hanging.
- **`run list` / `run show`** expose pending-age, last-observed pid, worktree path, and a
  one-line resume hint for any `attention-required` run.
- **per-node `run cancel <node>`** (branch-preserving) so a single stuck child can be
  unblocked without killing the fan-out; rollup can then terminalize the batch once every
  node is `merged | failed | cancelled`.

### 2.6 Typed outcome table — "merge is the only *success* truth", not the only *terminal* truth (A6)

`run merge` is the only **success** completion truth; it is not the only terminal state.
Make the negative outcomes a **typed transition table** (principle: typed over heuristic),
not an interpretation of signal combinations:

| Source | Outcome | Teardown? |
|---|---|---|
| `run merge` succeeds (`explicit-merge`) | `done` | yes |
| exit-status event non-zero / signal | `failed` | no (preserve branch) |
| exit 0, no merge event | `attention-required` (non-terminal) | no — manual finish |
| `run cancel` (run or `--node`) | `cancelled` | **no — preserve branch + worktree** (invariant 5) |
| confirmed-death backstop (§2.1a) | `failed` | no (preserve branch) |
| terminal `node.report{blocked}` | `blocked` (non-terminal, manual) | no |

`cancel` explicitly preserves work (never a teardown authorization) — closes the critique
gap that an implementer might read `cancel` as delete.

### 2.7 Why not the protocol now (and why 0.2.1)

The protocol model (A+C: worker self-reports `spawned→working→merging→merged|blocked|
failed` + renews a lease) is the *better mechanism*, but its cost is **worker
discipline** — every bundled SKILL must emit transitions at the right points, and a
skipped emit looks like a hang. That is exactly the "capability ahead of use" drag 0.2
is removing. Deferring it is not abandoning it: because pi.dev becomes the harness we
actually run, the protocol lands cleanly **as a pi.dev plugin in 0.2.1** — self-reporting
lives in *one* place (the harness) instead of being sprinkled across recipe skills. The
lease (the clean answer to the 9-issue supervisor-liveness bucket) rides in on that
plugin.

---

## 3. Engine end-state — 9 kinds → 2 topologies + a flag + recipes

The engine (Rust: schema/reducer/supervisor) knows only **topology**. Everything else
moves out of the engine.

| Today's kind | 0.2 disposition | Mechanism |
|---|---|---|
| **spinoff** | ✅ topology | the core; **always headless** (non-headless path removed) |
| **fan-out** | ✅ topology | genuine distinct need — N parallel independent units |
| **research** | → thin recipe | generic spinoff + `research` recipe |
| **technical-decision** | → thin recipe | generic spinoff + `adr` recipe |
| **bugfix** | → thin recipe | kind arm deleted; recipe kept |
| **make-skill** | → thin recipe | kind arm deleted; recipe kept |
| **code** | ❌ kind removed | value = interactivity → the `--interactive` flag (not a recipe) |
| **orchestrate** + **orchestrated** | ❌ removed entirely | multi-feature campaign absorbed by the stint loop (waves); no recipe — the stint *is* the recipe |

**Net:** the supervisor's *default* assumption is an **autonomous single-node worker**
(unless `--interactive`), collapsing the `Lifecycle` enum and ~24 kind-derived branch
points. The `Interactive` category stops being derived from `Kind` and becomes a single
explicit flag (§4).

**Fan-out is the one multi-node topology, and that is deliberate (critique A5).** The
critique flagged the apparent contradiction between "single-node supervisor" and retaining
fan-out. Resolution, settled before any reducer/schema deletion: **fan-out stays a single
run with N nodes** (not N independent runs — the batch needs one rollup, one wait, and
budgeted concurrency). So the reducer keeps its **node-set rollup + per-node teardown**
(that machinery is *essential*, not kind-derived accidental complexity); what is deleted
is the parent/child *cross-run* DAG bookkeeping of `orchestrate`/`orchestrated`. Per-node
`run cancel <node>` (§2.5) is what keeps one stuck child from starving the batch.

**Also cut (DECISION-1, executed here):** `pipeline` + `floor` (~20k LOC), the `harness`
heavy layer (`bakeoff` + `conformance` + `CodeHarness` trait + `aider` +
`claude-deepseek`), the `--harness` selection stack (4-level precedence resolver +
`manifest.harness`/`harness_source` provenance + DTO columns), and the mid-run
`discussion`/`spinoff` machinery (events, projections, counters, CLI subcommands) —
keeping only the terminal-report `discussion_items[]`/`spinoff_proposals[]` fields.

## 4. The workflow-recipe hybrid

Recipes are **thin, exec-able skills over one generic spinoff engine** — chosen for
runtime ergonomics, not just maintenance weight:

1. **On-demand context.** The worker exec's *only* its own recipe (`Skill(bugfix)`); the
   recipe loads into context when needed, not baked into every briefing.
2. **Named entry point + composability.** A recipe can call `/llm-review` +
   `/assess-findings`; a plain text fragment cannot.
3. **One place for the machinery.** Spawn / worktree / `run merge` / teardown live in the
   **CLI (Rust)**, behind verbs. Recipes never restate them — they reference the verbs.
4. **Light worker context (Jari's steer).** At runtime the in-worktree agent never loads
   the heavy spawn/supervisor/teardown mechanics — that is the orchestrator's + CLI's
   concern. Two audiences, two contexts.

Adding a recipe = writing one thin file (a few lines + a `/llm-review` call), so the
recipe library can grow freely ("more recipes is better"). Cost retained: each thin skill
keeps a `doctor` sync row + preview banner — cheap when the skill carries no duplicated
logic.

## 5. Harness

- **pi.dev = universal default** for every run, autonomous and interactive.
- **claude = the interactive, non-default opt-in.** In practice you reach for claude only
  under `--interactive`; an autonomous run defaults to pi.dev.
- The heavy `--harness` selection stack is gone (§3). The residual rule is this one
  sentence — no precedence resolver, no per-run provenance fields.

**Implement as a default/config, not a hard engine coupling (B1, per critique).** The
critique (gemini + openai) noted that hard-wiring "claude ⟺ interactive" into an engine
that otherwise "knows only topology" is itself a capability-scoped constraint. So the
rule lives as a **default** (pi.dev unless overridden) plus config, *not* as engine logic
that bars claude from an autonomous launch. The engine stays topology-only; the harness
choice is a launcher-level default Jari controls. The product intent (claude is the
interactive exception) is expressed as the default, not enforced by a special-case branch.

## 6. `--interactive` semantics

A single flag, not a second lifecycle:

- supervisor **never** auto-terminalizes and **never** auto-tears-down,
- it waits for an explicit `run merge` (→ teardown) or `run cancel`,
- a **dead pid is ignored** (human may have quit/restarted the agent — the ~1.5-day
  `agent-died-merge-no-teardown-interactive` case),
- no crash backstop, no idle net, no timeout — the human owns the whole lifecycle.

This is strictly *less* code than today's `Lifecycle::Interactive` branch: one flag that
says "supervisor, hands off — wait for explicit merge/cancel."

**Abandoned-interactive hygiene (critique).** Because an interactive run never
auto-terminalizes, a forgotten `run merge`/`run cancel` leaves durable pending runs +
worktrees + tmux windows forever. `doctor` (§7) therefore **reports** abandoned
interactive runs (pending past a long age, no live pid) with non-destructive resume/cancel
guidance — it flags them for Jari, it does not terminalize or delete them.

## 7. Migration — clean break + doctor sweep

**No backward compatibility.** 0.2.0 removes the cut kinds/flags/subcommands outright;
old invocations error. Justification: single-user internal tool, every call site
(bundled skills + Jari's own habits) updates in the same release, and a deprecation
window would mean carrying *both* surfaces — the exact dead-weight drag 0.2 removes.

`doctor` is the migration mechanism, scoped in two tiers:

- **Safe to prune (install-surface artifacts):** removed-kind skills stranded in
  `~/.claude/skills/` (`/worktree-bugfix`, `/orchestrate`, `/worktree-orchestrated`,
  `/worktree-make-skill`, pipeline skills), orphan companion files, dead `config.toml
  [harness]` sections, deregistered sync rows. Extends the existing skill-prune /
  orphan-companion detection to the newly-removed surfaces. Removed via an explicit
  `doctor` prune/fix action.
- **Never destroyed (history data):** `~/.orchestratectl/runs/*` directories whose kind
  was removed. This is the very evidence `feature-audit.md` mined (717 runs). `doctor`
  may **report** "N runs use a removed kind" but must not delete them.

**On-disk schema compat is a real, scoped cost (A6, critique).** "No CLI back-compat"
does *not* eliminate on-disk compat: preserving history while deleting kind variants /
events / DTO columns means normal enumeration or `run show` can crash on old data. So
0.2.0 ships a **read-only legacy decoder** — a permissive parser that reads pre-0.2 run
dirs into a reduced view (or reports raw metadata) so `doctor`/`run list` never fault on
them. This is a bounded, read-only parser, not a compat shim in the hot write path; scope
it explicitly rather than discovering it at first `run list` on old history.

## 8. Release plan

- **0.2.0 — the simplification.** Engine collapse (§3), thin supervisor (§2), `--interactive`
  flag (§6), pi.dev-as-default (§5), recipe hybrid (§4), manual resume/finish skill (§2.2),
  clean break + doctor sweep (§7). Breaking CLI change. Continue the in-flight pi.dev thread
  (`workmux-pi-agent-preset`, `config-subcommand`).
- **0.2.1 — the pi.dev self-reporting plugin.** The deferred protocol path (§2.4): worker
  transition self-reporting + a supervisor/worker lease, built inside pi.dev. Collapses the
  supervisor-liveness bucket if/when it lands.

Cadence per operating policy — release when a wave lands shippable user-facing work.

## 9. Relationship to ◆ DECISION-2 and the gated lanes

This design is the target the ADR (`arch-decision-rearchitect-vs-harden`) records and that
**◆ DECISION-2** re-triages the gated Lane A (26) + Lane E (3) issues against. Expected
dispositions once §3's cuts land:

- **OBSOLETE-as-subsumed:** the entire idle-unmerged/activity-clock family
  (`idle-unmerged-*`), the git-reconcile/synthetic-report machinery, the tmux tri-state /
  pane-aware liveness refinements, the lifecycle-branched re-basing, most of the run-show
  DTO re-derivation (Lane E) — their surface is deleted by the thin model.
- **KEEP-and-fix (fast-track, model-independent):** `merge-report-schema-lenience`
  (merge-first-then-validate — the terminal-report contract survives and must not reject a
  whole report over an advisory typo).
- **KEEP as the manual skill:** `run-salvage-command` → generalized into the resume/finish
  skill (§2.2).
- **DEFER to 0.2.1 (the plugin):** `peculiarly-cheerful-mine` (driver heartbeat/lease),
  `uncommonly-fuzzy-swing` (blocked→parent propagation), the supervisor-liveness bucket —
  these are the protocol's job, not the thin core's.

The formal per-issue verdict stays at DECISION-2 after the ADR; this section is the
predicted mapping, not the ruling.

---

## 10. Watched risks (noted, not designed away — Jari's call)

- **Web-tools default-on = exfil/prompt-injection surface (critique B2).** Every worker,
  including code-fix workers with no web need, can reach the network → data-exfiltration
  and prompt-injection exposure. Accepted for now (single-user tool; Jari confirmed "all
  can open web tools"). **What would change the call:** multi-user use, or handling
  untrusted repos/inputs → switch to recipe-declared static capabilities (research
  declares web; bugfix does not), which is also cleaner "told-not-guessed".

- **0.2.0 is a big-bang breaking release (critique B3).** It bundles the supervisor
  rewrite + ~20k-LOC deletion + schema/event removal + harness-default migration + recipe
  repackaging + migration tooling in one release → poor fault isolation. Accepted (Jari:
  "ei haittaa"). **Mitigation available if it bites:** sequence *within* 0.2.0 — land the
  subtractive cuts (pipeline/harness/kinds) first behind a green integrated gate, then the
  thin supervisor + exit-shim, then the recipe repackaging — so a regression is bisectable
  to one layer rather than the whole release.

---

## Expert thread

- group thread: `group_c292390c734842e89c7a1fe0e8ca2f12`
- gemini-3.1-pro-preview: `api_74fe455e631848a0b71ebb3fae300809`
- gpt-5.6-sol: `api_38f886f00b264d5e8a1fbcd5d2ae36ab`
- deepseek-v4-pro: `api_81a8758a56bb48738e80e0c6b00f5dad`

Continue any expert in context with `consult-llm -t <id>` if implementation surfaces a
question. All three verdicts were **revise** (direction sound; A1–A6 folded in above).
