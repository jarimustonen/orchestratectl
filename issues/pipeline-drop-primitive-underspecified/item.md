---
created: 2026-07-23
updated: 2026-08-14
type: improvement
status: obsolete
priority: normal
epic: code-pipeline
closed: 2026-08-14
closed_by: agent-cut-pipeline-floor-harness-heavy
---

# code-pipeline: DROP verdict has no primitive — design §2 vs §8 under-specified

## Description

Surfaced while building **T4** (inverted control loop scaffold,
`control-loop-inversion`). The design is internally inconsistent about how a
DROP (a verify finding dismissed with rationale) is represented, and the tier
classification depends on resolving it.

### The contradiction
- **design §2** lists the consequential-decision set as "DECLARE_CONVERGED /
  TRIGGER_RE_SPEC / ESCALATE / any DROP/`PROPOSE_SPINOFF` of a **non-trivial**
  finding." This phrasing treats **DROP as a decision primitive** that can be
  routine (trivial) or consequential (non-trivial), exactly parallel to
  `PROPOSE_SPINOFF`.
- **design §8** (verify → findings → action table) instead says DROP is
  "record **with rationale**" via an **envelope**, with **no typed primitive**
  in the `primitive` column (unlike FIX→`RE_CODE_CHUNK`, SPEC-FLAW→
  `TRIGGER_RE_SPEC`, etc.).

So §2 implies a `Drop { finding_id, rationale }` action subject to the
routine/consequential split; §8 implies dropping is a side-record with no action
and therefore no tier decision at all.

### Why it matters for the loop
The whole point of §0.2 tiering is that **every consequential judgment is
decider-tier and auditable**. If a non-trivial DROP is consequential (per §2)
but there is no primitive for it (per §8), then a real, consequential judgment —
"dismiss this finding" — happens with **no `DecisionEnvelope` and no
`decision_tier` stamp**, i.e. outside the audit invariant the design leans on.
That is the exact failure mode §8's own anti-sycophancy note ("dismissed findings
are recorded with rationale ... triage is auditable since no human watches") is
trying to prevent.

### Current T4 scaffold state
- `FindingVerdict::Drop` exists (a triage verdict), but there is **no
  `Action::Drop`** — faithful to §8, not §2.
- The classification table (`Action::decision_class`) therefore has no DROP arm;
  only `ProposeSpinoff` carries the trivial/substantial split (via an explicit
  `SpinoffScope`, since severity ≠ deferral-scope).
- Noted inline in `action.rs` on `FindingVerdict::Drop` pointing at this issue.

## Proposed resolution (for owner review — principle 4: evidence-backed, not
auto-applied)

Recommend making the two sections consistent by adding a **`Drop { finding_id,
rationale, scope: SpinoffScope }`** (or equivalent) primitive so that:
1. every dismissal produces a `DecisionEnvelope` with a `decision_tier`;
2. a **substantial** drop routes to the decider (consequential), a **trivial**
   one may stay coordinator-tier — same boundary as `ProposeSpinoff`;
3. §8's table gains a `primitive` entry for the DROP row.

Alternative (if the owner prefers §8's envelope-only stance): keep DROP as a
non-action envelope, but then **explicitly state in §2** that DROP is *not* a
primitive and is audited purely as a recorded rationale — and drop it from the
consequential-primitive enumeration to remove the "non-trivial DROP is
consequential" language, since there is no primitive to stamp.

Either way, §2 and §8 must agree. T5 (state machine) should not wire verify
triage until this is settled, or dismissals will silently escape the tier audit.

## Resolution

### 2026-08-14T04:42:34Z · @agent-cut-pipeline-floor-harness-heavy

Superseded by the 0.2 subtractive cut (cut-pipeline-floor-harness-heavy): the code-pipeline subsystem (pipeline/*, floor/*) and the harness heavy layer (bakeoff/conformance/CodeHarness/aider/claude-deepseek) it targeted were deleted. Nothing left to harden/wire/triage. See docs/decisions/0001-thin-supervisor-vs-harden.md D3.
