# Design — stint maintains an issue-derived execution DAG in TODO.md

Status: design of record for issue `stint-maintains-execution-dag`.
Resolves every bullet under the issue's *"Design must decide"* section, then
specifies the exact `SKILL.template.md` edits.

## The one idea that resolves everything

**The DAG in `TODO.md` is a derived VIEW, never a second source of truth.**

It is fully regenerable from three inputs the project already has:

1. `issuectl ls --status open --json` — the node set, each node's status,
   priority, and `blocked_by`.
2. The repo's **hot-file list** (root `AGENTS.md` / `CLAUDE.md`) — the
   file-collision partition → the DAG's *lanes*.
3. Issue **`blocked_by`** frontmatter — the *logical* dependency edges.

Because the DAG is derivable, three hard problems collapse into one cheap
operation — **regenerate**:

- *Staleness/repair* → re-derive; closed issues fall out, new ones fall in.
- *Sync with issuectl* → the DAG stores only what issuectl does **not**
  (the lane partition + rendered ordering); status is always read-through, so
  it cannot drift.
- *Racing the worktree's lifecycle* → the DAG never writes an "in-progress"
  bit; "in-progress" is read from issuectl. No write, no race.

Everything below is a consequence of this principle.

---

## Ground truth about the tooling (checked, 2026-07-27)

- `issuectl` schema **has** a `blocked_by` (optional list) field — but it is
  currently populated on **zero** of the 30 open issues. `related` is widely
  used as an informal predecessor breadcrumb (`@some-prior-issue`).
- **Decision — `blocked_by` is the authoritative dependency source; `related`
  is ignored for ordering.** `related` means "see also", not "must come after";
  reading order from it would be guesswork. Logical edges live in `blocked_by`.
- **No mass backfill.** The design does *not* migrate 30 issues. `blocked_by`
  is populated **lazily** — only when a stint's planning surfaces a real
  logical dependency. Ordering that is purely collision-based needs no
  `blocked_by` at all (lanes capture it). This keeps the change low-friction.
- Set a dependency with `issuectl set <slug> blocked_by '<upstream-slug>'`
  (list field; use `issuectl apply <patch.yaml>` for multi-value / transactional
  edits).

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
lane partition, the ordering within each lane, the head-of-line pointer, and
the cross-cutting `after` edges. It stamps a date so a resuming agent knows how
fresh the last regeneration was.

### Canonical format (the template stub the skill writes)

````markdown
## Execution DAG (<YYYY-MM-DD>)

Derived VIEW — regenerate from `issuectl ls --status open` + the hot-file list
in AGENTS.md. Holds NO status of its own; **issuectl is authoritative for
status**. Lanes = hot-file families (the file-collision partition); within a
lane issues are strictly sequenced, across lanes heads run in parallel.
`▶` = head-of-line (the lane's active frontier / next actionable).
`after <slug> (…)` = an ordering edge lane-order doesn't already imply
(cross-lane collision, or a logical `blocked_by` dep).

```
GLOBAL HEAD-OF-LINE: <slug>   ← start here on resume

LANE A — <hot-file family, e.g. supervise/* + reducer/schema>
  ▶ <slug-a1>
    <slug-a2>
    <slug-a3>   after <slug-b2> (needs its new API)     # logical dep
LANE B — <hot-file family, e.g. pipeline/* + floor/* + harness/*>
  ▶ <slug-b1>
    <slug-b5>   after <slug-a1> (collision: create.sh)  # cross-lane collision
LANE C — <hot-file family, e.g. workmux vendoring>
  ▶ <slug-c1>
UNLANED — no shared hot files, run anytime, no mutual sequencing:
    <slug-x>, <slug-y>
```
````

Notes baked into the format choice:

- **Issues are identified by slug, never by a positional code** (no `A1`/`B5`).
  Positional codes churn on every insert — churn is friction is drift. Slugs
  are stable; edges reference slugs.
- **Lane letters (A/B/C) are coarse, stable group labels**, one per hot-file
  family. Few and slow-changing, so they cost nothing to keep.
- `UNLANED` is the bucket for issues that touch no hot file — they collide with
  nothing and can be picked up anytime.

---

## Design decision 2 — Edge semantics

Two edge kinds, kept **distinct** because they come from different sources and
repair differently:

| Edge kind | Means | Source | Where stored |
|---|---|---|---|
| **Collision (sequencing)** | "same hot file — don't run these worktrees in parallel" | repo hot-file list | **implicit** in lane membership + order; cross-lane cases get `after <slug> (collision: <file>)` |
| **Logical dependency** | "this work needs that issue's code to exist first" | issue `blocked_by` | issuectl `blocked_by` (authoritative), **mirrored** as `after <slug> (needs …)` |

**The current hot-file-lane model is VALIDATED, with one revision.** Partitioning
by hot-file family is correct: collision is a property of the *file set* (a repo
fact), and lanes make the "at most one live worktree per lane" rule visually
obvious. The revision: the current DAG conflates collision and logical edges
into one prose `⚠ depends on` note. This design **separates them** — because a
collision edge is a *plan* property (never touches the issue) while a logical
edge is an *issue* property (recorded in `blocked_by`). Same visual (`after …`),
different provenance and different maintenance.

A single issue that touches *two* hot-file families (e.g. one editing both
`pipeline/*` and `create.sh`) lives in its primary lane and carries an explicit
`after <slug> (collision: <file>)` to the other lane's head — exactly the old
`A1 → B5` cross-lane edge, now typed.

---

## Design decision 3 — Where edges come from

- **Collision edges** ← the repo's hot-file list. The skill reads it
  *generically* ("the repo's AGENTS.md hot-file notes"); the actual list
  (`supervise/*`, `pipeline/*`, …) is a **project fact**, never hardcoded in the
  skill. Lane assignment = "which hot-file family does this issue's fix touch?"
  — a judgment the planner already makes in Phase 2's collision analysis.
- **Logical edges** ← issue `blocked_by`. When planning reveals a real
  dependency, the skill (a) `issuectl set <slug> blocked_by '<upstream>'` and
  (b) mirrors it as `after <upstream> (needs …)`. `related` is **not** read.

---

## Design decision 4 — Maintenance triggers (exact edit per phase)

The DAG is touched at four points. Each edit is small and local.

### Phase 0 — Orient → **REGENERATE + validate** (the self-heal step)

On every resume, before orienting the user, reconcile the DAG against reality:

1. `issuectl ls --status open --json` → the authoritative open set.
2. **Drop** any DAG line whose slug is not in that set (closed / renamed /
   obsolete). It landed or went away; issuectl is the authority.
3. **Add** any open issue missing from the DAG into its lane (by hot-file
   family; `UNLANED` if it touches none). Catches issues filed by *other*
   sessions since last handoff.
4. **Recompute head-of-line** for each lane (algorithm in decision 5) and the
   `GLOBAL HEAD-OF-LINE`.
5. Present the ready frontier to the user as "what's actionable."

A mechanical staleness check the skill can run to spot drift fast (pure
set-difference, read-only — no auto-edit):

```bash
comm -3 \
  <(issuectl ls --status open --json | jq -r '.[].slug' | sort) \
  <(grep -oE '[a-z0-9][a-z0-9-]+' TODO.md | sort -u)
```

Left-only slugs = open issues missing from the DAG (add them); this is a hint,
not an authority — the agent still assigns lanes by judgment.

### Phase 1 — Triage → **INSERT new fix-now bugs**

Each fix-now bug is already an open issue (filed by `/triage-bugs`). Insert one
line into its lane (chosen from the bug analysis' likely-touched files; `UNLANED`
if unclear). If it depends on another issue, `issuectl set … blocked_by` + an
`after …` annotation. Edit = add one line.

### Phase 2 — Plan → **INSERT planned units + set `blocked_by`**

Any feature/backlog unit pulled into the round that isn't yet an issue gets one
filed (per repo policy) and inserted. The **file-collision analysis Phase 2
already performs *is* the lane assignment** — "sequence these hot-file units" ≡
"they share a lane." Record real logical deps in `blocked_by`. Edit = add/adjust
lines + set `blocked_by`.

### Phase 3 — Orchestrate → **DO NOT write status into the DAG**

When a worktree is spawned for a head-of-line issue, the **worktree owns the
issue lifecycle** (`triaged` → `in-progress` → `fixed`). The DAG must **not**
write its own in-progress flag (that races the worktree's issuectl updates —
see decision 5). Optional, advisory only: annotate `(spawned <run-id>)` next to
a just-launched head as an in-session human breadcrumb; it is cleared by the
next Phase-0 re-derive and is never consulted as truth.

### Phase 7 — Handoff → **RECONCILE + persist**

Before handoff, re-derive once more (landed issues fall off, feedback-born
issues added), refresh the date stamp, and **commit `TODO.md`** so the next
`jatketaan @TODO.md` agent's Phase 0 opens onto an accurate graph. (Phase 5
report already surfaced what landed; Phase 7 just makes the DAG match.)

---

## Design decision 5 — Head-of-line selection (no race)

**The DAG never stores progress.** "Head-of-line" is *computed on read*:

```
for each lane L, in order:
  head(L) = the first issue i in L such that
      status(i) is non-terminal            # issuectl: open OR in-progress
      AND every s in blocked_by(i) is terminal-done
                                           # fixed | done | wontfix
                                           # | duplicate | cannot-reproduce | obsolete
  mark head(L) with ▶
GLOBAL HEAD-OF-LINE = head of the highest-priority lane, or the slug the
  handoff explicitly names as "start here".
across lanes: every head is a parallel candidate, EXCEPT honor cross-lane
  `after … (collision …)` edges — don't start two colliding heads at once.
```

Why this dodges the race with the worktree's own lifecycle updates:

- An issue issuectl marks **`in-progress`** (a live worktree is on it) is still
  its lane's `▶` head — that's where the lane's frontier *is* — but its
  issuectl status tells the orchestrator it is **already being worked, not free
  to spawn again**. So `▶` means "active frontier," and issuectl status (read,
  never written by the DAG) says whether it's free or busy.
- Because the DAG asserts no status of its own, a worktree flipping
  `in-progress → fixed` mid-stint can never contradict the DAG. Next re-derive,
  the fixed issue simply drops out and the next lane item becomes `▶`.

This is the crux: **ordering lives in the DAG, progress lives in issuectl, and
the head-of-line is the read-time join of the two.**

---

## Design decision 6 — Sync boundary with issuectl

The invariant, stated as an ownership table:

| Concern | Authority | In the TODO.md DAG? |
|---|---|---|
| Issue existence / title | issuectl | slug reference only |
| **Status** (open/in-progress/done…) | issuectl | **NO — never written; derived on read** |
| Logical dependency | issuectl `blocked_by` | mirrored as `after … (needs …)` for humans |
| Priority | issuectl | may hint lane / global-head order |
| **File-collision lanes** | repo hot-file list | **YES — the DAG's own value-add** |
| **Ordering / head-of-line** | derived (lanes + status + blocked_by) | **YES — the rendered view** |

**Rule:** the DAG stores only what issuectl does *not* — the lane partition and
the rendered ordering. Everything else is read-through. No status duplication ⇒
no drift ⇒ no reconciliation protocol beyond "regenerate."

---

## Design decision 7 — Staleness / repair

There is no bespoke reconciliation protocol; **every discrepancy is fixed by
re-deriving** at Phase 0 and Phase 7:

| Discrepancy | Repair (automatic on re-derive) |
|---|---|
| Closed issue still listed | not in `ls --status open` → dropped |
| Renamed slug | old slug absent from `ls` → dropped; new slug added to its lane |
| Open issue missing from DAG | present in `ls`, absent from DAG → added (the `comm -3` check surfaces it) |
| `blocked_by` points at an already-done issue | edge satisfied → the `after …` mirror can be dropped |
| Lane no longer matches an issue's real hot files | planner moves the line to the right lane during Phase 2/0 |

The design **mandates the regeneration step** (Phase 0 + Phase 7), which is what
makes the DAG self-healing rather than a slowly-rotting hand-list.

---

## Generic vs project-specific (skill stays generic)

The skill encodes the **convention** — lanes = hot-file families, the
`▶` / `after …` notation, "derive status, never store it," regenerate at
Phase 0 and Phase 7. The **project facts** — the actual hot-file list, the
actual lane set — stay in the repo's `AGENTS.md` / `TODO.md`, referenced, never
hardcoded. A tiny read-only `comm -3` one-liner (above) is inlined as prose; no
helper *script file* is warranted, because lane assignment needs judgment (which
hot-file family an issue's fix touches) and a fully-automatic generator would be
unreliable.

---

## Implementation plan (SKILL.template.md edits)

1. **New "Standing discipline" bullet — the DAG is a derived view.** State the
   one principle: TODO.md carries a lane-based execution DAG that is a *view*
   over open issuectl issues + the repo hot-file list; it stores ordering, never
   status; regenerate it, don't hand-maintain it.
2. **Phase 0** — add the *regenerate + validate* step (drop closed, add missing,
   recompute head-of-line, the `comm -3` staleness check) and "present the ready
   frontier."
3. **Phase 1** — add "insert each fix-now bug into its lane; set `blocked_by` +
   `after …` for any dep."
4. **Phase 2** — note the collision analysis *is* the lane assignment; insert
   planned units; record real deps in `blocked_by`.
5. **Phase 3** — add the guardrail: never write status into the DAG; the
   worktree owns the lifecycle; optional advisory `(spawned <run-id>)` only.
6. **Phase 7** — add "re-derive, refresh date, commit TODO.md so the next
   resume opens onto an accurate graph." (Fold into the existing handoff-block
   update step, which is already committed on its own.)
7. **New short "Execution DAG" reference section** giving the canonical format
   (the fenced stub above) and the head-of-line algorithm, so the skill is
   self-contained.
8. **Project prerequisites** — note the DAG relies on the repo documenting its
   hot-file list (already listed) and keeping the `## Execution DAG` section in
   TODO.md.

All project-specific lane content (supervise/pipeline/workmux families) stays in
`TODO.md`; the skill references "the repo's hot-file list," never the names.
