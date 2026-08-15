---
created: 2026-08-15
updated: 2026-08-15
type: improvement
status: open
priority: normal
epic: lifecycle-architecture-review
---

# Type node.report provenance instead of reason/via string sniffing

## Description

From /llm-review of A6. supervise::outcome::is_supervisor_failure classifies Blocked vs Failed by reason.starts_with("agent-") + a hard-coded reason list, and TerminalOutcome::Merged trusts via:"explicit-merge" from any report author. Teardown is unaffected (Blocked/Failed both -> PreserveWork; a spoofed merge marker still needs success:true and the reducer rejects contradictions), so this is observability/robustness not invariant-5. Persist a typed report origin (Agent | Supervisor | RunMerge{op_id,worker_oid}) on the node.report event and have classify read the typed field.
