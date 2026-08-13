---
created: 2026-07-28
updated: 2026-08-13
type: improvement
reporter: jari
status: obsolete
priority: normal
related: ['@interactive-code-run-self-merged']
closed: 2026-08-13
closed_by: adr-decision-2
---

# Code-inject the no-self-merge prohibition into every code-run spawn prompt

_Source: run/spawn.rs / worktree-code_

## Description

From the /llm-review of interactive-code-run-self-merged (openai + anthropic). The actual safeguard against a code-run agent self-merging is a prohibition in the agent's brief — but today that brief is SYNTHESIZED by the spawning orchestrator following worktree-code SKILL prose (step 2). That is fragile: omission, truncation, conflicting user text, or prompt injection can drop it. The CLI gate (interactive_merge_requires_confirmation) is only a tripwire — any caller can pass --confirm-interactive.

Proposal: append an IMMUTABLE policy block in code (run/spawn.rs prompt assembly, or wherever prompt.md is written) AFTER all user/untrusted brief content for every --kind code spawn: 'This is an interactive worktree; do NOT land this branch by any means (run merge, /worktree-merge, workmux merge, direct git merge/rebase/push, terminal node report). After /wrap-up, STOP and idle for the human.' Then test the FINAL prompt written to the run (prompt.md), not just the SKILL template. This makes the prohibition non-optional rather than agent-synthesized.

Also consider (openai finding 5, larger/likely-wontfix): a trusted, commit-bound approval mechanism (approval event bound to run+node+reviewed OID, emitted by a host/UI action outside the agent's tool authority) is the only thing that would make human review an actual invariant rather than an agent-behaviour convention. Record as the known architectural limitation.

Acceptance: every code-run prompt.md ends with the fixed prohibition regardless of brief content; a test asserts it on the materialized prompt.

## Resolution

### 2026-08-13T11:10:20Z · @adr-decision-2

The code kind + its SKILLs are removed; interactive runs let the human own the merge — ADR 0001 (thin supervisor). See docs/decisions/0001-thin-supervisor-vs-harden.md
