# Lifecycle subsystem: architecture map, bug taxonomy, and root-cause analysis

**Issue:** `arch-lifecycle-map-rootcause` (epic `lifecycle-architecture-review`, Lane F Phase 1)
**Status:** READ-ONLY investigation — no application code was changed by this work.
**Date:** 2026-08-12
**Feeds:** the Phase-2 design session (`arch-redesign-design-session`), the ADR
(`arch-decision-rearchitect-vs-harden`), and **◆ DECISION-2** (disposition of every open Lane A + Lane E issue).

This report has three parts:

- **§A** — an end-to-end architecture map of the run / supervisor / agent lifecycle.
- **§B** — a taxonomy of the open cluster-A (supervise/lifecycle) + cluster-B (run-show DTO) issues, bucketed by
  the **signal-combination** (pid × pane × branch × report × timestamp) each one represents.
- **§C** — the root-cause writeup: **inference-by-polling vs protocol-based self-reporting**, with each bucket
  classified as **ESSENTIAL** or **ACCIDENTAL** complexity, and the central hypothesis stated crisply and grounded
  in the code.

Every non-trivial claim is anchored to a `file:line` or an issue slug.

---

## §A. Architecture map: run / supervisor / agent lifecycle

### A.1 The three actors

1. **The CLI process** (`run create`, `run merge`, `run cancel`, `run show`, `run list`, `run wait`) — short-lived,
   appends events, reads projections. Never long-lived.
2. **The agent** — a detached `claude`/`pi`/`aider` process running inside a tmux window/pane in its own git
   worktree. It is the distributed party whose state the whole system is trying to know. It communicates back
   through exactly **one** first-class channel: a terminal `node.report` event (via `run merge` or `node report`).
3. **The supervisor** — one detached `orchestratectl supervise <run-id>` process per run
   (`crates/octl-cli/src/supervise/mod.rs`). It owns the run's lifecycle: it watches the agent, synthesizes
   terminal state when the agent doesn't report, rolls the run up to a terminal status, fires the completion
   hook, and tears the worktree/window/branch down.

The supervisor is the heart of the subsystem and the source of ~all cluster-A complexity.

### A.2 The on-disk state model

State lives under `~/.orchestratectl/runs/<run-id>/` as an **event log + folded projections**
(`crates/octl-core/src/schema.rs`):

- **`events.jsonl`** — the append-only source of truth. Each line is an `Event { ts, seq, kind, run_id, node_id?,
  idempotency_key?, data }` (`schema.rs:841-860`). Event kinds handled by the reducer: `run.created`, `run.status`,
  `run.notified`, `node.created`, `node.status`, `node.report`, `node.retry`, `child.spawned`,
  `discussion.opened`, `discussion.resolved`, `spinoff.proposed`, `spinoff.approved`, `spinoff.rejected`,
  `orchestrator.decision`, `supervisor.attached`, `supervisor.exited`, `discuss.critical`
  (verified against `crates/octl-core/src/reducer.rs`). Note `supervisor.started` / `supervisor.self-terminated`
  fall through the reducer catch-all to a **no-op** — they emit zero projection ops (`stalled.rs:126-129`).
- **`manifest.json`** — folded run projection (`schema.rs:516-601`). Key fields: `status`, `lifecycle`, `kind`,
  `applied_seq` (the watermark), denormalized counters (`node_count`, `open_discussions`, `pending_spinoffs`),
  `source_repo`/`source_branch`, `managed_tmux_session`, `notify_cmd`, `harness`, parent pointers.
- **`nodes/<node-id>.json`** — folded per-node projection (`schema.rs:612-696`). This is where nearly every
  inference signal is stored: `status`, `worktree_path`, `branch`, `base_sha`, `tmux_window`, `tmux_identity`
  (`session:window_id` + `socket` + `pane_id`), `agent_pid`, `agent_pid_start_time`, `supervisor_pid`,
  `last_report`, `retry_attempts`.
- **`discussions/*.json`**, **`spinoffs/*.json`** — human-decision projections.
- **`.lock`** — per-run advisory `flock`.
- **`supervisor.pid`** — the supervisor's own liveness record (pid + start-time).

**State machine.** Both runs and nodes use one `Status` enum: `Pending | Running | Blocked | Done | Failed |
Cancelled` (`schema.rs:465-491`). `Done | Failed | Cancelled` are **terminal** — `Status::is_terminal()`
(`schema.rs:488-491`) — and the reducer freezes `status` once terminal (`apply_run_status` / `apply_node_status`
/ `apply_node_report` are no-ops on a settled node; `schema.rs:456-464`). This is what makes a late agent report
racing a `run cancel` safe.

**`Lifecycle` is a category, never a transition:** `Autonomous | Interactive` (`schema.rs:446-454`), derived from
`Kind` (`Kind::lifecycle`, `schema.rs:402-417`). `Code`/`Orchestrate` are interactive; the other six kinds are
autonomous. This distinction drives most of the supervisor's branch points (whether to auto-teardown, whether a
dead pid is meaningful, whether to run the idle net). Polling `lifecycle` for a terminal state is a known bug
class — invariant 4 in the root `CLAUDE.md` (`skill-progress-polling-wrong-field`).

### A.3 The write pipeline: crash-atomic append-then-apply

All mutation flows through `append_and_apply_*` in `crates/octl-core/src/events.rs`
(`append_and_apply_event:340`, `append_and_apply_unlocked:409`, `append_and_apply_idempotent:589`). The atomicity
contract (root `CLAUDE.md` "State integrity invariants" 1–3):

1. **`applied_seq` watermark** (`schema.rs:521-537`, `events.rs`). The reducer advances `manifest.applied_seq`
   only after every projection an event touches is fsynced. On the next lock acquisition, events with
   `seq > applied_seq` are replayed before any new append — so the log may run ahead of the projections, but the
   gap is always healed before the next writer observes stale state.
2. **`LockedRun` witness** (`crates/octl-core/src/lock.rs:53-57`). A compile-time proof that the caller holds the
   run's exclusive `flock`. Only `RunLock<Exclusive>::witness()` (`lock.rs:128-130`) can mint one; the unlocked
   append entry points require `&LockedRun`, so the type system — not a doc comment — enforces lock-before-write.
   The witness is `!Send + !Sync` (the `PhantomData<*const …>`, `lock.rs:54-56`) so the proof can't cross a thread.
3. **`LOCK_SH` on every multi-file read** (`lock.rs:143-206`, `RunLock::with_shared_lock`). A reader touching more
   than one projection wraps the scan in the shared lock so it never observes a half-applied projection set. The
   reducer holds the exclusive lock while writing. `acquire_shared` never creates the run dir/lock file
   (`lock.rs:161-192`) — a reader must not author state.

Projection writes are typed `ProjectionOp::{Manifest,Node,Discussion,Spinoff}` (`reducer.rs:279-282`), each written
atomically to its own file (`reducer.rs:403-406`).

**Assessment:** this layer is the subsystem's strongest, most defensible part. It is a genuine, well-guarded
crash-atomic multi-writer store. Its complexity is **essential** (see §C).

### A.4 The supervisor tick loop

`boot_supervisor` (`mod.rs:540-639`) resolves paths, installs SIGINT/SIGTERM handlers (`mod.rs:468-493`),
atomically claims `supervisor.pid` (`pid_file::claim_pid_atomic`, closing the §7.6 double-launch TOCTOU,
`mod.rs:566-570`), emits `supervisor.started`, and seeds the own-run tail + one tail per discovered child
(`mod.rs:604-619`). Readiness is confirmed to the spawning `run create` down a pipe **after** boot succeeds
(`supervisor_readiness::ReadinessReporter`, `mod.rs:656-690`) so a slow-but-healthy boot is not mistaken for death.

`dispatch` (`mod.rs:641-...`) then runs the loop. Each iteration:

1. **Tail own + child event logs**, consuming `node.report`s from children (idempotent via
   `last_processed_report_seq_by_child`, `schema.rs:686-687`).
2. **`watchdog_tick`** (`mod.rs:2806-3319`) — the inference core (§A.5).
3. **No-worker guard** (`mod.rs:1184-1246`) — if a run has zero nodes, no children, and empty `spawned_children`
   past the create grace, terminalize it `failed` (`NO_WORKER_REASON`), re-verified under the exclusive lock.
4. **Rollup** (`mod.rs:1258-1286`) — `cleanup::rollup_status` returns a terminal status once all own nodes AND all
   tracked children are terminal; the supervisor appends the terminal `run.status` under a deterministic
   idempotency key. **The reducer never terminalizes a run** — only this rollup does. Without it a successful
   `node.report` leaves the run `pending` forever (`supervisor-complete-run-on-terminal-report`).
5. **Notify + teardown** (`mod.rs:1299-1352`) — on the terminal transition, fire the `--notify` hook FIRST
   (`notify::maybe_fire`), then tear down if warranted (§A.7).
6. Persist cursors (`state::save`, `mod.rs:1370`); sleep alternating `TAIL_TICK` / `WATCHDOG_TICK` (`mod.rs:1392`).
7. Loop-exit gate (`mod.rs:1382-1390`): exit only when `notified && all_work_done`.

Clean shutdown (`mod.rs:1399-1515`) emits `supervisor.exited` (or `supervisor.self-terminated` when the run dir
vanished, `mod.rs:1438-1447`), removes the pid file, and — on a signal — exits 130/143 after flushing logs.

### A.5 The watchdog: liveness inference from indirect signals

`watchdog_tick` (`mod.rs:2806`) is where the supervisor **guesses the agent's state**. It never receives a
"still alive" / "I'm done" message; it reconstructs the answer from proxies.

**Pass 1 — collect candidates under the shared lock** (`mod.rs:2838-2900`): scan `nodes/`, skip terminal nodes,
skip retry-parked nodes (`mod.rs:2868`), skip nodes younger than `WATCHDOG_SPAWN_GRACE` (5s, `mod.rs:436`, guards
the spawn-race false positive), build an `AgentProbe` per live node, and union the distinct tmux sockets. The tmux
snapshot is then collected once per socket per tick (`WatchdogTmuxSnapshot::collect`, `mod.rs:2906`) — batched to
one `tmux list-windows` per socket rather than one per node (`watchdog.rs:176-213`).

**Pass 2 — per-node verdict** (`mod.rs:2916-3302`). For each candidate the supervisor combines **five independent
proxy families**:

- **PID liveness** — `kill(pid,0)` + start-time identity to defeat PID recycling
  (`watchdog::check_liveness`, `watchdog.rs:471-512`). Verdicts: `Alive | Dead | Recycled | TmuxGone`
  (`watchdog.rs:24-47`).
- **tmux window presence** — a **tri-state** `TmuxProbe::{Present, Absent, Unknown}` (`watchdog.rs:138-146`).
  Only a *definitive* `Absent` (server answered, window not there) may flip a node to `TmuxGone`; `Unknown`
  (server down, tmux missing, probe timeout) defers to the PID (`watchdog.rs:485-511`). A wedged server is bounded
  by a 2s timeout (`watchdog.rs:153`) so it can't stall the tick.
- **Lifecycle re-basing** — `check_liveness_for_lifecycle` (`watchdog.rs:549-588`): for an INTERACTIVE node a
  dead/recycled pid with a live window is re-based to `Alive` (the human quit-and-restarted the agent, or it
  re-execed), because the pid is the *wrong signal* there (`agent-died-merge-no-teardown-interactive`, a ~1.5-day
  run whose agent merged 18 min after the watchdog declared it dead, `watchdog.rs:527-543`).
- **half-state streak gating** — `TmuxGone` is committed only after `HALF_STATE_TICKS` consecutive ticks
  (`mod.rs:2932-2937`); `Dead`/`Recycled` commit immediately.
- **git-reconcile probe** — before ever classifying a node `failed` on liveness loss, ask git whether the branch
  already merged into source with a clean worktree (`cleanup::node_branch_merged_to_source`, `cleanup.rs:524-578`:
  ancestor-of-source AND advanced past `base_sha` AND clean worktree). If so the true outcome is SUCCESS with a
  lost report (`false-failed-after-merge` / `supervisor-stuck-pending-after-self-merge`, `mod.rs:2940-3004`). This
  is re-verified under the exclusive lock to close the probe→synthesis TOCTOU (`mod.rs:2960-3015`).

If a node is dead-and-not-reconciled, the supervisor synthesizes a terminal `node.report` itself
(`mod.rs:3067-3158`) — either a `merge-reconciled` success (`VIA_MERGE_RECONCILED`, `cleanup.rs:151`) or an
`agent-died` failure stamped with a `recoverable_work` block if the branch has unmerged commits
(`cleanup::node_recoverability`, `cleanup.rs:639-663`). Empty-handed `Dead` autonomous single-node workers are
instead **parked for bounded auto-retry** (`mod.rs:3016-3066`, `autoretry-agent-died-worker`), bounded by the
durable `retry_attempts` counter (`schema.rs:688-695`).

**Pass 3 — the idle-unmerged safety net** (`mod.rs:3161-3301`). This is the most inference-heavy code in the
codebase and the clearest tell of the root cause. It exists for exactly one failure shape: an autonomous agent
that **committed cleanly-mergeable work, left a clean worktree, then skipped its mandatory `run merge` and dropped
to an idle shell** — pid stays `Alive`, no report ever lands, `rollup_status` returns `None` forever, the run
hangs `pending`, and a supervisor + window + worktree leak per occurrence (`agent-skips-run-merge-idle-pending`).
To distinguish "done and idle" from "still working silently", the supervisor takes the **max of three activity
clocks** (`cleanup::node_idle_unmerged`, `cleanup.rs:837-883`):

1. branch-tip committer time (`cleanup.rs:769-782`),
2. pane-transcript mtime (`agent_log_mtime`, `cleanup.rs:783-790`),
3. a **cumulative-CPU-rate clock** — sampled per node across ticks (`cpu_activity_clock`, `mod.rs:364-407`), with
   a rate floor (`CPU_ACTIVE_FLOOR_CENTIS_PER_SEC`) and a sliding baseline window (`CPU_BASELINE_WINDOW_SECS = 90`,
   `mod.rs:301-313`) to tell a busy-but-silent agent from an idle-TUI trickle. The comment history at
   `mod.rs:326-363` documents this being reopened because a claude idle TUI *does* burn a few centiseconds of CPU,
   defeating an any-delta clock.

Only when every clock is quiet past the threshold, the worktree is clean, and there is committed unmerged work does
the net terminalize the run to a **recoverable** `failed` a human can salvage with `run merge` (`mod.rs:3269-3299`).
Interactive kinds are exempt (a human owns their merge).

### A.6 The teardown gate (`supervise/cleanup.rs`)

Once the run is terminal and cleanup is warranted (autonomous kind OR an interactive kind reached terminal via an
explicit `run merge`, `mod.rs:1335-1336`), `cleanup_terminal_nodes` (`cleanup.rs:216-222`) closes each node's tmux
window, removes its worktree, and deletes its branch. The **destructive decision is gated on the terminal report's
shape** — another inference from the report payload:

- `node_branch_merged` (`cleanup.rs:170-180`): `success: true` AND `via ∈ {explicit-merge, merge-reconciled}` →
  the branch is a confirmed merge, safe to `git branch -D`.
- `node_report_is_blocked` (`cleanup.rs:197-204`): `success: false`, not cancelled, not an explicit merge → a
  human-owned handoff; **must NOT delete the branch/worktree** (`blocked-report-deletes-branch`). Records a
  `cleanup.branch_preserved` audit event instead.
- **Defense-in-depth source-relative check**: on any non-explicit-merge path, `git rev-list --count
  <source_branch>..<branch>` — if the branch has commits not reachable from the run's *recorded source branch*,
  preserve both worktree and branch. The last-resort backstop is `git branch -d` (refuses an unmerged branch) for
  every path except a confirmed `run merge` (root `CLAUDE.md` invariant 5).

### A.7 Notify: the one push back-channel

`notify::maybe_fire` (`notify.rs:77-163`) is the only signal the subsystem pushes *outward* (to the spawning
session). It fires the `--notify` command on the terminal transition, BEFORE teardown, at-least-once, deduped on a
durable `run.notified` marker (`no-completion-notification-to-parent`). The ordering is fixed: scan-for-marker →
spawn-hook → record-marker, all under one exclusive lock (`notify.rs:103-161`) so a crash between spawn and marker
re-fires (a duplicate) rather than dropping the signal. The hook is spawned detached and reaped on a thread so a
hung command can't wedge the single-threaded tick (`notify.rs:177-216`).

This channel is **run-scoped, terminal-only, and one-directional**: there is no "started", "heartbeat", "blocked",
or "progress" push. Everything else the parent wants to know it must *poll* (§A.8).

### A.8 The read surface (cluster B): inference on the reader side too

`run show` (`crates/octl-cli/src/run/show.rs`) and `run list` re-derive run health from the same kind of proxies,
under the shared lock, because there is no stored "run health" field:

- **Supervisor liveness** — `SupervisorView::probe` reads `supervisor.pid` and resolves
  `SupervisorState::{Alive, Dead, NotRecorded, Unreadable, Unknown}` (`dto.rs:26-100`). This five-state enum
  replaced a boolean `alive` that conflated "finished cleanly", "orphaned", and "I/O error"
  (`supervisorview-conflates-states`). `show.rs:91-102` explicitly notes this pairing with `manifest.status` is
  **not transactionally consistent** — the pid file is written under the exclusive lock but *removed* without it,
  and involuntary death is unsynchronized, so the liveness bit is a best-effort point-in-time hint.
- **Stall detection** — `run/stalled.rs` computes three read-time heuristics, each with its own grace window:
  `is_stalled` (undriven orchestrate driver, 12 min, `stalled.rs:76-98`), `is_stillborn` (supervisor died before
  `n-0001`, `stalled.rs:149-160`, `run-wait-stillborn-run-not-detected`), `is_orphaned` (supervisor died mid-run
  with ≥1 node, 15 min grace, `stalled.rs:172-...`, `run-wait-still`). All three are pure functions of
  `manifest.status` + supervisor-pid liveness + timestamps.
- **Landing signal** — `landed` / `landed_method` (`show.rs:34-43`, `run/landed.rs`) re-derives "did the work
  land" from `git cherry` patch-id equivalence, falling back to the report marker
  (`landing-signal-reliable-after-rebase`).
- **Counts** — `count_jsons` counts projection files on disk (`show.rs:315-329`).

The reader is doing the same job as the supervisor — reconstructing distributed state from proxies — which is why
cluster-B bugs are the same *shape* as cluster-A bugs (see §B, §C).

---

## §B. Bug taxonomy: open cluster-A + cluster-B issues by signal-combination

Enumerated from `issuectl ls --status open` and the TODO.md Lane A (26 listed; `worker-process-hang` is
in-progress/parked, not open) + Lane E lists. **25 open Lane A + 3 open Lane E = 28 issues.** They are bucketed by
the proxy signal-combination each represents — i.e. *which indirect signals the supervisor/reader was combining
when it guessed wrong*.

The proxy vocabulary: **PID** (pid + start-time), **PANE** (tmux window/pane presence), **BRANCH** (git
ancestry / ahead-count / worktree cleanliness), **REPORT** (the terminal `node.report`), **CLOCK** (activity
clocks: commit-time, pane-mtime, CPU-rate), **SUP** (supervisor.pid liveness), **TS** (manifest/node timestamps),
**COUNT** (projection-file counts).

### Bucket 1 — PID-liveness inference is the wrong or incomplete signal  (PID × PANE)
The "is the agent alive/working" question answered from pid + window, where the proxy is ambiguous.
- **`legacy-pid-identity-check`** — recycled bare-integer `supervisor.pid` mistaken for a live supervisor (PID
  recycling on the supervisor side; cf. `watchdog.rs:475-483` on the agent side).
- **`watchdog-pane-aware-liveness`** — liveness keys off `window_id`, not `pane_id`; a split interactive window
  whose *agent pane* dies while a user shell pane survives still reads Alive (`schema.rs:715-719`, `watchdog.rs`).
- **`idle-empty-handed-alive-agent-hangs`** — an alive agent that committed NOTHING and never reports: the
  idle-unmerged net (which requires committed work) doesn't cover it, so the run hangs `pending`. A gap *between*
  the PID signal and the CLOCK/BRANCH signals.

### Bucket 2 — Activity-clock inference: "done and idle" vs "still working"  (CLOCK × BRANCH × REPORT)
The most inference-dense subsystem; every one of these is a refinement of the three-clock heuristic, and the fact
that landing the parent fix (`agent-skips-run-merge-idle-pending`) *immediately spawned three of these* is the
canonical demonstration of the root cause.
- **`idle-unmerged-monotonic-clock`** — the CPU clock should use a monotonic `Instant`, not wall-clock, for
  elapsed time (`cpu_activity_clock`, `mod.rs:364-407`).
- **`idle-unmerged-process-tree-cpu`** — sum PROCESS-TREE CPU, not just the agent PID, so buffered child work
  isn't misread as idle (`watchdog::pid_cpu_time_centis`, `watchdog.rs:80-90`).
- **`idle-unmerged-e2e-preservation-test`** — e2e test that a synthesized idle-unmerged report preserves
  branch+worktree through teardown (the intersection of Bucket 2 and Bucket 5b).

### Bucket 3 — Branch × report reconciliation: work exists but the report is missing/late/malformed  (BRANCH × REPORT)
The terminal protocol event failed to convey what the branch already proves.
- **`merge-report-schema-lenience`** — a typo in an ADVISORY report field makes `run merge` REJECT the whole
  report and BLOCK the real code merge → run stuck pending (recurred across 2 workers). The one issue TODO flags
  as a FAST-TRACK, model-independent, merge-first-then-validate fix.
- **`run-salvage-command`** — a first-class command to recover a dead agent's stranded branch (the manual
  counterpart to `node_recoverability`, `cleanup.rs:639-663`).
- **`orchestrate-integration-branch-no-worktree-merge-fails`** — an integration branch created without a worktree
  makes `run merge` fail (BRANCH topology the report/merge path didn't anticipate).
- **`code-run-inject-no-selfmerge`** — inject the no-self-merge prohibition into every code-run SKILL, i.e. narrow
  the protocol so agents stop creating the branch/report mismatch in the first place.

### Bucket 4 — Supervisor existence/liveness inference: "who is watching?"  (SUP × TS)
The supervisor is itself a distributed process whose presence is inferred from a pid file + timestamps; a whole
family of bugs is "the watcher died/never started and nobody noticed".
- **`supervisor-spawn-fails-silently-at-run-create`** (high) — `run create` returns but no supervisor is running
  (the RESILIENCE half of a KEY LEARNING; investigative, no repro).
- **`run-create-back-to-back-no-supervisor`** — the second of two back-to-back `run create`s is left without a
  supervisor.
- **`reattach-does-not-bootstrap-crashed-at-creation-run`** — `run reattach` doesn't bootstrap a child that
  crashed at creation.
- **`cancel-dead-supervisor-recovery`** — extend dead-supervisor liveness recovery to the `run cancel` path.
- **`supervisor-stall-detection`** — supervisor reports `stalled:false` through a multi-hour silent hang; the 6h
  `run wait` default is too long (the read-side `is_orphaned`/`is_stalled` graces are the current partial answer).
- **`autoretry-crash-consistency`** — crash-consistency hardening for the agent-died auto-retry loop
  (`retry_attempts` durability across a supervisor crash, `mod.rs:3016-3066`).
- **`child-supervisor-spawn-exhaustion-lifecycle`** — propagate exhausted-retry state when a child-supervisor
  spawn keeps failing (`child_spawn_action`, `mod.rs:1619`).
- **`moderately-macabre-self`** — verify the reciprocal parent/child relationship before adoption (a child claims
  a parent that doesn't claim it back).
- **`peculiarly-cheerful-mine`** — an orchestrate-driver HEARTBEAT/lease so a silent-stalled driver is detected
  (explicitly DESIGN-FIRST; needs `LockedRun` + append invariants 1–2). **This issue is itself a partial move
  toward a protocol** — see §C.

### Bucket 5 — Teardown gate: report-shape → delete-or-preserve, and its trust boundary  (REPORT × BRANCH)
- **`teardown-gate-trust-and-lifecycle`** — harden the teardown-gate trust boundary + preserved-worktree
  lifecycle (the report payload is agent-authored and partly trusted for a destructive decision,
  `cleanup.rs:170-204`).
- **`interactive-merge-audit-marker`** — a distinguishable audit marker for a human-confirmed merge vs a
  supervisor-reconciled one (`VIA_EXPLICIT_MERGE` vs `VIA_MERGE_RECONCILED`, `cleanup.rs:140-180`).

### Bucket 6 — Outward propagation: the terminal/blocked signal doesn't reach the parent  (REPORT → parent)
The push back-channel (§A.7) is terminal-only and run-scoped, so anything a parent needs mid-flight is missing.
- **`no-completion-notification-to-parent`** — the base gap the `--notify` hook addresses (still open: multi-child
  runs, robustness).
- **`notify-run-level-summary`** — a run-level completion summary for `--notify` on multi-node runs.
- **`uncommonly-fuzzy-swing`** (high) — a spinoff blocked on genuine user input must PROPAGATE to the parent
  (with a delay), not silently block. This is a **missing protocol transition** (`Blocked` exists as a status,
  `schema.rs:472-473`, but has no outward channel).

### Bucket 7 — Watchdog structural / maintainability  (meta)
- **`watchdog-tick-verdict-refactor`** — extract the per-failure-mode blocks of `watchdog_tick` (`mod.rs:2806-3319`,
  a ~500-line function) into named verdicts. Pure evidence that the inference logic has outgrown its structure.

### Cluster B — read-side inference (Lane E, run-show DTO)  (SUP × TS × BRANCH × COUNT)
Same shape as cluster A, on the reader.
- **`run-show-null-worktree-path`** — `run show` reports null `worktree_path`/`source_branch` for a live pending
  run (the fields are read from the node projection that doesn't exist yet; `show.rs:173-180`).
- **`node-show-null-report`** — `node show` returns null report after a spinoff self-merge (the report is in
  `nodes/<node>.json:last_report`, `schema.rs:675-683`, but the read path misses it).
- **`count-jsons-swallows-io`** — `count_jsons` swallows a filesystem read error as a false `0`
  (`show.rs:315-320` returns `0` on `read_dir` error — an inference that can't distinguish "empty" from "unreadable").

**Distribution.** Of 28 open issues: Buckets 1+2+3 (agent-state inference: pid/clock/branch/report) = 10;
Bucket 4 (supervisor-existence inference) = 9; Buckets 5+6 (teardown gate + outward propagation) = 5; Bucket 7 =
1; cluster B (read-side inference) = 3. **~24 of 28 are direct consequences of reconstructing a distributed
process's state from indirect signals** — the two that aren't (`watchdog-tick-verdict-refactor`,
`code-run-inject-no-selfmerge`) are a refactor of, and a prophylactic against, that same inference machinery.

---

## §C. Root cause: inference-by-polling vs protocol-based self-reporting

### C.1 The central hypothesis (stated crisply)

> **The supervisor is an inference engine that reconstructs a distributed agent's true lifecycle state (starting /
> working / blocked / done-and-merged / dead) by polling a cross-product of indirect proxies — PID liveness ×
> tmux window/pane presence × git branch-ancestry × worktree cleanliness × three activity clocks × file
> timestamps — because the agent has exactly one first-class, optional, lossy way to report its state: a terminal
> `node.report`. Since no single proxy is unambiguous and the true state is only partially observable, the number
> of distinct edge cases the supervisor must handle is the combinatorial product of proxy states. Patching one
> cell of that product reliably exposes adjacent cells, so the open-issue count does not shrink under patching.
> Most of this complexity is ACCIDENTAL: it is the cost of an absent protocol, not of the supervision problem
> itself.**

### C.2 Evidence from the code

1. **The agent has no way to say "I'm done" other than the report, and the report is optional and lossy.** The
   entire git-reconcile fallback (`node_branch_merged_to_source`, `cleanup.rs:524-578`, wired at `mod.rs:2940-3004`)
   exists to answer "did it succeed?" from git when the report never arrived. The `merge-reconciled` synthetic
   report (`mod.rs:3072-3091`) is the supervisor *fabricating* the protocol event the agent should have sent.

2. **The agent has no way to say "I'm still working" — so the supervisor infers activity from three proxy clocks.**
   `node_idle_unmerged` (`cleanup.rs:837-883`) takes `max(commit_time, pane_mtime, cpu_rate)`. Each clock was added
   to plug a false-positive the previous clocks left open (the doc comment at `cleanup.rs:803-831` narrates the
   commit-clock → pane-clock → CPU-clock escalation). The CPU clock alone required a rate floor and a 90s sliding
   baseline (`mod.rs:301-407`) because a *silent idle TUI still burns CPU*. This is ~150 lines of heuristic whose
   only job is to guess a boolean the agent could assert in one event.

3. **The agent has no way to say "I died" — so PID liveness is a genuine backstop, but an ambiguous one.** A dead
   pid means "dead" for an autonomous fire-and-forget agent but "normal restart" for an interactive one
   (`check_liveness_for_lifecycle`, `watchdog.rs:549-588`), forcing a lifecycle-branched re-interpretation of the
   same signal. PID recycling forces a start-time identity check (`watchdog.rs:475-483`); tmux unreliability forces
   the `Present/Absent/Unknown` tri-state (`watchdog.rs:138-146`) and streak-gating (`mod.rs:2932-2937`).

4. **The supervisor is itself a distributed process with no liveness protocol — so the reader infers *its* state
   the same way.** `SupervisorView::probe` (`dto.rs:113-...`) + the stillborn/orphaned/stalled heuristics
   (`stalled.rs`) reconstruct "is anyone watching this run?" from a pid file + timestamps + grace windows. The
   comment at `show.rs:91-102` concedes this pairing is not transactionally consistent. Bucket 4 (9 issues) is
   this one missing protocol — supervisor liveness/heartbeat — expressed nine ways.

5. **The combinatorial-explosion signature is documented in the backlog itself.** TODO.md records that landing
   `agent-skips-run-merge-idle-pending` *immediately spawned three more* cluster-A refinements
   (`idle-unmerged-{monotonic-clock,process-tree-cpu,e2e-preservation-test}`). The DAG note calls this "a textbook
   illustration of why we're reviewing this subsystem instead of patching it." That is the hypothesis's central
   prediction — patching a cell exposes neighbours — observed in the wild.

### C.3 Essential vs accidental complexity

**ESSENTIAL** (survives any redesign; a correct distributed-agent supervisor needs these):
- **The crash-atomic event store** — `applied_seq` watermark, `LockedRun` witness, `LOCK_SH` reads (§A.3). Any
  file-based multi-writer store genuinely needs replay-on-recovery and lock discipline. This layer is well-built
  and largely bug-free (no open cluster-A issue targets `events.rs`/`lock.rs`/`reducer.rs` correctness).
- **A terminal-outcome contract** — *some* first-class "I finished, here is my outcome" event must exist. Today
  that is `node.report`; a redesign keeps an equivalent.
- **A liveness BACKSTOP for hard crash** — a process that is `SIGKILL`ed genuinely cannot self-report, so a dead
  process must be detectable externally. PID liveness (as a *backstop*, not the primary signal) is irreducible.
- **Teardown must gate on a merge assertion** — you cannot safely delete unmerged work, so *some* confirmation
  that work landed is required before destroying a branch/worktree (`cleanup.rs` gate, invariant 5).

**ACCIDENTAL** (collapses if the agent reports its own transitions over a protocol):
- **The three activity clocks and their interaction** (Bucket 2; `cleanup.rs:837-883`, `mod.rs:301-407`) — pure
  substitute for a `progress`/`working` heartbeat the agent never sends. **Would vanish entirely** under a
  protocol.
- **The idle-unmerged safety net** (Bucket 2, and half of Buckets 1/3) — a workaround for agents skipping
  `run merge`; a protocol where "done" and "merged" are asserted (or where merge is not the agent's job) removes
  the need to guess.
- **The git-reconcile fallback and synthetic `merge-reconciled` report** (`cleanup.rs:524-578`, `mod.rs:3072-3091`)
  — reconstructing a lost protocol event from git. Accidental: it exists only because the report can be lost.
- **Lifecycle-branched liveness re-basing** (`watchdog.rs:549-588`) — needed only because the pid is the wrong
  signal for interactive runs; an explicit agent-lifecycle event makes the pid a pure backstop with one meaning.
- **The tmux tri-state + streak-gating + pane-aware liveness** (Buckets 1, 7; `watchdog.rs`) — proxy-unreliability
  handling for a signal (tmux window presence) that is a stand-in for "is the agent process still attached".
- **The supervisor-liveness heuristics and their graces** (Bucket 4, 9 issues; `stalled.rs`, `dto.rs`) — a
  substitute for a supervisor heartbeat/lease. `peculiarly-cheerful-mine` (a driver heartbeat/lease) is already
  the backlog groping toward exactly this protocol.
- **The read-side re-derivation of run health** (cluster B; `show.rs`, `stalled.rs`) — the reader re-runs the
  supervisor's inference because run health is computed, never stored. A protocol that records health as state
  makes the read a lookup, not an inference (`count-jsons-swallows-io` disappears when counts are authoritative
  manifest fields rather than a directory scan).

### C.4 What a protocol-based model would change (for Phase 2 / DECISION-2)

The redesign question is: **should the agent push its lifecycle transitions as first-class events**
— e.g. `agent.started`, `agent.heartbeat`/lease-renewal, `agent.blocked{reason}`, `agent.done{merged_sha}`,
`agent.failed{reason}` — with PID liveness demoted to a pure crash-backstop and a supervisor lease making
supervisor-death self-evident? Under that model:

- Bucket 2 (activity clocks) collapses to a lease/heartbeat timeout — **fully removed**.
- Bucket 4 (supervisor existence, 9 issues) collapses to a supervisor lease — the single largest bucket.
- Bucket 6 (outward propagation) becomes trivial: `agent.blocked`/`agent.done` are already events the parent
  tails (the notify hook generalizes from terminal-only to per-transition).
- Buckets 1/3/5 shrink to their essential residue: a crash backstop, a terminal-outcome contract, and a
  merge-assertion teardown gate.
- Cluster B becomes lookups instead of inferences.

The counter-position (harden, don't re-architect) is that the essential residue (§C.3) is real and the crash-atomic
store (§A.3) is sound, so the accidental complexity can be *contained* (e.g. `watchdog-tick-verdict-refactor` +
fast-tracking model-independent fixes like `merge-report-schema-lenience`) without a rewrite. That trade-off is
**◆ DECISION-2 / the ADR (`arch-decision-rearchitect-vs-harden`)** — this report is the evidence base, not the
verdict.

### C.5 One-line summary

> ~24 of 28 open cluster-A/B issues are cells in the cross-product of *proxy* signals the supervisor polls to
> reconstruct an agent's state; they are accidental complexity created by the absence of an agent-lifecycle
> protocol, and they will not shrink under patching. The crash-atomic event store, a terminal-outcome contract,
> a crash backstop, and a merge-assertion teardown gate are the essential residue that any design keeps.
