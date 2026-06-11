# orchestratectl MVP — Validation

Assumptions in `design.md` that need empirical confirmation, cross-project coordination, or platform-specific verification before the corresponding child issue can be considered done. Each item lists the unknown, the proposed check, the blocking issue (if any), and the resolution plan if the check fails.

## Critical-path validations

### V1 — `create.sh` structured-stdout patch lands

**Assumption:** `~/.claude/skills/worktree/scripts/create.sh` will be patched to emit structured JSON stdout per `design.md` §8.1. The supervisor cannot extract `agent_pid`, `tmux_window`, or `worktree_path` reliably without this.

**Check:** Patch lands in `homebase` (or wherever the skill family is rooted) with:
- JSON output on stdout matching the §8.1 schema, including `schema_version: 1`.
- Human messages moved to stderr.
- Exit codes 0/1/2 per AGENTS-AI-FIRST-CLI §2.
- Partial-side-effect cleanup on failure paths verified by manual test (interrupt during `git worktree add`, during `workmux add`, during `tmux send-keys`).

**Blocks:** `breakdown.md` issue 10 (`all-kinds-spawn`) and indirectly issue 9 (`supervisor-process`) since the supervisor parses `create.sh`'s output.

**If the patch is rejected by the skill-family maintainer or impossible:** Fall back to having the supervisor parse `tmux list-windows -F` and `git worktree list --porcelain` after `create.sh` returns. Loses partial-failure detection robustness. Tracked as risk if it happens.

### V2 — `agent_pid` discovery from tmux pane

**Assumption:** `agent_pid_hint` returned by `create.sh` may be stale by the time the supervisor reads it (workmux's `tmux send-keys` doesn't synchronously block on the agent starting). The supervisor must re-discover the real agent PID via `tmux list-panes -F '#{pane_pid}'` for the new window, optionally walking child processes if the pane runs a shell first.

**Check:** Spawn each of the 8 kinds at least once on macOS and confirm:
- `tmux list-panes -t '🚀 wt/...'` returns a non-zero PID.
- That PID, or a child of it (Claude Code process), survives 10 seconds (i.e., the agent is actually running, not a transient launcher).
- `kill(pid, 0)` returns 0 while the agent runs, `ESRCH` once it exits.

**Blocks:** `breakdown.md` issue 9 (`supervisor-process`) — watchdog correctness depends on this.

**If discovery is unreliable:** Fall back to mandatory `node.heartbeat` events from each spawned agent kind (modify each skill prompt to emit a heartbeat every 30s). Larger change to the skill family; tracked as Plan B.

### V3 — `kill(pid, 0)` + start-time identity check across reboot

**Assumption:** `kill(agent_pid, 0)` + start-time comparison reliably distinguishes a still-running agent from a recycled PID, on macOS APFS + Linux.

**Check:**
- macOS: `sysctl({CTL_KERN, KERN_PROC, KERN_PROC_PID, pid}).kp_proc.p_starttime` returns a stable wall-clock timestamp for the process across reads.
- Linux: `/proc/<pid>/stat` field 22 (process start time in jiffies since boot) is stable and combineable with `/proc/stat` `btime` to get wall-clock.
- After reboot, an unrelated process with the same PID has a different start time → identity check refuses.

**Blocks:** `breakdown.md` issue 9 (`supervisor-process`).

**If unreliable on a platform:** Use the agent's tmux window presence as the sole liveness signal on that platform; document the platform-specific behavior. Lower assurance but functional.

### V4 — `fs2` flock on macOS APFS under concurrent writers

**Assumption:** Per-run `flock` via `fs2` correctly serializes 10–50 concurrent short-lived `event create` calls on macOS APFS without livelock, starvation, or data loss.

**Check:** Stress-test harness in `octl-core`:
- 50 threads, each acquiring per-run flock, appending one event line, releasing.
- Run 1000 iterations.
- Verify the final `events.jsonl` has 50000 distinct `seq` values, monotonic, no torn lines.
- Measure 99th-percentile lock-acquisition latency. Expectation: < 10 ms on M-series Mac.

**Blocks:** `breakdown.md` issue 2 (`state-schema-crate`) — the flock primitive is implemented there.

**If contention is too high:** Document the per-run write-burst cap in `design.md` and recommend supervisors batch their writes.

## Scale validations

### V5 — Supervisor process count at peak

**Assumption:** 100 concurrent supervisor processes are operationally fine on a macbook-class machine.

**Check:** Synthetic harness spawns 100 supervisor processes (no real agents), each tail-polling a fixture `events.jsonl` at 500 ms cadence. Measure:
- Total resident memory after 5 minutes.
- Total CPU % attributed to `orchestratectl supervise` processes.
- macOS file-descriptor budget consumption.
- User-perceived UI latency (does the system feel laggy?).

Expectation: < 1 GB resident, < 5% CPU, well under the 10240 FD per-process limit.

**Blocks:** Nothing immediately — this is a "fail fast on bad assumption" check. If the assumption is wrong, the design's supervisor-consolidation fallback (`design.md` §7 trade-offs) kicks in.

**If numbers are 2× theoretical:** Document, proceed.
**If 10×:** Halt before issue 10; implement supervisor consolidation (parent handles direct children without separate processes) in scope.

### V6 — `tmux list-windows` polling cost at peak

**Assumption:** Polling `tmux list-windows -F '#{window_name}'` every 500 ms from 100 supervisor processes does not overload the tmux server.

**Check:** Same synthetic harness as V5 but each fake supervisor invokes `tmux list-windows` once per tick. Measure tmux server CPU usage.

**Blocks:** Nothing immediately, same fallback semantics as V5.

**If tmux server CPU exceeds 20%:** Lengthen poll cadence to 2s; document the trade-off (slower watchdog response).

## Design-completeness validations

### V7 — Deterministic-ID dedup actually works

**Assumption:** The deterministic-ID rule (`design.md` §1.4) — `discussion_id = sha256(child_run_id + child_node_id + report_seq + item_index)[:10]` — produces stable, collision-free IDs across parent-supervisor restart.

**Check:** Unit + integration test in `octl-core` that:
1. Simulates a child writing `node.report` with 3 spinoff proposals + 2 discussion items.
2. Runs parent's report-consumption logic, captures emitted IDs.
3. Crashes the parent (or simulates that) after writing 2 of 5 derived events.
4. Restarts parent; re-runs consumption.
5. Verifies: final state has exactly 5 derived events (no duplicates), all IDs match the deterministic formula.

**Blocks:** `breakdown.md` issue 9 (`supervisor-process`) — this is the test of the §7.3 exact-once promise.

**If deterministic dedup misses a case:** Add scan-before-write as a belt-and-suspenders measure, but only if a real failure mode is found.

### V8 — `run reattach` end-to-end

**Assumption:** `orchestratectl run reattach <run-id>` can restart a supervisor that died mid-run and resume report consumption from the correct `last_processed_report_seq_by_child` position.

**Check:** Integration test:
1. Start a run with one spawned child agent (mock agent that writes a report after 2 seconds).
2. Kill the parent supervisor after the child reports but before the parent processes (i.e., during the 5 ms after report arrives, before §7.3 step 3 completes). This is racy; instead use a fault-injection hook in the supervisor to crash deterministically after step 2 of §7.3.
3. Run `orchestratectl run reattach <run-id>`.
4. Verify: the report is consumed exactly once; deterministic-ID dedup catches the partial write.

**Blocks:** `breakdown.md` issue 9.

### V9 — `run cancel` synthesized-report propagation

**Assumption:** `run cancel` synthesizing terminal `node.report` events does not break the parent supervisor's expectations.

**Check:** Integration test where a parent has 3 children, then issues `run cancel` on one child. Verify:
- Parent supervisor receives the synthetic `node.report` with `cancelled: true`.
- Parent marks the cancelled child `node.status: done` (or `cancelled` — chose one in implementation).
- Parent does not spawn any spinoffs from the cancelled report (since `spinoff_proposals: []`).
- Parent's tail-follow loop on the cancelled child's run is cleaned up.

**Blocks:** `breakdown.md` issue 9.

## Cross-project validations

### V10 — `issuectl` `--add-commit` field for child-run linkage

**Assumption:** `orchestratectl spinoff approve` invoking `issuectl new` produces an issue cleanly linked back to the parent run's epic (if applicable).

**Check:** Manual test, hand-spawn the chain, verify issuectl's resulting issue has the right `Refs-Issue:` trailer / metadata.

**Blocks:** `breakdown.md` issue 12 — but only the auto-issue-materialization path. Manual `--issue-slug` always works.

## Notes

- This document is updated as validations are run; each item moves to a "Resolved" section with the actual measured numbers / observations.
- Items V1–V4 must complete before the corresponding `breakdown.md` issues can be marked done.
- Items V5–V6 are gates: if they fail catastrophically, the design must change before more code lands.
- Items V7–V9 are correctness tests that must pass for the supervisor-process and run-reattach issues to ship.
