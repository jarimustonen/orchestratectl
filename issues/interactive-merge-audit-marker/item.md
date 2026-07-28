---
created: 2026-07-28
updated: 2026-07-28
type: improvement
reporter: jari
status: open
priority: normal
related: ['@interactive-code-run-self-merged']
---

# Distinguishable audit marker for human-confirmed vs bare interactive merge

_Source: run merge / interactive gate_

## Description

From the /llm-review of interactive-code-run-self-merged (anthropic + openai findings). Today both a human-confirmed code-run merge and an autonomous bare merge stamp the terminal node.report with via:"explicit-merge" — the reducer's confirmed-merge adoption gate and the supervisor cleanup both key on that exact string, so via MUST NOT change. Because of that, a bypass of the review gate (an agent passing --confirm-interactive) is not distinguishable from a legitimate human merge in the event log after the fact — the forensic gap that made the original incident hard to diagnose.

Proposal: when 'run merge --confirm-interactive' lands a Kind::Code run, add an ADDITIVE report field (e.g. interactive_confirmed:true) WITHOUT touching via. A recurrence then becomes greppable. Note: the marker is caller-forgeable, so document it as a forensic aid, not proof. Keep it out of the reducer's adoption/cleanup gates (they stay keyed on via).

Acceptance: additive report field on confirmed code-run merges; validator accepts it; a test asserts a bare autonomous merge lacks it and a confirmed code merge carries it; reducer/cleanup behaviour unchanged.
