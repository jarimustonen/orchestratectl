---
created: 2026-06-28
updated: 2026-06-28
type: task
assignee: jari
status: done
priority: normal
epic: orchestratectl-mvp
closed: 2026-06-28
---

# --help --json: expose requires edges once clap exposes a getter

## Description

v2 structured-help adds conflicts_with (via Command::get_arg_conflicts_with) but omits requires edges: clap 4.6 exposes NO public getter for Arg::requires (private field, no accessor — verified against clap_builder-4.6.0). Real requirements exist today (run create --parent-run-id <-> --parent-node-id). Revisit when clap adds a getter, or build a side-registry keyed by command-path/arg-id mirroring custom_accepted_values for --output. Spun off from help-json-v2-pack (see issues/help-json-v2-pack/handoff.md).

## Resolution (2026-06-28)

clap 4.6.1 / `clap_builder-4.6.0` still ships **no** public getter for
requirement edges (`Arg::requires` / `r_unless` are private fields; `Command`
only exposes `get_arg_conflicts_with` and `get_groups`). Re-verified against
the vendored source.

Rather than hand-maintain a side-registry keyed by command-path/arg-id — which
would silently **drift** from the `#[arg(requires = ...)]` declarations and so
violate the module's "don't fake metadata" principle — `FlagInfo` now reads the
*real* private fields back through `Arg`'s `Debug` projection, the only
drift-free source. Added (strictly additive, default `[]`, so still schema v2):

- `requires: Vec<String>` — unconditional (`IsPresent`) requirement targets.
  Conditional `requires_if` (`Equals(..)`) edges are excluded by design.
- `required_unless_present: Vec<String>` — from clap's `r_unless`
  (`required_unless_present` / `_any`).

`conflicts_with` was already present (P5.1, via the public getter) and is
unchanged.

Implementation: `crates/octl-cli/src/help.rs` (`requires` / `required_unless_present`
+ `debug_field_list` / `quoted_tokens` helpers, with a module note explaining the
Debug-projection choice). Guard tests pin the recovery so a clap upgrade that
changes the Debug format fails CI loudly instead of silently emptying the field:
synthetic-arg unit tests in `help.rs` and a real-tree assertion
(`requiring_flags_expose_the_edge`, `run create --parent-run-id ⇄ --parent-node-id`)
in `tests/help_json.rs`. Snapshots updated.

### Known gap (follow-up candidate)

`required_unless_present_all` writes a **separate** `r_unless_all` field that
`Arg`'s `Debug` does **not** print, so an all-of requirement is not represented.
None exist in the tree today; documented in code. Required-groups / argument-group
walks remain out of scope (the issue's original out-of-scope note). File a
follow-up if an all-of requirement or a required group is ever added.
