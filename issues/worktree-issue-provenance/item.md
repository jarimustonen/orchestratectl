---
created: 2026-08-16
updated: 2026-08-20
type: improvement
status: done
priority: high
lane: lifecycle
closed: 2026-08-20
commits:
- hash: 3b549ee
  summary: inject authoritative unlaned review-provenance policy into worker prompts
---

# Worktree-filed issues must not be laned and must record AI-review provenance

## Description

## The problem

A worktree agent that runs a review as part of its unit routinely files follow-up issues for
findings it did not act on. Two things go wrong today:

1. **Those issues can arrive already laned**, i.e. scheduled. A finding that no human has ever
   looked at enters the execution plan as accepted work. The lane-or-close gate — the one place
   a human decides whether a finding deserves a worker round — is bypassed entirely.
2. **Their provenance is thin or absent.** The issue body may mention "surfaced by /llm-review",
   or may not. A later reader cannot tell what was reviewed, which model said it, whether the
   assessment step agreed, or whether four models "agreeing" was independent judgement or the
   same prior repeated four times.

The cost is measurable. In `ossctl` on 2026-08-16, a sweep of the open issue base found roughly
40% of it was speculative hardening produced by review panels with no observed failure behind
it — cosmic-ray scenarios, checks duplicating checks that already existed, and hostile-input
defences on a path where the only actor is the maintainer's own machine. Five issues were closed
in one pass; one had already consumed a worker round before the pattern was spotted.

## What to change

**A worktree-filed issue must not be born laned.** Whatever an agent files during a run lands
unlaned, so the existing `open ∧ ¬laned` triage sweep surfaces it for a human lane-or-close
decision. This is a hard rule, not a default to be overridden by a confident agent: the value of
the gate is precisely that it cannot be self-granted. This applies to any issue an agent files
from inside a run, not only review findings — a review finding is just the common case.

**The source must be visible as an AI review finding**, in a form a reader and the triage skill
can both see, rather than buried in prose that may or may not be written.

**Record review metadata where it can be captured cheaply.** The point is to let a human weigh a
finding without re-running the review:

- which model or models raised it, named individually
- what was under review — the diff, commit range, or artifact, and the unit that ran it
- what the assessment step concluded, if `/assess-findings` ran: the classification, and whether
  the finding was confirmed, deferred, or spun off
- the severity and confidence the review itself assigned
- the run that produced it, so the trail back to the worktree exists

**Multi-model agreement must be recorded as what it is.** "All four models flagged this" reads
as strong corroboration and is currently used that way in issue bodies. It is not independent
confirmation — the models share training and priors, and correlate hardest on plausible-sounding
generic advice, which is exactly the failure mode this work targets. Record which models agreed,
but do not let the count be presented as evidence of severity.

Anything expensive to obtain is out of scope — capture what the review and assessment steps
already produce, not a new instrumentation layer.

## Companion work

`homebase` owns the consuming half: teaching `/triage-unlaned-issues` to evaluate these findings
on **content first** — does this describe a failure that can actually occur here, and what is the
damage if it does — using provenance only as supporting context. Neither half should assume the
other has shipped: this repo's change must stand alone (unlaned + visible source is useful even
with no consumer), and the triage skill must degrade gracefully when the metadata is absent.

## Acceptance Criteria

- [x] An issue filed by an agent from inside a run is never laned at creation.
- [x] Its source is machine-visibly marked as an AI review finding.
- [x] Where a review and assessment produced them, model names, review target, assessment outcome,
  severity/confidence, and the originating run are recorded on the issue.
- [x] Multi-model agreement is recorded as a list of models, not as a corroboration score.
- [x] Absent metadata degrades gracefully — a missing field never blocks filing.

## Tests Run

- [x] `cargo fmt --all --check`
- [x] `cargo clippy --locked --workspace --all-targets -- -D warnings`
- [x] `cargo nextest run --locked --release --workspace`
- [x] `cargo test --locked --release --workspace --doc`
- [x] `RUSTDOCFLAGS="-D warnings" cargo doc --locked --workspace --no-deps`
- [x] Worker prompt materialization test under stripped `PATH=/bin`
- [x] issuectl 0.16 temporary-repository contract probe for unlaned intake, provenance fields, and model labels
- [x] Bundled-skill snapshot loop (`cargo test -p taskfleet`; no snapshots changed)

## Implementation Notes

`run create` now injects authoritative run context into every materialized worker prompt. The
policy uses issuectl intake for unlaned filing, records a stable review source and originating
run in the core call, and enriches optional review metadata afterward. Named models are appended
to the issue's labels list, never collapsed into a corroboration score. `/llm-review` and
`/assess-findings` confirmed seven localized improvements; all were applied.
