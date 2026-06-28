# Validation — checked against current CLI reality

First-pass analysis (the issue body + the wording `worktree-orchestrated`
already shipped) assumed a particular `node report` contract. Checking it
against the actual binary surfaced three differences that the fix had to
correct rather than copy.

## 1. `node report` payload field names were stale

The issue body and the old `worktree-orchestrated` SKILL documented the
payload as `success`, `discuss[]` (with `chosen_path`),
`spinoff_candidates[]`, `wrap_up[]`. The **real** §7.3 schema enforced by
`octl-core/src/report.rs` (`validate_report_payload`) and consumed by the
supervisor (`octl-cli/src/supervise/reducer.rs`) is:

- `success` — required boolean
- `summary` — optional string
- `discussion_items[]` — `{topic, severity, options[]}`
- `spinoff_proposals[]` — `{proposed_title, proposed_kind, rationale}`
- `wrap_up_recommendations[]` — array of strings
- `cancelled` / `reason` — only on a `run cancel`-synthesized report

The validator only rejects *malformed* values of the **known** keys; an
unknown key such as `discuss` / `spinoff_candidates` / `wrap_up` passes
validation untouched and is then silently dropped (the supervisor reads
`discussion_items` / `spinoff_proposals`, never the stale names). So the
issue's example payload would have "worked" — released the supervisor —
while quietly losing every discussion item, spin-off proposal, and
wrap-up note.

**Decision:** all 8 SKILLs document the *real* schema, and the stale
field names in `worktree-orchestrated` were corrected. The SKILLs call
out the dropped-on-mismatch trap explicitly so an agent copying the old
names gets warned.

## 2. `node report` is a write verb; reads use `node show`

Several "Following progress" sections pointed at
`orchestratectl node report <node-id>` as the way to *read* a submitted
report. With the current CLI that subcommand **requires** `--from-file`
and *writes* a report; reading a node's projection is `node show <id>`.
Fixed in `worktree-spinoff`, `worktree-orchestrated`, and `fan-out`.

## 3. `worktree-code` is interactive — report is the post-merge closeout

For `--kind code` the lifecycle is `interactive`: the agent works
autonomously up to `/wrap-up`, then the human runs `/worktree-merge`. A
terminal report terminalizes the run and the supervisor's "work-complete"
exit closes the tmux window — so submitting it *mid-session* would close
the window the user is still working in. The `worktree-code` SKILL
therefore frames the report as the **post-merge closeout**, not an
autonomous-phase step, while still requiring it so the interactive run
does not linger at `pending` either.

## Run-id discovery from inside the worktree

The spawner cannot inject the run id into the `--task` brief — the brief
is consumed by the `run create` call that *generates* the run id. The
agent must discover it at runtime. `derive_branch_name`
(`octl-cli/src/run/create.rs`) names the branch `wt/<short>-<slug>` where
`<short>` is the first 10 alphanumerics of the run id, so:

```bash
short="$(git rev-parse --abbrev-ref HEAD | sed -E 's#^wt/([0-9a-z]{10}).*#\1#')"
run_id="$(ls -1 ~/.orchestratectl/runs/ | grep -m1 "^${short}")"
```

is the authoritative recipe baked into every SKILL. Node id is `n-0001`
for every single-worker kind.

## 4. Live smoke test surfaced a deeper CLI gap (done-criteria #3)

Done-criteria #3 asked to verify, on a real spawn, that the run reaches
`completed`, the supervisor exits, and the tmux window closes. Tested on
this very spinoff (run `01kw7btqhpdgjeh55zga7wghjs`):

- Submitted a valid `node.report` (event seq 4). `node show n-0001` →
  `status: done`, `last_report` populated. **The agent-facing contract
  works.**
- BUT `run show` stayed `status: pending` and supervisor PID 75074
  stayed alive for 18+ minutes. The supervisor does **not** complete the
  run or exit on an agent-submitted terminal report.

Root cause (read from source): `reduce_node_report`
(`octl-core/src/reducer.rs`) terminalizes only the node projection;
nothing rolls it up to the run manifest status or emits a `run.status`
event. The supervisor's `all_work_done` keys off the **manifest**
status, which only `run cancel` (`octl-core/src/cancel.rs`) ever sets.
The watchdog synthesizes reports only for **non-terminal** nodes whose
agent died, so an already-terminal node is skipped — even agent death
emits no `run.status`.

**This means the SKILL fix alone does NOT clear the dangling symptom.**
The agent now correctly reports, but the run still hangs `pending` and
the supervisor still polls forever (exactly the original bug) until the
CLI rolls a terminal node up into a terminal run.

Filed follow-up: **`supervisor-complete-run-on-terminal-report`** (run
completion / supervisor exit on a terminal `node.report`). It is the
prerequisite for the already-filed **`supervisor-close-tmux-on-terminal`**
(window close), whose premise — "the supervisor already exits on a
terminal report" — only holds for the `run cancel` path, not the
`node.report` path.

Per the issue's own instruction, the SKILL fix is NOT blocked on these
CLI gaps; the SKILLs were updated to (a) still mandate the report and
(b) tell the agent the report is its final action and not to wait on /
re-verify / re-submit against the still-pending run, with `run cancel`
named as the interim manual cleanup. The over-claim in the first draft
("transitions the run to completed and closes the tmux window") was
corrected to match verified reality.
