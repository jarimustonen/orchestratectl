# Target state 0.2 — the simplification (DECISION-1 outcome + design-session frame)

**Status:** DECISION-1 **decided** with Jari in the 2026-08-12 PO review (the stint-start
Wave-1 report conversation). This is the design session's (`arch-redesign-design-session`,
Phase 2) starting frame and the input to the ADR (`arch-decision-rearchitect-vs-harden`,
DECISION-2). Grounded in the three Phase-1 reports: `analysis.md`, `feature-audit.md`,
`alternatives.md`.

**Method note (why this is trustworthy).** Every linchpin below was (a) grounded in the
**real-usage evidence** (`feature-audit.md`: 717 runs) and (b) **verified against the code**
before it was accepted — not asserted. Examples: `bugfix`/`make-skill` confirmed as phantom
variants grouped with `Spinoff` in every predicate; cutting `code` confirmed to collapse the
`Lifecycle` inference axis (Code+Orchestrate are the only Interactive kinds); the heavy
`CodeHarness` trait confirmed to have the pipeline as its only real consumer. The design
session should keep this discipline: evidence + code-verify before deciding.

---

## 1. The confirmed working model

**stint → PO review → stint → …** Jari drives as product owner; autonomous `spinoff`
workers do the coding; review happens at the **round/PO level** (between stints), not
per-worktree. Interactivity is reached for **occasionally**, when a specific problem *must*
be done hands-on. This model is now the design's anchor — build for it, not for a general
orchestration tool.

Confirmed real usage shapes (from `feature-audit.md`):

- **spinoff = the norm** (83% of 717 runs; 96% of the last 120). Constant, high-frequency.
- **interactive = occasional**, and it can apply to *any* work (coding, bugfix, research,
  design) — its defining property is "needs interactivity", not a work-type.
- **fan-out = a real, distinct need**: process a work-queue of many parallel independent
  units (e.g. many research operations).

## 2. The reframe — three orthogonal axes today's 9 kinds conflate

Today's kinds fuse three independent things. Separating them is the whole simplification:

| Axis | What it is | Today | Target |
|---|---|---|---|
| **Topology** | one worker vs. N-parallel queue | `spinoff` vs. `fan-out` (+ others) | stays — a real structural difference |
| **How run** | autonomous-headless vs. interactive | baked into the `code` kind | an **explicit `--interactive` flag** on any run |
| **Workflow** | research / fix / design / ADR recipe | a dedicated *kind* each | a **skill / prompt-fragment** over the generic worker (see §5, OPEN) |

**"Told, not guessed."** The unifying principle (see §4): the system never *infers* a fact
it could be *told*. Interactivity → an explicit flag, not derived from kind. Lifecycle
transitions → worker-reported events, not inferred from pid×pane×branch. Run health →
stored state, not re-derived at read time. Every guess is a combinatorial edge case; every
explicit assertion is a lookup.

## 3. The cut / keep / change list (DECISION-1)

**CUT (decided direction; exact execution scoped by the design session):**

- **`orchestrate` + `orchestrated`** (kinds + `/orchestrate` + `/worktree-orchestrated`
  skills + integration-branch machinery + hierarchical report). The stint loop absorbs the
  multi-feature-campaign job as stint waves; the autonomous unattended niche is unused
  (6/717, 0 recent). Jari confirmed the stint model *is* the working model.
- **`code` as a kind** — it was only "spinoff, interactive-by-default". Interactivity
  becomes the orthogonal `--interactive` flag. Cutting `code` (after `orchestrate` goes)
  empties `Lifecycle::Interactive` → the kind-derived lifecycle *inference* collapses (~24
  supervisor/watchdog branch points), directly killing accidental complexity `analysis.md`
  §C.3 named. NB: interactivity does **not** disappear — it becomes explicit state.
- **`bugfix` + `make-skill` as kinds** — phantom variants (behaviourally `Spinoff` in every
  predicate; 4/717 and 0/717). Their *value is the normalized workflow*, which lives at the
  skill/fragment level, not the engine. Keep the workflows (§5), drop the kind variants.
- **`pipeline` + `floor`** (~20k LOC, code-pipeline / wave-build) — 0 workflows invoke it;
  the spec→chunk→verify path was superseded by plain spinoffs. Biggest single simplification.
- **`harness` heavy layer**: `bakeoff` + `conformance` + the `CodeHarness` trait + `aider` +
  `claude-deepseek`. These exist to serve the pipeline's tiered-model execution; they die
  with it. Keep the **light launcher path** (`select`/`pi`/`workmux_agent`, claude+pi). pi.dev
  serves the multi-*model* need, so taskfleet needs no multi-model *execution* layer.
- **Mid-run discussion / spinoff-proposal machinery** — the `discussion.opened/resolved` +
  `spinoff.proposed/approved/rejected` events, `discussions/`+`spinoffs/` projections,
  `open_discussions`/`pending_spinoffs` counters, and `discussion`/`spinoff` CLI subcommands
  (4/717, 5/717). This is the interactive-per-worker human-in-the-loop mode we're moving away
  from. **Keep the terminal-report `discussion_items[]` / `spinoff_proposals[]`** — decisions
  and follow-ups surface at the round/PO level (essential terminal-outcome contract).

**KEEP:** `spinoff` (topology), `fan-out` (topology), `research` + `technical-decision`
(at least as skills; see §5 open q), the **claude + pi** launcher (claude is the *current*
runtime — not a cut), the **crash-atomic event store** (`applied_seq` / `LockedRun` / shared
lock — the essential, bug-free foundation), `run merge` + the teardown gate (invariant 5),
the terminal-report contract, and interactivity **as a flag**.

**CHANGE:** **`spinoff` is always headless.** Remove the non-headless path (today someone can
spawn a spinoff as a tmux window in-session). A concrete "normalize the workflow so it gets
better" change of exactly the kind Jari named.

Net: **9 kinds → ~2–4 topologies**; interactivity becomes a flag; workflows become
skills/fragments; the heavy layers (pipeline/floor, harness trait, discussion machinery) are
gone. Because the survivors are all autonomous, the `Lifecycle` enum can retire as a
kind-derived concept.

## 4. Design philosophy for the redesign — "clean-slate the MODEL, keep proven primitives"

Recommended north stars for the design session (reactable, not settled):

1. **Told, not guessed** — explicit state over inferred state, everywhere (§2). Directly
   attacks the root cause (`analysis.md`: inference is the defect, not any one signal).
2. **New model, old foundation.** Clean-slate the *model* (write the target from requirements,
   don't refactor the watchdog into a protocol). **Keep the proven primitives** — the
   crash-atomic store is the one bug-free layer and is genuinely hard to re-get-right; a
   from-zero rewrite would re-introduce solved bugs. Clean slate at the *design* level,
   surgical at the *implementation* level.
3. **Usage-scoped, not capability-scoped.** The old design's sin was building generality
   (pipeline, multi-harness, 9 kinds) ahead of use — and that generality became the dead
   weight. Build for the known stint workflow; add generality only when a second real use
   appears.
4. **Typed/provable over heuristic.** Empirically in this codebase the typed, invariant-
   enforced parts (event store, `LockedRun` witness) stayed bug-free; the heuristic, inferred
   parts (activity clocks, tmux tri-state) accreted bugs. The store is the template; the
   watchdog is the anti-pattern.

## 5. Open questions for the design session (genuinely undecided)

1. **Workflow packaging.** Are the normalized workflows (bugfix, ADR, research-recipe,
   make-skill) **standalone skills**, or a **list of prompt-fragment files** the generic
   worktree skill composes/references? Jari leans toward the latter (lighter). Deferred to
   the design decision on purpose.
2. **How far to collapse `research` / `technical-decision`.** `research` needs
   WebSearch/WebFetch in the worktree allowlist — a real per-kind difference, OR a per-run
   tool-flag. Decides whether they stay topologies or become "spinoff + skill + tool-flag".
3. **The surviving supervisor core's shape** = DECISION-2 itself: `alternatives.md`'s fork —
   **thin model** (`run merge` is the only completion truth) vs. **protocol** (worker
   self-reports transitions + lease), with **exit-code + FIFO** as the cheap adjunct that
   helps either. Both realize "told, not guessed"; they differ in *how much* the worker tells.
4. **`--interactive` supervisor semantics** (no auto-teardown, human finalizes merge, dead
   pid ≠ terminalize) — the flag is DECISION-1's shape, but how the supervisor treats it is
   DECISION-2 territory.
5. **Migration/back-compat.** Removing kinds/flags/subcommands is a **breaking** CLI change.
   Single-user tool → a clean break is likely fine, but confirm (vs. a deprecation window).

## 6. Release scoping

**0.2 = the simplification + pi.dev** (Jari's call — one release, no separate 0.3). The pi.dev
thread (`workmux-pi-agent-preset`, `config-subcommand`) continues toward it alongside the
simplification landed from the design→ADR.

## 7. Relationship to DECISION-2 and the gated lanes

Run **DECISION-1's cuts first** — they *pre-shrink* DECISION-2's list: cutting `code` removes
the lifecycle inference; cutting the discussion machinery removes reducer/schema surface;
cutting `pipeline` drags out the harness trait. Many Lane A / Lane E issues **obsolete without
a fix** once their surface is gone. The formal per-issue re-triage (keep / defer / obsolete /
re-scope) still happens at **DECISION-2**, after the design session picks the surviving core's
model and the ADR records it. Until then Lanes A + E stay gated; this doc is the target they
will be re-triaged against.
