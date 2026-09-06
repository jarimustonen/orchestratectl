# Feature-usage / dead-weight drag audit

**Issue:** `arch-feature-usage-audit` (epic `lifecycle-architecture-review`, PHASE 1)
**Date:** 2026-08-12 · **Method:** read-only, headless · **Bias:** toward cutting
**Feeds:** DECISION-1 (dead-weight cut)

> PRIMARY-USER STEER (Jari, 2026-08-12): *"we have quite limited use cases; it's
> very possible some options really aren't needed."* This audit grounds the
> keep/cut call in **observed** usage, not in what the surface theoretically
> supports.

---

## 1. Executive summary

The tool exposes **9 run-kinds**, **2 additive command trees** (`pipeline`,
`harness`), a **4-harness selection/bakeoff subsystem**, and two collaboration
subsystems (**discussions**, **spin-off proposals**). Measured against 717 real
runs and every bundled/installed workflow skill, the actual use-set is **narrow**:

- **83 % of all runs (595/717) are `spinoff`.** In the most recent 120 runs it is
  **96 %** (115 spinoff, 4 research, 1 technical-decision).
- **Three kinds carry essentially all traffic:** `spinoff`, `research`,
  `technical-decision`. `code`, `orchestrated`, `orchestrate`, `fan-out`,
  `make-skill`, `bugfix` are rare-to-never in the recent window.
- **The entire multi-harness machinery is idle:** `--harness` is invoked by **0
  skills** and **0 of 717 runs** ever recorded a non-`claude` harness. `pipeline`
  and `harness` command trees are referenced by **0 workflow skills**.
- **Discussions and spin-off proposals are near-dead:** only **4/717** runs have
  any discussion content, only **5/717** have any spin-off-proposal content.

The dominant drag is not the run-lifecycle core (which every spinoff exercises) —
it is **~30 000 LOC of additive subsystems (`pipeline` + `harness` + `floor`) that
no workflow invokes**, plus per-kind branching in the supervisor/schema/reducer for
kinds that fire a handful of times a month or never.

**Headline removal candidates (see §5):** the `pipeline`/`floor` code-pipeline, the
`harness` bakeoff + `--harness` selection (aider/pi/deepseek adapters), and — pending
Jari's confirmation — the `bugfix`, `make-skill`, and possibly `fan-out`/`orchestrate`
kinds and the discussions/spin-off-proposal subsystems.

---

## 2. Evidence base (what I could gather headlessly)

### 2.1 Run-kind frequency — all 717 runs on disk (`~/.taskfleet/runs/*/manifest.json`)

| kind | count | share |
|---|---:|---:|
| **spinoff** | **595** | **83.0 %** |
| orchestrated | 30 | 4.2 % |
| code | 25 | 3.5 % |
| research | 21 | 2.9 % |
| fan-out | 20 | 2.8 % |
| technical-decision | 16 | 2.2 % |
| orchestrate | 6 | 0.8 % |
| make-skill | 4 | 0.6 % |
| **bugfix** | **0** | **0.0 %** |

Run history spans **2026-07-09 → 2026-08-12** (~5 weeks).

> **`bugfix` has NEVER been run** despite being a full run-kind with its own skill,
> schema variant, lifecycle, and supervisor path. `orchestrate` (the DAG driver) and
> `make-skill` are in single digits over five weeks.

### 2.2 Recency — the last 120 runs by mtime

| kind | count |
|---|---:|
| spinoff | 115 |
| research | 4 |
| technical-decision | 1 |

The **live** working set is three kinds, overwhelmingly `spinoff`. `code`,
`orchestrated`, `orchestrate`, `fan-out`, `make-skill`, `bugfix` do not appear in the
recent 120 at all.

### 2.3 August-only distribution (this stint's activity)

`spinoff` 338 · `research` 14 · `technical-decision` 11 · `orchestrated` 4 · `code` 4
· `orchestrate` 2 · (`fan-out` 0, `make-skill` 0, `bugfix` 0).

### 2.4 What the workflow skills actually invoke

`--kind` values across bundled `SKILL.template.md` files (each kind has a dedicated
skill, so presence ≠ usage — cross-reference with §2.1/§2.2):

`spinoff` 14 · `orchestrated` 10 · `fan-out` 7 · `research` 4 · `orchestrate` 4 ·
`code` 4 · `technical-decision` 2 · `make-skill` 2 · `bugfix` 2 (+ stray
`orchestrator`/`discuss` strings in prose).

### 2.5 `run create` flag usage across bundled skills

| flag | # skills | verdict |
|---|---:|---|
| `--report-file` | 12 | **core** (merge path) |
| `--idempotency-key` | 9 | **core** |
| `--prompt-file` | 7 | **core** |
| `--headless` | 5 | active (batch spawns) |
| `--parent-run-id` / `--parent-node-id` | 5 | active (only for orchestrate DAG) |
| `--dry-run` | 3 | active |
| `--tmux-session` | 2 | active (headless campaigns) |
| `--notify` | 2 | active (completion hook) |
| `--layout` | 1 | marginal |
| `--no-hooks` | 1 | marginal |
| **`--harness`** | **0** | **IDLE — never invoked** |
| **`--agent-startup-timeout`** | **0** | idle (default 90 always used) |

Confirmed in run history: **0/717 manifests recorded a non-`claude` harness**;
**0/717 recorded `--notify` in the manifest** (notify is a supervisor hook, may not
persist to manifest — treat as "used by 2 skills, low volume").

### 2.6 Additive subsystems — referenced by ZERO workflow skills

`grep` across `crates/taskfleet-cli/skills/*` **and** the installed
`~/.claude/skills/*` for `pipeline run`, `harness bakeoff`, `--harness`,
`taskfleet pipeline`, `taskfleet harness`: **no matches**. These command
trees exist only as standalone CLI surface; nothing in the worktree/stint/fan-out/
orchestrate workflow family drives them.

### 2.7 Discussions & spin-off proposals — near-dead on disk

- Runs with **any** `discussions/` content: **4 / 717**.
- Runs with **any** `spinoffs/` (spin-off-proposal) content: **5 / 717**.

Both are top-level CLI command trees (`discussion list|show|resolve`, `spinoff
list|approve|reject`) plus schema fields (`discussion_items`, `spinoff_proposals` in
the §7.3 report), reducer projections, and supervisor consumption paths — exercised
by well under 1 % of runs.

### 2.8 Subsystem sizes (drag-per-LOC context)

| subsystem | path | LOC |
|---|---|---:|
| code-pipeline + harness + floor | `src/{pipeline,harness,floor}` | **25 406** |
| — harness adapters alone | `src/harness/*.rs` | 5 371 |
| supervisor | `src/supervise/*` | 12 137 |
| function count in pipeline/harness/floor | — | ~898 fns |

---

## 3. Per-surface classification

Legend: **🟢 ACTIVELY USED** · **🟡 IDLE-BUT-SHALLOW** (little code beyond a skill +
enum arm) · **🔴 IDLE-AND-HEAVY** (removal candidate with real drag).

### 3.1 Run-kinds

| kind | class | evidence | drag if kept |
|---|---|---|---|
| **spinoff** | 🟢 | 595 runs, 96 % of recent | none — this IS the product |
| **research** | 🟢 | 21 runs, steady in recent | shares spinoff lifecycle; low marginal drag |
| **technical-decision** | 🟢 (low) | 16 runs, appears recently | shares autonomous single-node path; low |
| **code** | 🟡 | 25 runs total, **0 in recent 120** | interactive lifecycle + hidden `--confirm-interactive` merge gate + `interactive-code-run-self-merged` guard. Its own `Lifecycle::Interactive` branch |
| **orchestrated** | 🟡→🔴 | 30 runs, only 4 in Aug | parent/child DAG bookkeeping, `--parent-*` flags, retry-desync exclusions, shared-integration-branch merge |
| **orchestrate** | 🟡→🔴 | **6 runs total** | worktree-less driver special-case (`Interactive` + "no worktree of its own"), hierarchical report.yaml/report.md, whole `/orchestrate` skill |
| **fan-out** | 🟡→🔴 | 20 runs, **0 in recent 120** | driver-node-has-no-agent special-case, concurrency budget, enumeration/resume |
| **make-skill** | 🟡 | **4 runs total** | own kind arm + skill; otherwise rides spinoff lifecycle |
| **bugfix** | 🔴 | **0 runs ever** | full kind arm, `/worktree-bugfix` skill, schema/lifecycle/retry inclusion — pure dead weight |

Every non-`spinoff` kind adds an arm to the exhaustive `match`es in
`schema.rs` (`wire_name`, `lifecycle`, `is_autonomous_single_node_worker`) and
propagates through the reducer, supervisor watchdog/retry, and doctor. `code` and
`orchestrate` are the two that force a **second lifecycle** (`Interactive`) and its
whole branch of supervisor logic; without them the supervisor could assume every run
is an autonomous single-node worker.

### 3.2 Additive command trees

| surface | class | evidence | drag |
|---|---|---|---|
| **`pipeline run`** (spec→code→floor→verify→merge) | 🔴 | referenced by **0 skills**; not wired to `run create`/supervisor | large `--` flag surface (`--intent`, `--max-build-concurrency`, `--max-recode-per-chunk`, `--file-scope-slack`, `--chunk-timeout`, wave-build concurrency…); part of the ~25 k-LOC block; shells to real `claude -p`; its own live providers + envelope + orchestrator |
| **`floor`** (deterministic gate: git/gates/parse/runner/snapshot) | 🔴 | only consumed by `pipeline` | ~7 files; only reachable via the idle pipeline |
| **`harness bakeoff` / `conformance`** | 🔴 | referenced by **0 skills**; standalone | 5 371 LOC of adapters (aider, pi, claude, claude-deepseek, stub) + conformance harness + registry, all to compare loops nobody runs from a workflow |

### 3.3 Harness selection

| surface | class | evidence | drag |
|---|---|---|---|
| **`run create --harness`** + `TASKFLEET_HARNESS` env + `config.toml [harness]` per-kind precedence | 🔴 | `--harness` in **0 skills**, **0/717 runs** non-claude | a 4-level precedence resolver (`harness::select::resolve_with`), per-kind config validation, `manifest.harness` + `harness_source` provenance fields on every run, `run show`/`run list --json` columns, doctor exposure — all to select a worker that is **always** `claude` |

### 3.4 Collaboration subsystems

| surface | class | evidence | drag |
|---|---|---|---|
| **discussions** (`discussion list|show|resolve`) | 🔴 | 4/717 runs | CLI tree + `discussion_items` §7.3 field + reducer projection + `discussions/*` file layout + `LOCK_SH` read paths + supervisor consumption |
| **spin-off proposals** (`spinoff list|approve|reject`) | 🔴 | 5/717 runs | CLI tree + `spinoff_proposals` §7.3 field + reducer projection + `spinoffs/*` layout + approve/reject state machine |

Both are report-payload fields that the merge path (`--report-file`) still parses and
validates on **every** spinoff merge, even though virtually no run populates them.

---

## 4. Where the drag concentrates (for DECISION-1)

1. **~30 k LOC of additive, workflow-unreachable subsystems** — `pipeline` +
   `floor` + `harness`. Nothing in the daily spinoff/research/decision flow touches
   them. Highest cut-value-per-risk: removing them cannot break any observed workflow
   because no skill invokes them.
2. **A second run lifecycle (`Interactive`) that exists for `code` + `orchestrate`**,
   two kinds that are absent from the recent 120 runs. Retiring them collapses the
   supervisor to a single autonomous-worker model — directly relevant to the epic's
   "polling-inference edge cases are combinatorial" hypothesis.
3. **A 4-level harness-selection resolver + per-run provenance fields for a
   choice that is constant (`claude`).** Pure config/schema/DTO drag.
4. **Two collaboration subsystems (discussions, spin-off proposals) at <1 % usage**
   that nonetheless keep schema fields, reducer projections, shared-lock read paths,
   and validation live on the hot merge path.
5. **`bugfix` — a fully-built kind with zero runs.** Free to delete.

Each idle kind also multiplies the **integrated-gate** risk surface (the
lane-misprediction and test-isolation failure modes documented in `CLAUDE.md`): more
kinds ⇒ more exhaustive-match arms ⇒ more places a parallel change can collide.

---

## 5. Removal candidates (biased toward cutting)

| # | candidate | usage evidence | est. drag removed | risk |
|---|---|---|---|---|
| R1 | **`harness bakeoff` + `conformance` + non-claude adapters** (aider/pi/deepseek/stub) | 0 skills, 0 runs | ~5 k LOC + registry + tests | low (nothing invokes) |
| R2 | **`run create --harness` selection stack** (flag, env, config precedence, `manifest.harness`/`harness_source`, DTO columns) | 0 skills, 0/717 non-claude | resolver + config validation + schema/DTO fields + doctor | low |
| R3 | **`pipeline` + `floor` code-pipeline / wave-build** | 0 skills, not wired to run/supervisor | bulk of the ~25 k-LOC block + ~898 fns | low (additive, isolated) |
| R4 | **`bugfix` kind** (enum arm, skill, lifecycle/retry inclusion) | 0 runs ever | match arms + `/worktree-bugfix` | low |
| R5 | **`make-skill` kind** | 4 runs / 5 weeks | match arms + skill | low-med (Jari may value it) |
| R6 | **discussions subsystem** (CLI tree, schema field, reducer, layout, locks) | 4/717 runs | CLI + reducer projection + §7.3 field + hot-path validation | **med** — decide with orchestrate |
| R7 | **spin-off proposals subsystem** (CLI tree, schema field, reducer, approve/reject SM) | 5/717 runs | CLI + reducer + §7.3 field + state machine | **med** — decide with orchestrate |
| R8 | **`orchestrate` + `orchestrated` DAG kinds** and the `Interactive`-driver / parent-child / shared-integration-branch machinery | 6 + 30 runs; 0 in recent 120 | second lifecycle, `--parent-*`, retry-desync exclusions, hierarchical report | **high** — biggest simplification, biggest behavior change |
| R9 | **`fan-out` kind + driver-no-agent special-case** | 20 runs, 0 in recent 120 | driver-node special-case, concurrency/resume | med |
| R10 | **`code` interactive kind + `--confirm-interactive` gate** | 25 runs, 0 in recent 120 | the entire `Lifecycle::Interactive` branch (shared with orchestrate) | high — kills interactive review workflow |
| R11 | **`--agent-startup-timeout`, `--layout`, `--no-hooks` flags** | 0 / 1 / 1 skills | minor create-arg surface | low |

**Cutting R1–R4 is nearly free** (zero observed usage, isolated code) and would remove
the largest LOC mass. R6–R10 are the substantive architecture decisions and should be
confirmed with Jari (§6) before DECISION-1 commits.

**Dependency note:** R8 (retire `orchestrate`/`orchestrated`) largely subsumes R6/R7
— discussions and spin-off proposals mainly exist to feed an orchestrator parent. If
the DAG kinds go, the collaboration subsystems lose their last real consumer and can
go with them. Likewise R10 (`code`) and R8 together are what justify the whole
`Interactive` lifecycle; cutting only one still leaves the branch.

---

## 6. CONFIRM WITH JARI (DECISION-1 / design session)

I inferred the idle set headlessly and cannot interactively confirm the use-set.
The following are **yes/no questions**; a "cut" answer turns the matching R-row above
into a Phase-2 removal.

1. **Harness selection (R1, R2):** You have never run a non-`claude` worker (0/717).
   Can we delete the whole harness-selection stack (`--harness`, the env/config
   precedence, `manifest.harness`) and hard-wire `claude`? **Cut / keep?**

2. **Harness bakeoff (R1):** Do you ever run `harness bakeoff` / conformance to
   compare agent loops, or was it exploratory? **Cut / keep?**

3. **Code-pipeline / wave-build (R3):** `pipeline run` and the `floor` gate are
   invoked by no workflow. Is the spec→code→floor→verify→merge pipeline a live
   ambition, or dead exploration we can remove? **Cut / keep?**

4. **`bugfix` kind (R4):** Never run once. Do you still want a distinct bugfix
   run-kind, or is `/worktree-spinoff "fix X"` enough? **Cut / keep?**

5. **`make-skill` kind (R5):** 4 runs in 5 weeks. Keep as a first-class kind, or fold
   skill-authoring into a plain spinoff? **Cut / keep?**

6. **`fan-out` (R9):** 20 runs but none in the recent 120. Still part of your
   workflow, or superseded by spawning spinoffs directly? **Cut / keep?**

7. **`orchestrate` + `orchestrated` (R8):** 6 + 30 runs, none in the recent 120. This
   is the biggest simplification available — dropping the DAG driver collapses the
   supervisor to a single autonomous-worker model and removes the `Interactive`
   driver special-case. Do you still orchestrate multi-feature DAGs, or has the
   pattern become "several independent spinoffs"? **Cut / keep?**

8. **`code` interactive kind (R10):** 25 runs, none recent. Do you still do
   human-reviewed interactive worktrees (`/worktree-code` + `/worktree-merge`), or is
   everything autonomous-spinoff now? Cutting this removes the entire `Interactive`
   lifecycle branch. **Cut / keep?**

9. **Discussions (R6):** 4/717 runs ever used them. Have you ever acted on a
   `discussion resolve`, or can the subsystem + its §7.3 report field go? **Cut / keep?**

10. **Spin-off proposals (R7):** 5/717 runs. Do you review/approve proposed spin-offs
    via `spinoff approve|reject`, or can it + its §7.3 field go? **Cut / keep?**

11. **Minor flags (R11):** `--agent-startup-timeout` (0 skills), `--layout` (1),
    `--no-hooks` (1). OK to drop the ones you don't recognize using? **Cut / keep?**

**Default recommendation if you don't want to decide each line:** cut R1–R4
immediately (zero-usage, isolated, low risk), and treat R6–R10 as a single
"retire everything but autonomous single-node worker (spinoff/research/
technical-decision/make-skill)" proposal for the DECISION-1 design session — that is
the cut that most directly attacks the epic's combinatorial-edge-case hypothesis.

---

## 7. Method notes / caveats

- Usage is measured over the on-disk run history (`~/.taskfleet/runs`,
  717 runs, 2026-07-09 → 2026-08-12) — a single primary user (Jari) over ~5 weeks.
  A kind at 0 runs is strong evidence of non-use; a kind in single digits is weak
  evidence and warrants the confirm question rather than an automatic cut.
- "Referenced by 0 skills" was checked against **both** the bundled
  `crates/taskfleet-cli/skills/*/SKILL.template.md` **and** the installed
  `~/.claude/skills/*` — so it reflects the running workflow surface, not just source.
- LOC figures are `wc -l` on the relevant source trees; they measure code mass, not
  cyclomatic drag, but the additive subsystems (§2.6, §2.8) are the clearest targets.
- This is a read-only inventory. No application code was changed. Phase-2 removals
  are out of scope and gated on §6 confirmation + DECISION-1.
