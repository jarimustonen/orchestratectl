# Design — stint maintains an issue-derived execution DAG in TODO.md

Status: design of record for issue `stint-maintains-execution-dag`.
Resolves every bullet under the issue's *"Design must decide"* section, then
specifies the exact `SKILL.template.md` edits.

> **Revised 2026-07-27 after an `/llm-panel` pass** (architect / maintainability /
> trigger-fit; synthesis in `history/2026-07-27-panel-stint-dag.md`). The panel caught
> a central over-claim ("derived view") and two verified correctness bugs in the first
> draft. This section states the corrected core; the per-decision sections below carry
> the folded fixes, each tagged **[panel]**.

## The one idea that resolves everything

**The `TODO.md` DAG owns the *scheduling plan* and nothing else. `issuectl` owns
status. They never store the same fact, so they cannot drift.**

The first draft called the DAG a "pure derived view, fully regenerable." The panel
(architect + maintainability, independently) showed that is a contradiction: the DAG
uniquely holds three facts that live in **no** other source —

- each issue's **lane assignment** (which hot-file family its fix touches),
- the **intra-lane order** (the recommended sequence within a lane), and
- each issue's **collision tags** (which hot files it touches, for cross-lane safety).

`issuectl` does not model scheduling, so this information genuinely lives in the DAG.
The DAG **is** the source of truth for the plan. What it must *never* store is
**status** — the volatile thing a worktree flips (`in-progress → fixed`) that would
race the orchestrator. So:

| Fact | Owner | In the DAG? |
|---|---|---|
| Status (open / in-progress / done…) | issuectl | **never** — read-through only |
| Logical dependency (`blocked_by`) | issuectl `blocked_by` | mirrored for humans |
| Which files are collision-prone | repo hot-file list (AGENTS.md) | referenced, not copied |
| Lane assignment, intra-lane order, collision tags | **the DAG** | **yes — its whole job** |

The three hard problems resolve like this:

- *Sync with issuectl* → the DAG stores only the plan, never status ⇒ no drift.
- *Racing the worktree's lifecycle* → the DAG never writes a status bit; the
  head-of-line is computed at read time by joining the DAG's ordering with issuectl's
  live status. No write, no race. **[panel]** This claim is scoped to *DAG-vs-worktree*;
  concurrent stint orchestrators are out of scope (single-conductor assumption below).
- *Staleness/repair* → **[panel]** reconciliation is a **stateful merge**, not a
  from-scratch regeneration. Phase 0 drops closed issues and adds new open ones **while
  preserving** the existing lane assignment, order, and collision tags. Regenerating the
  plan from scratch every resume would force the agent to re-derive the collision matrix
  and risk hallucinating edges — the architect's headline warning. "Merge, don't
  regenerate."

Everything below is a consequence of this corrected principle.

### Single-conductor assumption (scope boundary) **[panel]**

The "no race" guarantee covers the DAG vs. a worktree's own status writes. It does **not**
cover two `/stint` orchestrators spawning the same head concurrently — that race exists
before either worker sets `in-progress`. This is **out of scope**: the workflow assumes a
**single active stint conductor** (the human runs parallel *worktrees*, but one
*orchestrator*). An atomic `issuectl claim` API would be the fix if that assumption ever
breaks; it is not built here.

---

## Ground truth about the tooling (checked, 2026-07-27)

- `issuectl` schema **has** a `blocked_by` (optional list) field — but it is
  currently populated on **zero** of the 33 active issues. `related` is widely
  used as an informal predecessor breadcrumb (`@some-prior-issue`).
- **Decision — `blocked_by` is the authoritative dependency source; `related`
  is ignored for ordering.** `related` means "see also", not "must come after";
  reading order from it would be guesswork. Logical edges live in `blocked_by`.
- **No mass backfill.** The design does *not* migrate 33 issues. `blocked_by`
  is populated **lazily** — only when a stint's planning surfaces a real
  logical dependency. Ordering that is purely collision-based needs no
  `blocked_by` at all (lanes capture it). This keeps the change low-friction.
- **[panel] Add a dependency additively, never destructively.** `issuectl set <slug>
  blocked_by …` risks *replacing* the list. Use `issuectl apply <patch.yaml>` with the
  **`add_blocked_by`** list-op (verified: it appends, leaving existing deps intact):
  ```yaml
  slug: <downstream-slug>
  add_blocked_by:
    - <upstream-slug>
  ```
- **[panel] The active set is `open ∪ in-progress`, not `open`.** Verified:
  `issuectl ls --status open` returns **only** status==`open` (29 issues) and **excludes
  the 4 `in-progress`**. A Phase-0 refresh keyed on `--status open` would silently drop
  every live worktree's issue from the DAG. There is no single OR query, so the active
  set is the **union of two calls**:
  ```bash
  issuectl --json ls --status open        # + …
  issuectl --json ls --status in-progress # union these two (33 total)
  ```
  (Add `--status testing` if the project uses it.) Deferred issues stay `open` but carry
  a `deferred` label — see decision 5, they are excluded from head-of-line.

---

## Design decision 1 — Representation

**Chosen: a lane-grouped list inside one fenced block**, with a `▶`
head-of-line marker per lane and inline `after …` edge annotations.

Rejected alternatives:

- **YAML-in-a-fence (structured).** Tempts an agent to treat it as a second
  *structured store* and hand-maintain `status:` fields inside it → exactly the
  drift we are trying to prevent. A prose-ish lane list keeps it obviously a
  *view*, not a database.
- **Flat node/edge list.** Loses the at-a-glance "what can I start right now"
  readability that the hand-built lane block already proved works for a human.
- **Checklist (`- [ ]`).** Checkboxes *are* status — they would duplicate
  issuectl and drift. Forbidden.

The lane block carries **only** what issuectl and the hot-file list don't: the
lane assignment, the intra-lane order, the head-of-line pointer, and the
cross-cutting `after` edges. It stamps a date so a resuming agent knows how
fresh the last merge was.

**[panel] The DAG lists the *selected execution workstream*, not the whole
backlog.** Deferred issues and adjacent-but-not-scheduled work live in a separate
`## Adjacent backlog` section (as `TODO.md` already does today), never as lane
heads. Only issues chosen into the current plan are laned. This prevents the
head-of-line algorithm from surfacing a deferred or unrelated open issue as
"actionable."

### Canonical format (the template stub the skill writes)

**[panel] The block is delimited by HTML-comment fences** so an agent (and the
`comm -3` staleness check) can extract exactly the DAG nodes without scooping up
prose, backlog slugs, or command fragments elsewhere in `TODO.md`.

````markdown
## Execution DAG (<YYYY-MM-DD>)

Scheduling PLAN — this block is the source of truth for lane assignment + order;
**issuectl is authoritative for status** (never copied here). Reconcile by
MERGE at Phase 0/7 (drop closed, add new-open, preserve existing order/edges) —
do not regenerate from scratch. Lanes = hot-file families; within a lane ≤1 live
worktree at a time, listed in recommended order; across lanes heads run in
parallel unless they share a `collision:` file.
`▶` = head-of-line snapshot (RE-COMPUTE from issuectl at pick time, never trust
between merges). `after <slug> (needs …)` = logical `blocked_by` mirror.
`collision: <file>` = touches a second lane's hot file (spawn-time exclusion).

<!-- execution-dag:begin -->
```
GLOBAL HEAD-OF-LINE: <slug>   ← start here on resume

LANE A — <hot-file family, e.g. supervise/* + reducer/schema>
  ▶ <slug-a1>
    <slug-a2>
    <slug-a3>   after <slug-b2> (needs its new API)     # logical dep
LANE B — <hot-file family, e.g. pipeline/* + floor/* + harness/*>
  ▶ <slug-b1>
    <slug-b5>   collision: create.sh                    # shares a hot file w/ lane A
LANE C — <hot-file family, e.g. workmux vendoring>
  ▶ <slug-c1>
UNLANED — no shared hot files, run anytime, no mutual sequencing:
    <slug-x>, <slug-y>
```
<!-- execution-dag:end -->
````

Notes baked into the format choice:

- **Issues are identified by slug, never by a positional code** (no `A1`/`B5`).
  Positional codes churn on every insert — churn is friction is drift. Slugs
  are stable; edges reference slugs.
- **Lane letters (A/B/C) are coarse, stable group labels**, one per hot-file
  family. Few and slow-changing, so they cost nothing to keep.
- `UNLANED` is the bucket for issues that touch no hot file — they collide with
  nothing and can be picked up anytime.
- **[panel] Cross-lane collision is a `collision: <file>` tag, not an `after
  <slug>` edge.** A collision is a shared-*resource* fact ("this issue touches
  `create.sh`"), not a dependency on whichever issue currently happens to be the
  other lane's head. Tagging the file keeps the constraint correct when heads
  advance; the spawn-time rule (decision 5) does the exclusion. `after <slug>` is
  reserved for *logical* deps (mirrors `blocked_by`).

---

## Design decision 2 — Edge semantics

Two edge kinds, kept **distinct** because they come from different sources and
repair differently:

| Edge kind | Means | Source | Where stored |
|---|---|---|---|
| **Collision (resource)** | "touches this hot file — don't run alongside another worktree touching it" | repo hot-file list | **implicit** in lane membership; cross-lane cases get a `collision: <file>` tag |
| **Logical dependency** | "this work needs that issue's code to exist first" | issue `blocked_by` | issuectl `blocked_by` (authoritative), **mirrored** as `after <slug> (needs …)` |

**The current hot-file-lane model is VALIDATED, with two revisions.** Partitioning
by hot-file family is correct: collision is a property of the *file set* (a repo
fact), and lanes make the "at most one live worktree per lane" rule visually
obvious.

- *Revision 1 — separate the two edge kinds.* The current DAG conflates collision
  and logical edges into one prose `⚠ depends on` note. This design **separates
  them** — a collision is a *plan/resource* property (never touches the issue) while
  a logical edge is an *issue* property (recorded in `blocked_by`). Different
  provenance, different maintenance.
- *Revision 2 **[panel]** — collision is a resource tag, not a slug edge.* A single
  issue touching *two* hot-file families (e.g. both `pipeline/*` and `create.sh`)
  lives in its primary lane and carries a `collision: create.sh` **tag** — not an
  `after <other-head>` edge. Rationale: a collision constrains this issue against
  *every* concurrently-running worktree that touches `create.sh`, not just whichever
  issue is the other lane's head right now; and the tag doesn't go stale when heads
  advance. The exclusion is enforced at spawn time (decision 5), not baked as a
  point-to-point edge.

---

## Design decision 3 — Where edges come from

- **Collision edges** ← the repo's hot-file list. The skill reads it
  *generically* ("the repo's AGENTS.md hot-file notes"); the actual list
  (`supervise/*`, `pipeline/*`, …) is a **project fact**, never hardcoded in the
  skill. Lane assignment = "which hot-file family does this issue's fix touch?"
  — a judgment the planner already makes in Phase 2's collision analysis.
- **Logical edges** ← issue `blocked_by`. When planning reveals a real
  dependency, the skill (a) records it additively via `issuectl apply` with
  **`add_blocked_by`** (never `set`, which can replace the list) and (b) mirrors it
  as `after <upstream> (needs …)`. `related` is **not** read for ordering.

---

## Design decision 4 — Maintenance triggers (exact edit per phase)

The DAG is touched at **five** phases (**[panel]** Phase 6 added). Each edit is
small and local, and — **[panel]** every edit that touches `TODO.md` or issue
frontmatter is **committed at the end of its phase**, before any worker is
spawned. This obeys the repo's "keep main clean; never leave main
modified-but-uncommitted across a phase" invariant, and guarantees a worker never
branches off a tree where the scheduling/`blocked_by` metadata is uncommitted.

### Phase 0 — Orient → **stateful MERGE + validate** (the self-heal step) **[panel]**

On every resume, before orienting the user, reconcile the DAG against reality.
This is a **merge, not a regenerate-from-scratch**: preserve the existing lane
assignment, order, and `collision:` tags (they are the plan, and re-deriving them
risks hallucinated edges). Do this **first**, before anything else in the phase —
skipping it means acting on stale data.

1. Fetch the **active set** = `issuectl ls --status open` **∪** `--status
   in-progress` (**[panel]** `open` alone drops live worktrees). Add `--status
   testing` if the project uses it.
2. **Drop** any DAG line whose slug is not in the active set (it closed / merged /
   was renamed). issuectl is the authority.
3. **Add** any active issue missing from the DAG into its lane (by hot-file
   family; `UNLANED` if it touches none). Catches issues filed by *other* sessions.
   Skip issues carrying a `deferred` label — they belong in `## Adjacent backlog`,
   not a lane (decision 5).
4. **Validate [panel]:** every `after <slug>` / `blocked_by` target resolves to a
   real issue (flag danglers); no self-dependency; no cycle. On a cycle or dangling
   ref, surface it to the user rather than silently emitting a wrong head.
5. **Re-compute head-of-line** per lane and the `GLOBAL HEAD-OF-LINE` (decision 5).
6. **Commit** the refreshed `TODO.md` (only if it changed).
7. Present the ready frontier to the user as "what's actionable."

Fast mechanical drift check (read-only) — **[panel]** scoped to the delimited DAG
block so it ignores prose and the backlog section:

```bash
comm -3 \
  <(issuectl --json ls --status open  | jq -r '.[].slug'
    issuectl --json ls --status in-progress | jq -r '.[].slug' | sort -u) \
  <(sed -n '/execution-dag:begin/,/execution-dag:end/p' TODO.md \
      | grep -oE '[a-z0-9][a-z0-9-]+' | sort -u)
```

Left-only slugs = active issues missing from the DAG (add them); right-only =
stale DAG lines (drop them). A hint, not an authority — lane assignment stays a
judgment.

### Phase 1 — Triage → **INSERT new fix-now bugs**

Each fix-now bug is already an open issue (filed by `/triage-bugs`). Insert one
line into its lane (chosen from the bug analysis' likely-touched files; `UNLANED`
if unclear). If it depends on another issue, record it via `issuectl apply`
`add_blocked_by` + an `after …` mirror. **Commit** the `TODO.md` edit. Edit = add
one line.

### Phase 2 — Plan → **INSERT planned units + set `blocked_by`**

Any feature/backlog unit pulled into the round that isn't yet an issue gets one
filed (per repo policy) and inserted. The **file-collision analysis Phase 2
already performs *is* the lane assignment** — "sequence these hot-file units" ≡
"they share a lane." Record real logical deps via `add_blocked_by`. **Commit** the
`TODO.md` + frontmatter edits before Phase 3 spawns anything.

### Phase 3 — Orchestrate → **DO NOT write status into the DAG**

When a worktree is spawned for a head-of-line issue, the **worktree owns the
issue lifecycle** (`triaged` → `in-progress` → `fixed`). The DAG must **not**
write its own in-progress flag (that races the worktree's issuectl updates —
see decision 5). Optional, advisory only: annotate `(spawned <run-id>)` next to
a just-launched head as an in-session human breadcrumb; it is cleared by the
next Phase-0 merge and is never consulted as truth.

### Phase 6 — Absorb feedback → **INSERT feedback-born issues** **[panel]**

Feedback often files new issues or reveals a new dependency. Because Phase 6 can
run a mini-round (Phases 2–4 in miniature), the DAG must be updated **before** that
mini-round consults it — otherwise the mini-round sequences against a stale graph.
So: any issue filed / any `blocked_by` changed in Phase 6 is inserted into the DAG
(same edit as Phase 1/2) and **committed** before spawning. If feedback is captured
durably without a mini-round, the insert still happens so Phase 7 is already current.

### Phase 7 — Handoff → **final MERGE + persist**

Before handoff, merge once more (landed issues fall off, any remaining
feedback-born issues added), refresh the date stamp, and **commit `TODO.md`** so
the next `jatketaan @TODO.md` agent's Phase 0 opens onto an accurate graph. (Phase 5
report already surfaced what landed; Phase 7 just makes the DAG match.)

---

## Design decision 5 — Head-of-line selection (no DAG↔worktree race)

**The DAG never stores progress.** "Head-of-line" is *computed on read* by joining
the DAG's lane order with issuectl's live status. `▶` in the file is only a
**snapshot** from the last merge — **[panel]** always re-compute at pick time, never
trust the printed marker between merges.

```
active(i)   := issuectl status(i) ∈ {open, in-progress}   # NOT terminal
eligible(i) := active(i)
               AND 'deferred' ∉ labels(i)                 # [panel] deferred parked
               AND every s ∈ blocked_by(i) is DEP-SATISFIED
               AND status(i) ≠ in-progress                # already being worked

DEP-SATISFIED(s) :=                                        # [panel] success ≠ any terminal
     status(s) ∈ {fixed, done}                            # dependency delivered
   ; status(s) ∈ {wontfix, obsolete, cannot-reproduce}    # cancelled — NOT satisfied:
       → the dependent is NOT eligible; flag it to the user
         (the code it needed was never built)
   ; status(s) = duplicate → follow the canonical issue, evaluate that
   ; s not found           → validation error (dangling ref)

for each lane L (in the DAG's order):
  head(L) := the first eligible(i) in L                   # the free frontier
  # if L's first item is in-progress, that's the live frontier; head(L) is the
  # next eligible item, but do NOT spawn past a lane's one live worktree.
  mark head(L) with ▶ (snapshot)

spawn-eligibility across lanes [panel]:
  a head is spawnable iff none of its collision files (its lane's hot-file family
  + any `collision: <file>` tag) is currently held by an in-progress worktree.
  occupied := ⋃ hot-files(j) for j in-progress
  spawnable(head) := collision-files(head) ∩ occupied = ∅

GLOBAL HEAD-OF-LINE [panel] := deterministic pick among spawnable heads —
  1. the slug the handoff explicitly names as "start here" (if still spawnable)
  2. highest issuectl priority
  3. earliest in its lane's recommended order
  4. slug ascending (final tiebreak)
```

Why this dodges the race with the worktree's own lifecycle updates:

- An issue issuectl marks **`in-progress`** (a live worktree is on it) is the
  lane's *live* frontier, but it is **not eligible to spawn again** (the
  `status ≠ in-progress` clause). So the DAG's `▶` shows where the lane is, and
  issuectl status — read, never written by the DAG — says free or busy.
- Because the DAG asserts no status of its own, a worktree flipping
  `in-progress → fixed` mid-stint can never contradict the DAG. Next merge, the
  fixed issue drops out and the next lane item becomes eligible.

This is the crux: **ordering lives in the DAG, status lives in issuectl, and the
head-of-line is the read-time join of the two** — recomputed, never cached.

---

## Design decision 6 — Sync boundary with issuectl

The invariant, stated as an ownership table:

| Concern | Authority | In the TODO.md DAG? |
|---|---|---|
| Issue existence / title | issuectl | slug reference only |
| **Status** (open/in-progress/done…) | issuectl | **NO — never written; read-through only** |
| Logical dependency | issuectl `blocked_by` | mirrored as `after … (needs …)` for humans |
| Priority | issuectl | may hint global-head order |
| Which files are collision-prone | repo hot-file list (AGENTS.md) | referenced, not copied |
| **Lane assignment, intra-lane order, `collision:` tags** | **the DAG** | **YES — its whole job** |
| **Head-of-line** | derived (lane order ⋈ status ⋈ blocked_by) | snapshot `▶` only; recomputed on read |

**Rule:** the DAG stores exactly the *scheduling plan* (lane assignment, order,
collision tags) and never status. issuectl stores status and never scheduling.
No fact lives in both ⇒ no drift. **[panel]** The one honest correction to the
first draft: the DAG is *not* "fully derivable" — the plan facts live only here.
It is derived **for status**, authoritative **for the plan**.

---

## Design decision 7 — Staleness / repair (stateful merge) **[panel]**

Reconciliation is a **stateful merge**, not a from-scratch regeneration — the
architect's headline point. The existing plan (lane assignment, order, collision
tags) is preserved; only the node *set* is reconciled against issuectl. This
runs at Phase 0 and Phase 7:

| Discrepancy | Repair (merge, preserving existing plan) |
|---|---|
| Closed issue still listed | absent from the active set (`open ∪ in-progress`) → dropped |
| Renamed slug | old slug absent from active set → dropped; new slug added to its lane |
| Active issue missing from DAG | present in active set, absent from DAG → added (the `comm -3` check surfaces it) |
| `blocked_by` target now `fixed`/`done` | dep satisfied → the `after …` mirror may be dropped |
| `blocked_by` target `wontfix`/`obsolete` | **[panel]** dependent NOT unblocked → flag to user |
| Dangling / cyclic `blocked_by` | **[panel]** validation error → surfaced, not silently rendered |
| Lane no longer matches an issue's real hot files | planner moves the line during Phase 0/2 |

Why merge, not regenerate: re-deriving lane assignment + collision edges from
scratch each resume would force the agent to recompute the hot-file collision
matrix over all active issues — computationally wasteful and prone to
hallucinating or dropping a subtle cross-lane `collision:` tag, which could let
two colliding worktrees spawn and break the build. Preserving the curated plan and
only reconciling the node set is both cheaper and safer.

---

## Generic vs project-specific (skill stays generic)

The skill encodes the **convention** — lanes = hot-file families, the
`▶` / `after …` / `collision:` notation, "status lives in issuectl, never the
DAG," merge-don't-regenerate at Phase 0 and Phase 7. The **project facts** — the
actual hot-file list, the actual lane set — stay in the repo's `AGENTS.md` /
`TODO.md`, referenced, never hardcoded. A tiny read-only `comm -3` one-liner
(decision 4) is inlined as prose; no helper *script file* is warranted, because
lane assignment needs judgment (which hot-file family an issue's fix touches) and
a fully-automatic generator would be unreliable.

---

## Post-skill-review refinements (2026-07-27) **[skill-review]**

An `/llm-skill-review` pass (executor / trigger-fit / cross-skill / blast-radius /
design-fidelity; the fidelity lens confirmed the skill faithfully implements this design)
found real ambiguities in the *mechanics*. Folded into the skill (and reflected here):

- **Reserve collision files at launch, not at first commit.** A spawned worker stays
  `open` until its first commit, so keying spawn-eligibility purely on issuectl
  `in-progress` leaves a window where two heads sharing a hot file both look free. Fix:
  eligibility treats every **launched-but-unsettled run this round** as holding its
  collision files *and* its issue (also closes duplicate-spawn). This tightens decision 5.
- **No spawn breadcrumb in the DAG.** The optional `(spawned <run-id>)` note is dropped —
  it left `TODO.md` dirty across a phase and polluted the drift grep. Launched run-ids live
  in conductor memory. (Supersedes the "advisory breadcrumb" allowance in decision 4.)
- **`UNLANED` = *confirmed* no hot file.** "Unclear" must be laned conservatively, never
  defaulted to `UNLANED` (which asserts parallel-safe). Aligns Phase 1 with Phase 2.
- **Commit names issue files too.** `issuectl` does not auto-commit; every phase commits
  `TODO.md` **plus** the exact issue files `issuectl` rewrote (not `git add -A`).
- **Don't mutate an in-progress issue's frontmatter** — worker-owned; record the dep in
  the DAG and reconcile after landing (respects `worktree-spinoff`'s no-caller-issuectl rule).
- **Validation halts scheduling.** A dangling/cyclic edge invalidates the DAG; repair
  before spawning — not just "surface".
- **Dep-satisfied = {fixed, done}; every other terminal (incl. `duplicate`) does not
  satisfy → flag.** Dropped the unimplementable "follow the canonical duplicate" step.
- **Drift check parses only node slugs** (leading-token `sed`, deferred + epics filtered);
  **`UNLANED` is one slug per line**; **`## Adjacent backlog` lives outside the delimiters**.
- **Active set = non-terminal statuses per the project's schema** (open ∪ in-progress ∪
  any others the schema defines), not a hardcoded pair.

Deferred as **spin-offs** (real but out of scope for the DAG convention): the
`/triage-bugs`-vs-`/stint` disagreement over who sets `in-progress`; a clean-tree preflight
before `git pull`; a durable landed-commit id so git-verify doesn't depend on a branch the
supervisor may already have torn down; bounding `run wait` against the false-`pending` bug.

## Rejected / deferred (from the panel) **[panel]**

- **Structured `execution:` frontmatter + an `issuectl execution render`
  generator/validator tool (gpt-5.6's heavier recommendation).** REJECTED for this
  issue: it converts a skill-prose change into a tooling project, duplicates plan
  state into issue frontmatter, and contradicts the issue's "cheaply
  machine-updatable *in TODO.md* by an agent" intent. The stateful-merge framing
  gets the same honesty (the DAG owns the plan) without new tooling. **Deferred**
  as a future evolution if the prose DAG demonstrably drifts or produces a wrong
  head in practice — the one unresolved tension the panel leaves open.
- **Atomic `issuectl claim` API for concurrent orchestrators.** Out of scope under
  the single-conductor assumption; build it only if that assumption breaks.

---

## Implementation plan (SKILL.template.md edits)

1. **New "Standing discipline" bullet — the execution DAG.** State the corrected
   principle: TODO.md carries a lane-based execution DAG that is the *scheduling
   plan* over the active issuectl set + the repo hot-file list; it stores lane
   order, never status; **merge it, don't regenerate**; recompute head-of-line
   from issuectl on read.
2. **Phase 0** — add the *stateful-merge + validate* step (active set = open ∪
   in-progress, drop closed, add missing, validate deps, recompute head-of-line,
   the scoped `comm -3` check, commit), and "present the ready frontier."
3. **Phase 1** — "insert each fix-now bug into its lane; `add_blocked_by` +
   `after …` for any dep; commit."
4. **Phase 2** — note the collision analysis *is* the lane assignment; insert
   planned units; record deps via `add_blocked_by`; commit before Phase 3.
5. **Phase 3** — the guardrail: never write status into the DAG; the worktree owns
   the lifecycle; spawn-eligibility = collision-files ∩ in-progress = ∅; optional
   advisory `(spawned <run-id>)` only.
6. **Phase 6 [panel]** — insert feedback-born issues / changed deps before any
   mini-round; commit.
7. **Phase 7** — "merge once more, refresh date, commit TODO.md." (Folds into the
   existing handoff-block update step, already committed on its own.)
8. **New short "Execution DAG" reference section** giving the delimited canonical
   format and the head-of-line algorithm, so the skill is self-contained.
9. **Project prerequisites** — the DAG relies on the repo documenting its hot-file
   list (already listed) and keeping the delimited `## Execution DAG` section in
   TODO.md.
10. **Migrate `TODO.md` [panel]** — bring the live Execution DAG into the new
    canonical format (delimiters, slug identity, `▶`, `collision:` tags, backlog
    separated) so the very next resume validates against the convention.

All project-specific lane content (supervise/pipeline/workmux families) stays in
`TODO.md`; the skill references "the repo's hot-file list," never the names.
