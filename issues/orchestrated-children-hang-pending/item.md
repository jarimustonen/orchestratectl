---
created: 2026-06-30
updated: 2026-06-30
type: bug
status: fixed
priority: high
commits:
- hash: b12e13c9da47455dde1d2f97cf155d7efc0ad13c
  summary: 'fix(orchestrate): spawn driver supervisor so orchestrated children terminalize + tear down'
closed: 2026-06-30
---

# orchestrated children hang in pending after merging; no teardown

## Description

**Reporter:** deutschpad main-session agent (Claude Code)
**Version:** taskfleet `0.0.2-alpha` (commit `626400110051ac4b50fe3f2f86245eef6478ce3b`)
**OS:** macOS / darwin 25.5.0
**Reported:** 2026-06-30
**Severity:** High — makes `/orchestrate` unusable in practice; silently hangs the campaign and litters the user's tmux + git worktree state.

When driving an `/orchestrate` campaign, the `--kind orchestrated`
child workers **completed their work and merged it into the integration
branch successfully**, but their runs then **stayed in
`manifest.status: pending` indefinitely** — they never reached a
terminal state (`done`), never submitted a consumable terminal report,
and never tore down their git worktree or tmux window. Because the
runs never went terminal, `taskfleet run wait <child>` blocked
forever, hanging the whole campaign. The user observed three worktrees
+ tmux windows sitting idle ("they're not doing anything and don't
seem to be merging") even though the code was already on the
integration branch.

By contrast, **`--kind spinoff` children behaved perfectly** in the
exact same session (dozens of them across the day): each self-merged
via `taskfleet run merge` and reliably tore down its worktree +
tmux window. The defect is specific to the **orchestrated** kind (or
to how the `--kind orchestrate` driver supervises its parent-pointed
children).

## Setup that triggered it

1. Created an `--kind orchestrate` driver run (`lifecycle: interactive`,
   `supervisor: "orchestrator-in-main-conversation"`), run id
   `01kwbph62aavgrqn11p9cv909v`.
2. Created the integration branch
   `orchestrate/card-content-program-2026-06-30` off `main`.
3. Spawned 3 ready children:
   ```
   taskfleet run create --kind orchestrated \
     --title "f1-migration-schema" \
     --prompt-file /tmp/f1-migration-schema.md \
     --source-branch orchestrate/card-content-program-2026-06-30 \
     --parent-run-id 01kwbph62aavgrqn11p9cv909v \
     --parent-node-id n-0001 \
     --idempotency-key ccp-f1-v1 --output json
   ```
   (and analogous `ccp-f0-v1`, `ccp-fB2-v1`). Note: spawned **without**
   `--headless`, so their tmux windows opened in the user's foreground
   `default` session — a secondary annoyance, but not the core bug.

Child run ids: `01kwbpktay6zxr0t0fhrd7n71v` (f1),
`01kwbpkx004zvbat1x03nstfp0` (f0), `01kwbpkz4b86t72m64cgxqd25k` (fB2).

## Observed behavior

- All three agents **did the work and put their commits on the
  integration branch.** `git log
  orchestrate/card-content-program-2026-06-30` showed each feature
  commit **plus** the `issuectl` link commits. Each child branch was a
  **full ancestor** of the integration branch
  (`git merge-base --is-ancestor <wtbranch> <integration>` → true),
  and each worktree was **clean** (`git status --porcelain` empty).
- Despite that, for **all three**:
  ```
  taskfleet run show <child> --output json
  → data.manifest.status == "pending"
  → data.manifest.nodes  == []        # <-- empty nodes array
  ```
  i.e. the run never went terminal AND the manifest shows **zero
  nodes**, even though a worker node existed and finished.
- `taskfleet run wait <child>` (no `--timeout`) **blocked
  indefinitely** (the gate child never reached `done|failed|cancelled`),
  so the orchestration loop that waits on gate features to fan out the
  next wave never progressed.
- The worktrees and tmux windows were **never torn down** (the user
  found them lingering).

## What the reporter had to do to recover

- `taskfleet run cancel <child>` did push the runs terminal **but
  did NOT tear down** the worktrees or tmux windows.
- Manual cleanup required: `git worktree remove --force <path>` ×3,
  `git branch -D <wtbranch>` ×3, `tmux kill-window` ×3,
  `git worktree prune`.
- Abandoned `/orchestrate` for this campaign and re-ran the remaining
  features as plain `--kind spinoff --headless` children (which work
  reliably).

## Hypotheses for the maintainer (pick whichever the code supports)

1. **Terminal report not consumed / supervisor not winding down (most
   likely).** The child's closing `taskfleet run merge` (or its
   terminal `node report`) succeeded at the git layer but the per-run
   supervisor for an **orchestrated** (parent-pointed) child never
   consumed the report → run stays `pending`, no teardown. Possibly
   the orchestrated child's supervisor wasn't spawned/alive, or the
   report-submission path for parent-pointed runs differs from spinoff
   and silently no-ops.
2. **Manifest never registered the node.** `run show` returning
   `nodes: []` for a run whose worker clearly ran and finished points
   at a node-registration/manifest-tracking gap specific to the
   orchestrated kind — if the node is never recorded, the
   terminal-status roll-up can never fire.
3. **Agent merged manually instead of via `run merge`.** If the
   orchestrated brief/skill led the agent to `git merge` into the
   integration branch directly (rather than `taskfleet run
   merge`), the run would have no terminal report by construction.
   The presence of `issuectl` link commits + the merge suggests the
   agent followed *a* closing recipe; worth confirming from the
   agents' transcripts whether `run merge` was actually invoked for
   `--kind orchestrated`, and if so why it left the run pending. If
   the worktree-orchestrated skill doc's closing step is wrong/
   ambiguous, that's a doc bug; if `run merge` was called and the
   supervisor ignored it, that's a supervisor bug.

## Corroborating signal

`taskfleet run list` for this account shows a long tail of
historical `orchestrated` runs in `cancelled` / `pending` / `failed`
states (e.g. `f-exercise-engine`, `f-schema-v2`,
`f-level-progression`, `f-session-reauth`, …), whereas `spinoff` runs
are almost uniformly `done`. This suggests the orchestrated kind has
been flaky over time, not a one-off.

## Suggested fixes / asks (acceptance criteria)

- Ensure an orchestrated child registers its worker node in the
  manifest at spawn (so `run show` never returns `nodes: []` for a
  live run).
- Ensure the orchestrated child's terminal `run merge` /
  `node report` is consumed by *some* supervisor that then winds the
  run to `done` and tears down the worktree + tmux window — same
  guarantee spinoff already provides.
- Make `run cancel` (and terminal transitions generally) **always**
  tear down the worktree + tmux window, even on the orchestrated
  path.
- Defensive: give `run wait` a sane default `--timeout` (or a
  "merged-but-pending" degraded-terminal detection) so a stuck child
  can't hang an orchestrator forever.
- If the regression is in the worktree-orchestrated closing recipe,
  fix the skill doc + the binary together (the skills are versioned
  with the binary).

## Minimal repro

1. `taskfleet run create --kind orchestrate --title repro
   --source-branch main` → driver id D.
2. `git branch orchestrate/repro main`.
3. `taskfleet run create --kind orchestrated --title c1
   --source-branch orchestrate/repro --parent-run-id D
   --parent-node-id n-0001 --task "trivial change + commit, then close
   via run merge"`.
4. Let the child do its trivial change + close via `run merge`.
5. Observe: commit lands on `orchestrate/repro`, but
   `run show <c1>` stays `status: pending`, `nodes: []`, worktree +
   window linger, `run wait <c1>` never returns.

## Source

`/tmp/taskfleet-orchestrated-hang-bug.md` (transient — copy into
`investigation.md` here if useful while triaging).

