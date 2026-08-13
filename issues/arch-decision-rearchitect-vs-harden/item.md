---
created: 2026-08-12
updated: 2026-08-13
type: task
status: done
priority: high
epic: lifecycle-architecture-review
labels: [decision, architecture]
closed: 2026-08-13
closed_by: adr-decision-2
---

# DECISION (ADR): harden the current model vs re-architect the lifecycle core

## Description

PHASE 3 (THE decision point). Drive the call to a recorded ADR (/worktree-technical-decision): keep-and-harden the polling-inference supervisor, or re-architect to the protocol/state-machine model chosen in the design session — with the phase-1/2 evidence, a migration sketch, and the blast radius. Its outcome GATES the disposition of every open cluster-A/B issue (see the DAG decision node). Blocked by arch-redesign-design-session. Deliverable: an ADR in docs/decisions/.

## Resolution

### 2026-08-13T11:11:04Z · @adr-decision-2

ADR recorded at docs/decisions/0001-thin-supervisor-vs-harden.md: THIN supervisor model (run merge = the only success-completion truth; pid×pane×branch×clock inference deleted) + hardenings A1–A6; protocol/self-report+lease DEFERRED to 0.2.1 as a pi.dev plugin; engine 9 kinds -> 2 topologies + --interactive + recipe skills; pi.dev universal default; clean-break migration + doctor prune + read-only legacy decoder. Per-issue re-triage of all Lane A + Lane E issues applied via issuectl (12 obsolete-closed, 7 deferred-0.2.1, 6 keep-0.2, 3 rescope-0.2). signal-exit-143-regression carved out (parallel worktree), untouched.
