---
created: 2026-06-28
updated: 2026-06-28
type: task
assignee: jari
status: open
priority: normal
epic: orchestratectl-mvp
---

# --help --json: expose requires edges once clap exposes a getter

## Description

v2 structured-help adds conflicts_with (via Command::get_arg_conflicts_with) but omits requires edges: clap 4.6 exposes NO public getter for Arg::requires (private field, no accessor — verified against clap_builder-4.6.0). Real requirements exist today (run create --parent-run-id <-> --parent-node-id). Revisit when clap adds a getter, or build a side-registry keyed by command-path/arg-id mirroring custom_accepted_values for --output. Spun off from help-json-v2-pack (see issues/help-json-v2-pack/handoff.md).
