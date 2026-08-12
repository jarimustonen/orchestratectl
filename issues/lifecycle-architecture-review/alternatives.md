# Supervision architecture alternatives

Phase-1 research for epic **lifecycle-architecture-review**, issue
**arch-supervision-alternatives**. Feeds the Phase-2 design session
(`arch-redesign-design-session`) and DECISION-2 (`arch-decision-rearchitect-vs-harden`).

Scope: survey supervision architectures **beyond** today's polling-watchdog, which
_infers_ a distributed process's lifecycle from four indirect signals — PID
liveness × tmux pane presence × git branch state × node report. Compare each
alternative against the current design on **edge-case surface**, **crash-recovery
behavior**, and **implementation complexity**, and show concretely how each would
collapse the current cluster-A edge cases. Research only — no code changes.

---

## 1. TL;DR

- **The recurring bug is not any one signal — it is *inference itself*.** The
  supervisor reconstructs one distributed fact ("is this unit DONE, and was its
  work saved?") by cross-referencing four independent, individually-lossy signals
  (pid, pane, branch, report). Every new combination of those signals is a new
  edge case, so the bug list is combinatorial and patching never shrinks it. This
  matches the epic hypothesis (57% of open issues cluster here) and the root-cause
  framing in `arch-lifecycle-map-rootcause`.

- **Two genuinely divergent directions survive scrutiny, and they pull opposite
  ways.** (1) **Collapse the inference** — make the worker's `run merge` call the
  *only* completion truth and demote the watchdog to a pure crash backstop (the
  "thin model", Option D). (2) **Replace inference with a reliable protocol** — a
  worker-driven state machine written to the append-only event log, reconciled
  only on a lease timeout (Options A+C fused). Direction 1 removes machinery;
  direction 2 adds a channel. Both eliminate the pid×pane×branch×report guessing;
  they disagree on whether the cure is *less* mechanism or a *better* mechanism.

- **The single highest-leverage change is already latent in the code.** The
  watchdog *already* has a git-branch reconcile probe that can detect "the work
  landed" independent of the report, and `run merge` already is a durable,
  locked, event-appending transition. The thin model mostly means **trusting
  those two and deleting the liveness-guessing that races them** — not building
  something new.

- **Exit-code + FIFO signaling (Option B) is a real, low-effort improvement but a
  partial one.** A wrapper that captures the worker's true exit status turns
  "agent-died vs agent-finished" from a *guess* (pid gone → assume died) into a
  *fact* (exit 0 with a merge marker → done; non-zero → failed). It closes the
  liveness half of cluster-A cheaply, but on its own it does not make "was the
  work merged?" reliable — that still needs the branch/report truth.

- **No option removes the need for a crash backstop.** Whatever the happy-path
  signal, the supervisor and the agent can each die at any instant, so *some*
  reconciler must exist. The design question is whether reconciliation is the
  *primary* mechanism (today) or a *rare fallback* behind an authoritative signal
  (every alternative here). Every option below is really a choice of **what is
  authoritative** and **what is fallback**.

---

## 2. Requirements (grounded in this codebase)

The supervisor exists to run this lifecycle for N units in parallel:

1. **Spawn** an external agent process (`claude`, `pi`, `aider`, …) inside a
   dedicated git worktree, hosted in a tmux window (`create.sh` → `workmux add`).
2. **Know reliably when each unit is DONE** — and, critically, *in which sense*:
   finished-and-merged, finished-but-unmerged (blocked handoff), or died. These
   three are different dispositions with different teardown rules.
3. **Merge** the unit's branch back to its recorded source/parent branch
   (`orchestratectl run merge`, which rebases + merges under a lock).
4. **Tear down** the worktree + tmux window + branch — but **only when the work is
   safely landed**. Unmerged work must survive teardown (invariant 5,
   `blocked-report-deletes-branch`).
5. **Survive crashes on both sides without data loss or orphans:**
   - *Agent-process crash* — mid-work, and possibly *after* it already merged
     (`false-failed-after-merge`) or *before* it reported.
   - *Supervisor crash* — at any point, including between "fire completion notify"
     and "tear down" (`no-completion-notification-to-parent`,
     `supervisor-dies-before-worker-node`).
6. **Never destroy an unrelated resource** — not another repo's supervisor, not a
   user's tmux pane that merely `cd`'d into the worktree
   (`find-window-by-path-cross-session-kill`).

Two hard constraints the alternatives must respect, because they are load-bearing
invariants today (see `CLAUDE.md` → *State integrity invariants*):

- **Data-loss asymmetry.** Destroying unmerged human/agent work is far worse than
  leaving an orphan window or firing a duplicate notification. Every current bias
  (interactive liveness prefers the window; notify is at-least-once; teardown is
  gated on terminal outcome) exists to protect this asymmetry. Any alternative
  inherits it.
- **Append-only event log is the source of truth.** State lives under
  `~/.orchestratectl/runs/<id>/` as an event log + projections, mutated only
  through the `LockedRun` witness + `append_and_apply_*` API, with an
  `applied_seq` watermark replayed on the next lock. Any new signal channel should
  land *in the log*, not beside it, or it re-introduces a second source of truth.

### 2.1 What "the current model" actually is (the baseline to beat)

The watchdog (`crates/octl-cli/src/supervise/watchdog.rs`, `mod.rs`) polls on a
tick and computes a `Liveness` verdict per node by cross-referencing:

| Signal | Source | Failure mode as a truth signal |
|---|---|---|
| **PID** | `kill(pid,0)` + `sysinfo` start-time (recycle defense) | Goes stale on interactive agent restart/re-exec; a fire-and-forget agent that *finished* looks identical to one that *died* |
| **Pane** | `tmux list-windows` tri-state (`Present`/`Absent`/`Unknown`) | Server-unreachable ≠ window-gone; a foreign pane can share the cwd; a zombie pane outlives a dead agent |
| **Branch** | `git rev-list <source>..<branch>` ancestry reconcile | Only tells you commits exist / are merged — not whether the agent is *done* producing them |
| **Report** | terminal `node.report` (`success`, `via:"explicit-merge"`, blocked) | Optional and advisory — an agent can finish and merge *without* writing one, or write `failed` *after* the branch already merged |

The verdict then feeds rollup + teardown. The problem: **no single signal is
authoritative**, so the supervisor encodes a growing decision table over their
*combinations*. The cluster-A issues are exactly the cells of that table:

- dead pid + live pane → `agent-died-merge-no-teardown-interactive`
- `failed` report + merged branch → `false-failed-after-merge`
- unmerged branch + idle-alive pid + no report → `agent-skips-run-merge-idle-pending`,
  `idle-empty-handed-alive-agent-hangs`
- `success:false` report + committed branch → `blocked-report-deletes-branch`
- pane shared by a foreign session → `find-window-by-path-cross-session-kill`
- supervisor dies mid-transition → `supervisor-dies-before-worker-node`,
  `no-completion-notification-to-parent`

Each fix adds a guard for one more cell (start-time recycle defense, tri-state
tmux, source-relative ancestry check, at-least-once notify, idle-unmerged CPU
clock). They are individually correct and collectively unbounded. That is the
thing every option below is trying to escape.

---

## 3. Options

Each option is described as a mechanism, then scored on the three axes, then
related to the cluster-A cells it collapses. The framing throughout: **which
signal is authoritative, and what is the fallback?**

### Option A — Worker-driven state machine over a reliable channel

**Mechanism.** The worker is contractually required to announce its own
transitions over a reliable, ordered channel, and the supervisor *only* consumes
those announcements — it does not infer state. Concretely for this codebase, the
reliable channel is the **append-only event log itself**: the worker (via the
bundled SKILL, or a thin `orchestratectl node transition <state>` verb) appends
typed lifecycle events under the run lock:

```
node.spawned → node.working → node.merging → node.merged | node.blocked | node.failed
```

Each transition is a durable, `LockedRun`-witnessed append with an `applied_seq`
watermark, exactly like every other event today. The supervisor's tick becomes a
*reducer over the worker's self-reported transitions*, not a liveness guesser. The
classic reference model is Erlang/OTP supervision: children are linked to a
supervisor that reacts to **explicit exit signals** carrying a reason, not to
polled health — "let it crash" means the crash is *always observed and reported*,
never inferred ([Armstrong 2003](https://erlang.org/download/armstrong_thesis_2003.pdf);
[OTP supervisor principles](https://www.erlang.org/doc/system/sup_princ.html)).
systemd's `Type=notify` is the same idea for OS processes: the unit is not
considered ready/among-the-living by guesswork — it *sends* `READY=1` /
`STOPPING=1` on a socket the manager provides
([sd_notify(3)](https://www.freedesktop.org/software/systemd/man/latest/sd_notify.html)).

- **Edge-case surface:** *Small on the happy path* — the (pid×pane×branch×report)
  decision table collapses to "read the last transition event". The residual
  surface is entirely about the *unreliable half*: what if the worker **stops
  announcing** (crash, hang, or a buggy SKILL that skips a transition)? That is
  handled by a lease (Option C), not by the state machine itself. So A alone
  doesn't remove inference — it *relocates* all of it into a single "worker is
  silent" case instead of a matrix.
- **Crash-recovery:** *Strong for the supervisor, contract-dependent for the
  worker.* Supervisor crash is trivial: state is the replayed event log, so a
  restarted supervisor resumes from `applied_seq` with no lost transitions
  (mirrors event sourcing's core property — rebuild state by replaying the log,
  [Fowler](https://martinfowler.com/eaaDev/EventSourcing.html)). Worker crash
  *before* it announces `node.merged` is the hole: the log's last event is
  `node.working`, which is indistinguishable from "still working" without a
  liveness/lease backstop.
- **Implementation complexity:** *Medium-high.* Needs a stable transition
  vocabulary, a new worker-facing verb, edits to *every* bundled SKILL to emit
  transitions at the right points, and a reducer rewrite. The risk is a **buggy
  or skipped self-report**: a worker that forgets to emit `node.merging` looks
  hung. The channel is reliable; the worker's *discipline* is the new failure
  mode (the OTP model avoids this by making the exit signal automatic at the
  runtime level — we can't, because our workers are opaque LLM processes).
- **Collapses which cluster-A cells:** `false-failed-after-merge` (merge is a
  reported transition, not an inference from branch vs report),
  `orchestrated-children-hang-pending` (explicit `node.merged` ends the wait),
  `interactive-code-run-self-merged` (interactive vs autonomous is just a
  different terminal transition, no liveness bias needed). Does **not** by itself
  fix the silent-worker cases — those need C.

### Option B — Exit-code + named-pipe/FIFO completion signaling

**Mechanism.** Wrap the worker launch so its **true exit status** is captured and
its completion is signaled over a reliable OS primitive rather than inferred from
pid disappearance. Two cooperating pieces:

1. A launcher shim (the tmux pane runs `octl-run-worker <run> <node> -- claude …`)
   that `wait()`s on the child and records the exact exit code plus a completion
   token. POSIX `wait`/exit-status is the canonical "process finished, and here is
   why" signal — the thing a bare `kill(pid,0)` probe throws away.
2. A **named pipe (FIFO)** the supervisor opens for reading and the shim writes to
   on exit. A FIFO is "a reliable, unidirectional, streamed pipe through the
   kernel with filesystem naming"; the reader sees a definite EOF/last-write when
   the writer closes, giving an unambiguous completion edge instead of a polled
   guess ([fifo(7)](https://man7.org/linux/man-pages/man7/fifo.7.html)). The shim
   writes `{exit_code, merged_marker}` then closes; the supervisor's read
   completing *is* the completion event.

- **Edge-case surface:** *Cuts the liveness half sharply, leaves the merge half.*
  "Did the agent die or finish?" stops being a pid guess — a clean exit with code
  0 and a merge marker is *finished*; a non-zero exit is *failed*; a crash is a
  writer that closes the FIFO *without* a completion token (SIGPIPE/short read).
  But "did the work land on the source branch?" is still a separate fact that
  exit code alone doesn't carry — you still consult the branch/report for the
  *disposition* (merged vs blocked-unmerged). So B shrinks the table from 4
  signals to ~2.
- **Crash-recovery:** *Good for agent crash, awkward for supervisor crash.* Agent
  crash: the FIFO writer dies without emitting the token → the supervisor reads a
  truncated stream and classifies "died without completing" reliably. Supervisor
  crash: **FIFOs are not durable** — an unread completion write is lost when the
  reader is gone (SIGPIPE to the writer, or the message simply evaporates). A
  restarted supervisor cannot replay the pipe. So B needs the completion *also*
  persisted (write the token to the run dir before/around the FIFO write), at
  which point the FIFO is just a low-latency wakeup and the *durable* truth is
  back in the log. This is a real limitation, not a detail:
  pipes are for liveness-latency, not for durable state.
- **Implementation complexity:** *Low-medium, and mostly additive.* The shim is a
  small, testable, harness-agnostic wrapper (the merge/report path is already
  harness-agnostic per `crates/octl-cli/CLAUDE.md`), and FIFO read/write is stock
  libc. It does **not** require touching every SKILL. Main cost: getting the
  crash-durability right (don't trust the pipe as the record) and portability of
  the shim's `wait`+FIFO across macOS/Linux. Note this project already learned
  that `flock` isn't on stock macOS (`merge-lock-flock-not-portable-macos`) — FIFO
  semantics differ subtly across platforms too (POSIX leaves read+write open
  undefined; Linux defines it), so the shim must stay in well-defined territory.
- **Collapses which cluster-A cells:** `agent-died-merge-no-teardown-interactive`
  and the recycled-pid class (real exit status replaces the pid/start-time
  heuristic), `idle-empty-handed-alive-agent-hangs` (an idle-but-alive agent that
  is genuinely done still has to *exit or write the token* — no more inferring
  done-ness from idleness + a CPU clock). Does **not** fix
  `blocked-report-deletes-branch` or `false-failed-after-merge` — the
  merge/disposition truth is orthogonal to exit code.

### Option C — Event-sourced state with a worker heartbeat / lease

**Mechanism.** Keep the append-only event log as the sole state (already true),
and add a **lease**: the worker periodically renews a durable heartbeat
(`node.heartbeat` events, or a monotonic `lease_until` field bumped under the
lock). The supervisor treats a node as alive **iff its lease is unexpired**, and
only on lease-expiry does it run *one* reconciliation to decide died-vs-finished.
This is the systemd hardware/software-watchdog pattern lifted to the app layer:
the service must send `WATCHDOG=1` within `WatchdogSec` or the manager acts
([sd_notify(3)](https://www.freedesktop.org/software/systemd/man/latest/sd_notify.html)),
and the lease/lease-expiry model from distributed systems (a lease is a
time-bounded grant that is *self-invalidating* on crash, so no probe is needed to
notice death — [Gray & Cheriton, "Leases", SOSP
1989](https://dl.acm.org/doi/10.1145/74850.74870)). Kubernetes uses exactly this
shape for node liveness (`Lease` objects + `NodeReady`) and delays a Job's
terminal condition until pods are confirmed terminal
([k8s Job](https://kubernetes.io/docs/concepts/workloads/controllers/job/)).

- **Edge-case surface:** *Replaces the whole liveness matrix with one scalar
  comparison* (`now > lease_until`). No pid probe, no start-time recycle defense,
  no tmux tri-state needed for liveness — the lease *is* liveness. The residual
  surface is tuning: lease duration vs. clock skew vs. long-legitimate-silence
  (the same tension the idle-unmerged CPU clock fights today,
  `idle-unmerged-monotonic-clock`). A too-short lease reaps a busy-but-quiet
  agent; a too-long one delays teardown. But it is **one knob**, not a matrix.
- **Crash-recovery:** *Best-in-class and symmetric.* Supervisor crash: replay the
  log, read the latest lease, resume — nothing lost. Agent crash: the lease simply
  expires (self-invalidating — the dead worker stops renewing), and the supervisor
  reconciles at expiry. This is precisely why leases exist: **death is detected by
  the absence of a positive signal, with a bounded, tunable delay, and needs no
  liveness probe at all.** It also cleanly handles the crash-*after*-merge case if
  merge is a logged transition: the reconciler at lease-expiry reads
  `node.merged` and tears down normally.
- **Implementation complexity:** *Medium.* The event log, lock, watermark, and
  replay all already exist — this is adding one event type + one timestamp field +
  a renewal call in the worker loop, and *deleting* pid/tmux liveness from the
  hot path. The cost is (a) requiring the worker to renew (a SKILL/shim
  obligation, same discipline risk as A) and (b) picking lease/skew parameters.
  Uses a monotonic clock, not wall time (the exact lesson already filed as
  `idle-unmerged-monotonic-clock`).
- **Collapses which cluster-A cells:** the entire liveness family —
  `agent-died-merge-no-teardown-interactive` (interactive just renews a longer
  lease while the human is away; no window-vs-pid bias needed),
  `idle-empty-handed-alive-agent-hangs` and `agent-skips-run-merge-idle-pending`
  (a genuinely-done agent's lease lapses → deterministic reconcile, no CPU-time
  heuristic), the recycled-pid and tmux-probe-timeout classes (no pid/tmux probe
  in the liveness decision), and `supervisor-dies-before-worker-node` (pure log
  replay). It composes naturally with A (transitions) and B (lease renewal can
  piggyback on the shim), which is why the strong protocol recommendation is
  **A+C together**.

### Option D — The thin model: `run merge` is the ONLY completion truth

**Mechanism.** Delete inference. There is exactly one way a unit becomes DONE: the
worker calls `orchestratectl run merge`, which — under the run lock — rebases,
merges, appends a durable `explicit-merge` transition, and *that append is the
completion fact*. The supervisor no longer asks "is it done?" from pid/pane/branch
at all. It runs **one** residual job: detect the case where the worker will
*never* call `run merge` (it crashed or is a blocked handoff) and handle *that*.
That residual is a pure crash backstop — rare, not the primary path.

This is the smallest possible design: it accepts that the *only* actor who knows
"my work is complete and here it is" is the worker, and makes the worker *say so
through the one durable, already-correct transaction we have*. Everything else
(pane presence, pid liveness, idle CPU clock, branch reconcile) becomes either
deleted or demoted to "only consulted when the worker demonstrably died without
merging."

- **Edge-case surface:** *Smallest of all options on the happy path* — there is no
  decision table, because there is no inference. `run merge` succeeded → done +
  teardown; it didn't → not done. The entire residual surface is the **negative
  case**: worker died before merging, or finished-but-blocked (deliberately
  didn't merge). Both are *already* handled by existing invariants (the
  blocked-report gate + source-relative ancestry check in
  `cleanup.rs`), which is why this model is mostly *deletion*. The subtle risk: if
  you demote liveness too far, a crashed-before-merge autonomous agent could hang
  forever pending — so D **still needs a minimal liveness backstop** (even if just
  "pid gone AND no merge event AND lease expired → failed"). D without *any*
  backstop is not viable; D with a *minimal* one is the leanest viable design.
- **Crash-recovery:** *Excellent for the merge path, requires a backstop for the
  no-merge path.* Supervisor crash mid-teardown: `run merge` already recorded the
  durable merge transition, so a restarted supervisor re-reads it and completes
  teardown idempotently (this is essentially `reducer-adopt-explicit-merge`, which
  is *already implemented* — the reducer adopts a late explicit-merge report so the
  supervisor stays the sole teardown actor). Agent crash before merge: no merge
  event ever appears → the backstop must eventually terminalize it as failed
  (preserving the branch per invariant 5). So D's crash-recovery is only as good
  as its backstop for the negative case.
- **Implementation complexity:** *Lowest net new code — it is subtractive.* Much
  of it exists: `run merge` is durable and locked; `reducer-adopt-explicit-merge`
  makes a late merge authoritative; teardown-on-terminal-outcome is built. The
  work is *removing* the branch-reconcile-implies-done inference, the idle-unmerged
  synthesizer, and most liveness verdicts, then adding one narrow crash backstop.
  The risk is behavioral, not code-volume: **you lose the safety net that today
  rescues an agent that finished but forgot to call `run merge`**
  (`agent-skips-run-merge-idle-pending` was filed *because* an agent did exactly
  this). D's premise is that the right fix for "agent forgot to merge" is to make
  the SKILL/harness *reliably call merge* (or auto-merge on clean exit via the
  Option B shim), not to keep a heuristic that guesses done-ness from idleness.
- **Collapses which cluster-A cells:** nearly all of them, by construction —
  because they are all inference cells and D removes inference.
  `false-failed-after-merge` (a report can't contradict the merge transition —
  merge *is* the truth), `blocked-report-deletes-branch` (blocked = "did not call
  merge" = branch preserved, no interpretation), `orchestrated-children-hang-pending`
  and `interactive-code-run-self-merged` (done ⟺ merge event; interactive just
  means a human triggers it). The cells it does **not** free-collapse are the
  no-merge negatives (`agent-skips-run-merge-idle-pending`,
  `idle-empty-handed-alive-agent-hangs`) — D's answer is "fix the trigger, keep a
  minimal backstop," which is a *policy* choice the design session must ratify.

### Option E (cross-cutting) — Supervision-tree restart strategies

Not a standalone replacement, but the literature's answer to "what do you *do*
when the backstop fires," and worth naming because the project is already drifting
toward it. OTP/Akka supervisors don't just *detect* failure — they apply a
declared **restart strategy** (`one_for_one`, `rest_for_one`, escalation, with
max-restart-intensity so a crash loop escalates instead of thrashing)
([Akka fault tolerance](https://doc.akka.io/libraries/akka-core/current/typed/fault-tolerance.html)).
The project already has `autoretry-agent-died-worker` (bounded retry) and
`autoretry-crash-consistency` (durable retry-pending marker + CAS branch
deletion). Whichever primary model wins (A+C, B, or D), the *negative*-case
handler should be an explicit, intensity-bounded restart strategy in this OTP
sense, with the retry-pending marker durable in the log — not ad-hoc per-cell
recovery. This is complementary to every option above, not an alternative to them.

---

## 4. Trade-off comparison

Baseline = today's polling-inference watchdog. "Authoritative signal" is the one
fact the design *trusts*; "fallback" is what runs only when the authority is
silent.

| Option | Authoritative signal | Fallback | Edge-case surface | Supervisor-crash recovery | Agent-crash recovery | Impl complexity | Net code |
|---|---|---|---|---|---|---|---|
| **Baseline** (today) | none (4-way inference) | is the whole model | **Combinatorial** (the cluster-A table) | Log replay (good) but re-runs inference | Guessed from pid/pane/branch | — | — |
| **A** State machine (self-reported transitions) | last transition event | needs C for silent worker | Small on happy path; all risk in "worker silent" | Log replay (excellent) | Poor *without* C (last event = `working`) | Medium-high (every SKILL) | + |
| **B** Exit-code + FIFO | worker exit status | branch/report for disposition | ~2 signals (liveness fact, merge fact) | Weak — FIFO not durable, must persist token | **Strong** (true exit code) | Low-medium (shim, no SKILL churn) | + (additive) |
| **C** Event-sourced + lease | unexpired lease | reconcile once at expiry | One scalar (`now > lease_until`) | **Excellent** (replay + lease) | **Excellent** (lease self-invalidates) | Medium (log exists; add event+field) | + / − |
| **D** Thin: `run merge` only | the merge transaction | minimal crash backstop | **Smallest** (no inference) | Excellent for merge path (already built) | Backstop-dependent for no-merge | **Lowest** (subtractive) | −− |

Reading the table: **C and D are the two crash-recovery-strongest, lowest-residual
options, and they embody the divergence** — C makes a *better* authoritative
signal (lease + transitions), D makes a *minimal* one (merge only). B is the cheap
partial win that pairs with either. A is really "C's transitions without C's lease"
and shouldn't ship alone.

---

## 5. Recommendation shortlist (deliberately divergent)

Two primary recommendations that genuinely pull in opposite directions, plus a
cheap increment that helps under either. The design session should treat #1 and #2
as a real fork, not two flavors of one answer.

### Recommendation 1 — *Collapse to the thin model* (Option D + a minimal lease backstop)

**Thesis: the cure is less mechanism.** Make `run merge`'s durable transition the
*only* completion truth, delete the branch-reconcile-implies-done inference and the
idle-unmerged synthesizer, and keep exactly one narrow crash backstop: "pid gone
**and** no merge event **and** lease expired → terminalize failed, preserve
branch." This leans on machinery that already exists and works
(`reducer-adopt-explicit-merge`, the invariant-5 teardown gate) and mostly
*removes* code, which is the fastest way to shrink a combinatorial bug surface.

- **Best when** the design session concludes the real defect is "too many signals,"
  and is willing to make the SKILL/harness reliably call `run merge` (or auto-merge
  on clean exit via a shim) rather than keep heuristics that guess done-ness.
- **Cost accepted:** loses today's "agent finished but forgot to merge" safety net;
  must replace it with a *reliable trigger*, not a guess.
- **Migration:** smallest. Demote the watchdog to the backstop above; keep the log,
  lock, merge path, and teardown gate untouched.

### Recommendation 2 — *Replace inference with a protocol* (Options A + C fused)

**Thesis: the cure is a better mechanism.** Worker self-reports typed transitions
into the event log (`spawned→working→merging→merged|blocked|failed`) and renews a
lease; the supervisor is a reducer over transitions plus a single lease-expiry
reconcile. This is the OTP / systemd-notify / lease model applied faithfully:
liveness is a positive signal with a bounded self-invalidating timeout, and state
is never inferred from indirect proxies.

- **Best when** the session wants the strongest crash-recovery and a principled,
  extensible model (it generalizes cleanly to multi-node runs, orchestrate DAGs,
  and non-claude harnesses), and accepts the up-front cost of a transition
  vocabulary + touching every bundled SKILL + lease tuning.
- **Cost accepted:** worker *discipline* becomes the new failure mode (a SKILL that
  skips a transition looks hung); more moving parts than D.
- **Migration:** larger but incremental — land the lease first (C) to kill the
  liveness matrix, then layer transitions (A) to kill the report/branch inference.

### Recommendation 3 (adjunct, not a fork) — *Ship the exit-code + FIFO shim now* (Option B)

Regardless of the #1/#2 outcome, wrap worker launch in a shim that captures the
true exit status and signals completion over a FIFO (with the token also persisted
durably). It is cheap, harness-agnostic, needs no SKILL churn, and immediately
converts "agent-died vs agent-finished" from a pid guess into a fact — closing the
liveness half of cluster-A. Under #1 it *is* the reliable merge trigger
(auto-merge on clean exit); under #2 it is the natural place to renew the lease.
It de-risks either primary direction and can land first.

**The divergence to resolve in Phase 2 / DECISION-2:** is the lifecycle core best
made *smaller* (trust one transaction, delete the rest — R1) or *smarter* (a real
self-reporting protocol with leases — R2)? R1 optimizes for the fewest edge cases
and least code today; R2 optimizes for principled robustness and future scale.
They are not reconcilable into one recommendation, and that is the point.

---

## 6. Citations

Primary sources (supervision / process-management literature, actor systems, job
runners) and the current code.

**Current code & issues (primary):**
- `crates/octl-cli/src/supervise/watchdog.rs` — the 4-signal liveness verdict
  (PID + start-time, tmux tri-state) and the interactive window-vs-pid bias.
- `crates/octl-cli/src/supervise/{mod.rs,cleanup.rs,notify.rs,reducer.rs}` —
  rollup, teardown gate, idle-unmerged synthesizer, at-least-once notify.
- `crates/octl-core/src/{events.rs,lock.rs}` — append-only log, `LockedRun`
  witness, `applied_seq` watermark, `LOCK_SH` read invariant.
- `CLAUDE.md` → *State integrity invariants* — invariants 1–5, the data-loss
  asymmetry, the teardown/preservation gate.
- Issues: `arch-lifecycle-map-rootcause`, `agent-died-merge-no-teardown-interactive`,
  `false-failed-after-merge`, `blocked-report-deletes-branch`,
  `agent-skips-run-merge-idle-pending`, `idle-empty-handed-alive-agent-hangs`,
  `find-window-by-path-cross-session-kill`, `reducer-adopt-explicit-merge`,
  `no-completion-notification-to-parent`, `autoretry-agent-died-worker`,
  `autoretry-crash-consistency`, `idle-unmerged-monotonic-clock`.

**Supervision & actor systems:**
- J. Armstrong, *Making reliable distributed systems in the presence of software
  errors* (PhD thesis, 2003) — origin of "let it crash": failures are *observed
  and reported* to a supervisor, never inferred.
  https://erlang.org/download/armstrong_thesis_2003.pdf
- Erlang/OTP, *Supervisor Behaviour* (design principles) — children linked to a
  supervisor that reacts to explicit exit signals + restart strategies.
  https://www.erlang.org/doc/system/sup_princ.html
- Akka, *Fault Tolerance* — restart strategies, escalation, max-restart-intensity.
  https://doc.akka.io/libraries/akka-core/current/typed/fault-tolerance.html

**OS process supervision & signaling:**
- systemd `sd_notify(3)` — `READY=1` / `STOPPING=1` ready protocol and the
  `WATCHDOG=1` keepalive/`WatchdogSec` lease.
  https://www.freedesktop.org/software/systemd/man/latest/sd_notify.html
- Linux `fifo(7)` — named-pipe semantics: reliable kernel pipe with filesystem
  naming, EOF on writer close, SIGPIPE, blocking/non-blocking open.
  https://man7.org/linux/man-pages/man7/fifo.7.html

**Leases, event sourcing & job runners:**
- C. Gray & D. Cheriton, *Leases: An Efficient Fault-Tolerant Mechanism for
  Distributed File Cache Consistency* (SOSP 1989) — the self-invalidating
  time-bounded grant that detects death without a probe.
  https://dl.acm.org/doi/10.1145/74850.74870
- M. Fowler, *Event Sourcing* — rebuild current state by replaying an append-only
  event log; the supervisor-crash recovery property every option here relies on.
  https://martinfowler.com/eaaDev/EventSourcing.html
- Kubernetes, *Jobs* — terminal `Complete`/`Failed` conditions added only after
  pods are confirmed terminal (v1.31+); the "delay the terminal verdict until you
  are sure" discipline.
  https://kubernetes.io/docs/concepts/workloads/controllers/job/
- Kubernetes, *Pod Lifecycle* — `Succeeded`/`Failed` phases keyed on container
  exit status, and node liveness via `Lease` objects.
  https://kubernetes.io/docs/concepts/workloads/pods/pod-lifecycle/
