---
name: fan-out
description: Fan out N≥5 similar, fully independent units of work as parallel autonomous worktrees via `orchestratectl run create --kind fan-out` (top-level driver) plus one `--kind fan-out` child per unit (parent-pointed). Each child commits a disjoint output file inside the current git repo and merges itself back. Manages enumeration, concurrency (default 10), manifest-tracked state, and resume. Requires a git repo with a clean source branch. NOT for generic parallel commands, dependent workflows, shared-file edits, or tasks needing per-unit human review. For heterogeneous dependency-ordered features (a DAG rather than identical units), use `/orchestrate` instead.
version: 1
cli_version: "{{CLI_VERSION}}"
schema_version: 1
---

# fan-out

A **fan-out** is a top-level driver run plus N≥5 sibling children, each
processing one of N similar, fully independent units. Every child writes
a disjoint output file (so siblings never edit the same path), commits,
and merges itself back. The driver tracks per-unit state in the run's
manifest and supports resume across interruptions.

Read `orchestratectl-overview` first; read `worktree-spinoff` for the
shared autonomous-merge contract; read `worktree-orchestrated` to
understand the child-spawn pattern (`fan-out` uses the same
parent-pointer mechanism but the children are homogeneous instead of
DAG-ordered).

## When to use

- ✅ N≥5 fully independent units doing the same operation on different
  inputs ("convert each receipt PDF to JSON", "regenerate each NFO
  file in `media/`", "apply codemod across 47 packages").
- ✅ Each unit's output is a disjoint file or path (siblings cannot
  race).
- ❌ Fewer than 5 units → just spawn `/worktree-spinoff` N times.
- ❌ Units share output (edit the same file) → serialize via
  `/worktree-code` or restructure to disjoint outputs.
- ❌ Units have dependencies (B needs A's output) → that is a DAG,
  use `/orchestrate`.
- ❌ Per-unit human review required → use `/worktree-code` per unit.

## Workflow

### 0. Validate context

1. Working directory must be a git repo with a clean current branch.
2. Enumerate the units. The enumeration MUST be deterministic and
   re-derivable from the source branch state (file glob, JSONL input
   file, generated list committed to the repo). Re-enumeration on
   resume must produce the same list in the same order; otherwise the
   manifest cannot resume correctly.
3. Decide concurrency. Default is 10. Raise only if the units are
   genuinely cheap and the machine can take it; lower for heavy
   per-unit cost (model calls, IO bottlenecks).
4. `orchestratectl version --output json` to confirm
   `{{CLI_VERSION}}`.

### 1. Build the unit brief template

Each child gets a self-contained brief with one unit interpolated.
The template should include:

1. **Per-unit objective** — what to produce for unit `<id>`.
2. **Input path** — the source the unit reads (the enumerated input).
3. **Output path** — the disjoint file the unit writes. Use the unit
   id or a hash so two unit ids cannot collide.
4. **Done criteria** — output file exists, committed.
5. **No spin-offs, no discuss items** — children should run silently
   to completion; surfacing every receipt OCR run as a discussion
   item drowns the user.
6. **Closing step** — every child MUST end with a single
   `orchestratectl run merge "$run_id"` call (see "Terminal report
   (mandatory)" below), which merges the unit back AND submits its
   terminal report in one step. Without that report the unit's node
   never reaches `completed`, the driver never sees `child.report`, and
   that slot stays occupied — stalling the whole batch at the
   concurrency limit.

### 2. Create the driver run

```
orchestratectl run create \
  --kind fan-out \
  --title "<batch-slug>" \
  --task "Driver brief: enumerate <N> units, fan out at concurrency <C>" \
  [--source-branch <branch>] \
  [--idempotency-key <key>]
```

The driver's `--kind fan-out` recipe writes the manifest with all
enumerated units in `pending`, then begins fanning out children up to
the concurrency limit. The driver itself merges nothing.

### 3. Fan out children

For each unit, the driver spawns a child via the same CLI:

```
orchestratectl run create \
  --kind fan-out \
  --title "<batch-slug>/<unit-id>" \
  --task "<unit-specific brief>" \
  --source-branch <integration-branch-or-source> \
  --headless \
  --parent-run-id <driver-run-id> \
  --parent-node-id <driver-node-id> \
  --idempotency-key <batch-slug>-<unit-id>
```

Per-unit notes:

- Use a stable `--idempotency-key` per unit (e.g.
  `<batch-slug>-<unit-id>`) so retries on transient errors do not
  double-spawn the unit.
- Prefer `--headless` for fan-out children: a batch of N≥5 (often 20)
  windows would otherwise flood the user's foreground tmux session. With
  `--headless` they land in a detached `headless` session
  (`tmux attach -t headless` to watch); auto-cleanup still closes each
  window on terminal. Use one shared `--tmux-session <batch-slug>` if you
  want the whole batch in its own named session.
- The `child.spawned` event lands on the driver's log; the driver's
  supervisor spawns each child's supervisor (single-arbiter
  invariant).
- Children inherit the same `--kind fan-out` so their tmux windows
  carry the fan-out emoji (🪭) and the supervisor applies the
  fan-out merge policy (merge to integration branch, no review by
  default).

### 4. Drive concurrency + resume

The driver tails its own event log and the manifest:

- Up to `<C>` units in `status: running` at any moment.
- On a child completing (`status: done`, `child.report`
  arrives), mark the unit `done` in the manifest and spawn the next
  pending unit.
- On a child failing, retry the unit up to N times with the same
  idempotency key. If it still fails, mark `failed` in the manifest
  and continue; surface the count at the end.
- On driver interruption (Ctrl-C, supervisor crash), `orchestratectl
  run reattach <driver-run-id>` rebuilds state from the manifest +
  event log and resumes from the first non-`done` unit.

### 5. Success envelope (driver)

```json
{
  "schema_version": 1,
  "data": {
    "run_id": "01HZ-DRIVER",
    "supervisor": 12345,
    "kind": "fan-out",
    "lifecycle": "autonomous",
    "tmux_window": "🪭 wt/<batch-slug>",
    "branch": "wt/<batch-slug>"
  }
}
```

### 6. Report to the caller

Tell the user:

- Driver run id, total unit count, concurrency.
- Source/merge branch (every child merges here).
- How to follow: `run show <driver-run-id>` for aggregate counts,
  `event tail <driver-run-id> --follow` for per-unit events.
- Estimated wall-clock if the per-unit cost is known.
- That `run reattach` is the resume path on interruption.

## Terminal report (mandatory)

The merge **is** the closing step for a child unit. A single
`orchestratectl run merge "$run_id"` rebases + merges the unit's branch
back into its source branch AND submits the terminal report (stamped
`via: "explicit-merge"`) in one call. Until that report lands the child's
supervisor keeps polling, the unit never reaches `completed`, the driver
never sees `child.report`, and the slot stays occupied — so the batch
stalls at the concurrency limit instead of fanning out the next pending
unit.

A typical fan-out unit runs silently and has nothing structured to
surface, so the minimal form is what the unit brief should instruct each
child to run **as its final action, before its session ends**:

1. **Discover the run id** from inside the worktree. The branch is
   `wt/<short>-<slug>`, where `<short>` is the first 10 alphanumerics of
   the run id:

   ```bash
   short="$(git rev-parse --abbrev-ref HEAD | sed -E 's#^wt/([0-9a-z]{10}).*#\1#')"
   run_id="$(ls -1 ~/.orchestratectl/runs/ | grep -m1 "^${short}")"
   ```

2. **Merge and report in one step.** No `--source` is needed — the
   fan-out source/integration branch is the child's recorded
   `source_branch`, and `run merge` defaults to it:

   ```bash
   orchestratectl run merge "$run_id"
   ```

   With no `--report-file`, `run merge` submits a minimal
   `{success: true, summary}` report — exactly what a silent fan-out unit
   needs. On a clean merge the child's supervisor winds the unit down,
   mirrors `child.report` onto the driver's log, closes the tmux window,
   removes the worktree, deletes the branch, and frees the slot for the
   next pending unit. No manual tmux/git cleanup.

   On a merge conflict/failure `run merge` exits non-zero with
   `error.code: "merge_failed"` and submits **no** report — the node
   stays live. Resolve the conflict (or run `/complex-rebase`) and re-run
   `orchestratectl run merge "$run_id"`.

3. **If the unit genuinely has follow-up** — discussion items, spin-off
   proposals, or wrap-up recommendations — write a §7.3 payload to a temp
   file and pass it with `--report-file`. The file is validated **before**
   the merge, so a malformed report aborts cleanly without merging. Use
   these exact field names; an unknown key like `discuss` /
   `spinoff_candidates` / `wrap_up` passes validation but its contents
   are silently dropped.

   ```bash
   cat > /tmp/node-report-${run_id}.json <<'JSON'
   {
     "success": true,
     "summary": "<unit-id>: <one-line outcome>",
     "discussion_items": [],
     "spinoff_proposals": [],
     "wrap_up_recommendations": []
   }
   JSON

   orchestratectl run merge "$run_id" --report-file /tmp/node-report-${run_id}.json
   ```

   - `success` — **required** boolean. `true` when the unit's output
     committed and merged; `false` when the unit failed (the driver
     records it as `failed` and continues).
   - `summary` — optional one-line result; prefix with the unit id so
     the driver's aggregate log is legible.
   - `discussion_items[]` / `spinoff_proposals[]` /
     `wrap_up_recommendations[]` — normally empty for fan-out units; see
     `worktree-spinoff` for the full per-field shape if a unit genuinely
     needs to surface one.

This closing step is **not optional**. A merged unit with no report holds
its concurrency slot forever — and `run merge` is what submits that
report, so there is nothing extra to run after it.

## Issue Management

Fan-out children do NOT touch `issuectl`. The driver owns issue
interaction (typically a single epic issue closed when the batch is
done) so that N children referencing the same issue do not race.

## Errors

Likely codes:

- `invalid_arguments` — missing/empty `--title` or `--task`, bad
  concurrency value.
- `enumeration_empty` — the enumeration step produced zero units.
  Refuse to spawn; surface the cause.
- `manifest_corrupt` — resume detected a manifest schema mismatch.
  Tell the user to inspect `<dir>/manifest.json`; do not silently
  drop units.
- `child_spawn_failed` — see `worktree-orchestrated`; the driver
  retries with the same idempotency key.
- `worktree_create_failed` on a child — the driver records the unit
  as `failed` and continues with the rest; do not abort the whole
  batch on one git refusal.

## Following progress

- `orchestratectl run show <driver-run-id>` — aggregate counts
  (pending / running / done / failed) and the next units to fan out.
- `orchestratectl event tail <driver-run-id> --follow` —
  authoritative stream; `child.spawned` and `child.report` events
  arrive here per unit. For per-child progress read
  `data.manifest.status` (terminal: `done | failed | cancelled`) via
  `orchestratectl run show <child-id>`.
- `orchestratectl node list <driver-run-id>` — per-unit table.
- `orchestratectl node show <child-run-id> <child-node-id>` — terminal report for one
  unit (the child's closing `orchestratectl run merge` is what *writes*
  it, merging and reporting in one step; see "Terminal report
  (mandatory)").
- `orchestratectl run wait <child-id> <child-id> …` — **block until
  every** listed child settles (terminal `done | failed | cancelled`),
  with the correct backoff baked in. This is the multi-run primitive:
  pass all child run-ids to wait for the whole batch in one process
  (add `--any` to return on the first, `--fail-on-error` to exit
  non-zero if any child failed). Use it instead of a `for id in …`
  shell loop — that pattern silently word-splits under zsh.

## Install or upgrade `orchestratectl`

This skill was installed for `orchestratectl {{CLI_VERSION}}`. Compare
`.data.version` from `orchestratectl version --output json` to
`{{CLI_VERSION}}`:

- **Missing**: install via Homebrew / Cargo / shell installer.
- **Older**: ask the user to upgrade; stop — fan-out child-spawn
  semantics may have changed.
- **Newer**: `orchestratectl skill install --force` (or just `fan-out
  --force`).
- **Equal**: proceed.

## Example

```
/fan-out Convert every PDF in corporate/receipts/2026-05/ to JSON via vision OCR — one output file per input under corporate/receipts/2026-05/json/
```
