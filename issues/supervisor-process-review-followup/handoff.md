# supervisor-process-review-followup — handoff (DISCUSS items)

The A3 `/llm-review` on supervisor-process (`b1e43ce..6c50c9a`) ran with four models
(Gemini, OpenAI, Claude, DeepSeek) over two rounds, then `/assess-findings`. FIX-class
findings are applied in this branch; SPIN-OFF findings are filed as issuectl issues
(label `review-spinoff`). The three items below are **DISCUSS** — they need *your* call,
not mine, because each is a design-semantics decision (and two would change `design.md`
§7, which the blocker escape hatch says I must not touch).

Full triage: `history/assessment-supervisor-process.md` (gitignored). Raw reviews:
`history/review-supervisor-process*.{md,txt}`.

---

## D1 — Should the parent really wait for the agent to die before consuming a report? (design.md §7.3)

**This needs your call.** All four reviewers flagged it; all four called it a design
decision, not a bug.

§7.3 step 2 says the parent should consume a child's `node.report` only when **both**
(a) the report event is seen **and** (b) the child agent's PID has exited + its tmux
window is gone. The implementation doesn't do (b) — it consumes the report the instant
it tails it.

- **Three of four (Gemini, GPT, Claude) say the code is *better* than the spec:** having
  the *parent* poll the *child's* agent PID is a layering violation; the agent fsyncs
  `node.report` before exiting and the deterministic-ID dedup already makes replay safe,
  so the dual-signal guard buys latency and cross-layer coupling for no correctness gain.
  Their recommendation: **drop the guard from §7.3** and define `node.report` as the
  terminal, immutable commit point (agent must not write meaningful events after it).
- **DeepSeek dissents:** an *interactive* (`code`) agent might write a corrected/second
  report; consuming the first as terminal could strand a still-running agent.

**My read:** the simplification is probably right, and it's the kind of thing that's
cheap to ratify now and expensive to unwind later. But it's a functional contract about
what an agent is allowed to do after reporting — your decision. **If you say "drop it,"
I (or a follow-up) edit §7.3 and add the F4 watchdog as the sole synthesizer for
agents that die *without* a report.** I did **not** touch the code or the spec here.

`design.md §7.3 change needed.`

## D2 — Lock the deterministic-ID formula: does `item_kind` belong in the hash? (design.md §7.3 / §1.4)

**This needs your call — but it's a small one.**

The code hashes `child_run_id : child_node_id : report_seq : item_kind : item_index`.
The §7.3 example text hashes `… : report_seq : i` — no `item_kind`. So a tool that
computes IDs from the literal §7.3 formula would get *different* IDs than taskfleet.

There's no real collision risk either way (the `d-`/`s-` prefix and separate
`discussions/` vs `spinoffs/` dirs already keep the namespaces apart), so `item_kind` is
harmless belt-and-suspenders. The only question is **which string is canonical** before
any external consumer locks onto the wire format. There are none yet, so now is the
free moment to decide.

**My recommendation:** update `design.md` §7.3/§1.4 to *include* `item_kind` (match the
code — it's strictly safer). The alternative is deleting `item_kind` from the reducer.
Either is a one-liner; I left both untouched because the canonical-format choice is
yours. `design.md §7.3/§1.4 change needed.`

## D3 — Who completes a leaf run? (design.md §7.4 — clarification, maybe not a change)

**This needs your call, or at least your memory of the intended design.**

For a *leaf* run (a `spinoff` with no children), the agent writes `node.report` and
exits. But in this commit range the supervisor's own-run tail only handles
`child.spawned` and `run.status` — there is **no `node.report` handler on the own tail**,
and nothing here transitions a leaf run's `run.status` to `done` after its agent reports.

Three possibilities, and I genuinely can't tell which from this code alone:
1. The `node.report → run.status: done` projection lives elsewhere (a core reducer /
   `node report` CLI path I didn't review — it's outside the review range), and this is
   a non-issue.
2. Leaf runs are expected to be closed by the watchdog (agent PID + tmux gone) or by
   `run cancel`, never self-completing — in which case §7.4 should *say* so.
3. It's an actual gap and a leaf supervisor spins until something external stops it.

**My read:** most likely (1) or (2), but the spec doesn't make it explicit, and "the
supervisor runs forever on a finished leaf run" is exactly the kind of thing that bites
weeks later. A sentence in §7.4 naming the leaf-completion path would close it. I filed
this as DISCUSS rather than chasing it because confirming it means reading the
node-report CLI path that's out of the A3 review scope. `design.md §7.4 clarification.`
