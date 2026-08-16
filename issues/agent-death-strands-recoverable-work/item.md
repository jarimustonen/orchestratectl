---
created: 2026-07-26
updated: 2026-07-26
type: bug
reporter: claude-code
status: fixed
priority: high
closed: 2026-07-26
---

## Summary

When a supervised agent's process **dies after committing complete, clean, mergeable
work** on its worktree branch — but **before** calling `orchestratectl run merge` — the
supervisor records a terminal `agent-died` **failed** report and preserves the branch, but
does nothing to signal that the stranded work is *recoverable*. The run reads as a total
failure. A human must manually discover the unmerged commits (`git log <source>..<branch>`)
and hand-spawn a salvage. The auto-reconcile path only rescues work that was *already
merged* to source; committed-but-unmerged work falls through the gap.

**Version:** orchestratectl `0.1.0`
**Reporter:** ossctl `/stint` session (2026-07-26)

## Real incident (evidence)

Run `01kyea2hfmy1qnr6aph1tby57r` (`--kind spinoff --headless`, title `contract-command`,
source `main`):

- `node.created` `2026-07-26T04:14:06Z`, agent_pid `70939`, base_sha `3270f8f`.
- Agent worked ~31 min and **committed a complete, green implementation** on its branch
  `wt/01kyea2hfm-contract-command` — commit `ee39196` (2,408 insertions; independently
  re-verified green afterwards: `fmt` clean, `clippy -D warnings` clean, 43 tests pass).
- `node.report` `2026-07-26T04:46:07Z`: `{failed:true, reason:"agent-died",
  summary:"Agent for node n-0001 stopped responding: agent-died"}`; `run.status:failed`;
  `cleanup.branch_preserved {reason:"blocked report"}`.
- Net: a `failed` run whose branch was exactly `source + 1 clean, mergeable, green commit`.
  Nothing in the failure envelope hinted the work was recoverable; salvage was fully manual.

## Root-cause scope (important — this is NOT a false-reap)

The `agent-died` verdict is correct. `check_liveness` (`crates/octl-cli/src/supervise/
watchdog.rs:407`) returns `Liveness::Dead` purely from `pid_file::pid_alive(pid)` →
`kill(pid, 0)` failing. The **underlying agent process genuinely exited**; the supervisor
imposes no max-lifetime kill (the only SIGTERMs are its own shutdown path). So the watchdog
did the right thing.

**Why the agent process itself exited at ~31 min is out of scope for orchestratectl** — it
is an agent-runtime concern (Claude Code / codex process exit/crash/limit), not a supervisor
defect, and orchestratectl cannot prevent it. This issue is strictly about **what
orchestratectl does with the salvageable work a dead agent leaves behind.**

## The gap

The reconcile-to-success branch in `crates/octl-cli/src/supervise/mod.rs` (~L2115) only
converts a dead node to SUCCESS when `cleanup::node_branch_merged_to_source` is already
true (branch merged, terminal report lost). There is **no handling for the strictly more
common case**: the branch has commits *ahead of* source that would merge cleanly, but the
merge was never called. That work is silently downgraded to a `failed` run + a preserved
branch with no recoverability signal.

## Proposed fix (options, pick per design taste)

1. **Minimum (signal-only, low-risk):** on synthesizing an `agent-died` failed
   `node.report`, compute `commits_ahead = git rev-list --count <source>..<branch>` and a
   clean-merge check, and stamp them into the report (e.g. `recoverable:true`,
   `unmerged_commits:N`, `merges_cleanly:true/false`, worktree path). `run show` / `run
   wait` then surface "N unmerged commits recoverable on <branch>" instead of a bare
   failure — a caller (or `/stint`) can decide to salvage without hand-rolling `git log`.
2. **Recovery command:** add `orchestratectl run salvage <run-id>` that fast-forwards /
   cherry-picks the preserved branch into a fresh worktree for review+merge (what the
   `/stint` conductor did by hand this time), or merges directly under a `--no-review`
   flag.
3. **Policy (opt-in, boldest):** a `--merge-on-agent-death=if-clean` run-create policy that
   auto-reconciles a dead node to SUCCESS when the branch merges cleanly — symmetric with
   the existing already-merged reconcile, gated so it never lands unreviewed work unless
   asked.

Option 1 is the safe floor and unblocks tooling immediately; 2/3 are the ergonomic wins.

## Acceptance Criteria

- [ ] An `agent-died` (or other non-cancel agent-death) failed `node.report` carries a
      machine-readable recoverability signal when the preserved branch has clean, unmerged
      commits ahead of source (commit count + clean-merge verdict + branch/worktree path).
- [ ] `run show` and `run wait` surface that signal so a caller can detect stranded,
      salvageable work without manually running `git log <source>..<branch>`.
- [ ] Behavior is unchanged when the branch has no commits ahead of source (genuine
      empty-handed death) or when the branch does not merge cleanly (flagged, not
      auto-merged).
- [ ] No regression to the existing already-merged reconcile-to-success path
      (`node_branch_merged_to_source`).
- [ ] (If option 2/3 taken) a documented, tested recovery/merge path; auto-merge never
      lands unreviewed work unless explicitly opted in.

## Related

- `@false-failed-after-merge` — sibling reconcile case (branch already merged; run
  wrongly `failed`). This issue is the *unmerged-but-recoverable* twin.
- `@cancel-liveness-from-log` — liveness derived from the event log.
- `@agent-died-merge-no-teardown-interactive` — teardown after mid-session agent-died.

## Comments

The `/stint` conductor recovered this incident by hand: verified the preserved branch was
green, spawned a salvage spinoff that fast-forwarded `ee39196`, ran `/llm-review` +
`/assess-findings` (which caught a real `../`-escape floor-bypass bug, fixed in `6c8362a`),
and merged. It worked, but only because the operator thought to check `git log
main..<branch>` on a "failed" run. The tooling should make that recoverability first-class.

### 2026-07-26T13:20:58Z · @claude-code

Acceptance floor (option 1) landed in 10ea129 + 0bae127: agent-died FAILED node.report now carries a machine-readable recoverable_work block (unmerged commit count via rev-list source..branch, clean-merge verdict via FF-check + git merge-tree --write-tree, branch + worktree path). run show and run wait surface it (JSON verbatim + one-line text), gated on a failed status. Empty-handed death and the already-merged reconcile-to-success path are unchanged. Passed multi-model /llm-review + assessment (history/review-agent-death-strands-recoverable-work.md). Option 2 (run salvage) DEFERRED as a clean follow-up (filed separately) to keep this correctness-sensitive supervise diff additive-only. Other deferred follow-ups: multi-node (n-0001) recoverability surfacing, git-subprocess timeouts under the run lock, typed report-extension validation.
