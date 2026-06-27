# help-json-v2-pack — handoff

Status of the three coordinated §14 improvements landed on the
`help-json-v2-pack` branch, and what is deliberately still missing.

## Landed

- **clap-native resolution** (`help-json-clap-native-resolution`):
  `crate::help::resolve_help_request` replaces the hand-rolled argv scan
  with a single clap lenient parse (`ignore_errors` + `disable_help_flag` +
  `allow_external_subcommands` + injected global `--help`). Unknown
  subcommand under structured help → error envelope (exit 1).
- **deprecation convention** (`help-json-deprecation-convention`):
  `[deprecated]` / `[deprecated: <note>]` help-text prefix, parsed and
  stripped by the walker; `deprecated`/`deprecation_note` on flags,
  positionals, and subcommands.
- **richer arg metadata + schema v2** (`help-json-richer-arg-metadata`):
  see the chunk-C commit message for the full field list.

## Still missing / deferred

### `requires` edges — BLOCKED (no clap getter)

The richer-metadata issue asked for `requires` edges alongside
`conflicts_with`. clap 4.6 exposes a public getter for conflicts
(`Command::get_arg_conflicts_with`) but **none** for requirements: the
`Arg::requires` data lives in a private `requires: Vec<(ArgPredicate, Id)>`
field with no `get_requires` accessor (verified against
`clap_builder-4.6.0/src/builder/arg.rs`). So `requires` cannot be projected
without faking it.

Decision: omit the field entirely rather than emit an always-empty or
fabricated one ("don't fake metadata"). The CLI does declare real
requirements today (`run create` `--parent-run-id` ⇄ `--parent-node-id`),
so this is a genuine gap, not a non-issue.

Follow-up filed: **`help-json-requires-edges`** — revisit when clap adds a
public getter, or implement a side-registry keyed by command-path/arg-id
(mirroring the `custom_accepted_values` pattern already used for
`--output`).

### `examples: []` — out of scope

§14 also specifies a per-subcommand `examples` array of
`{description, argv}` pairs. Not in any of the three issues' scope (it needs
authored example content per command, a separate content task), so it was
not added. Track separately if wanted.

### Notes for the next editor

- `conflicts_with` reflects clap's **declared** direction only
  (`task → [prompt_file]`, but `prompt_file → []`). clap stores the edge on
  the declaring arg; symmetrizing would require scanning every sibling.
- Flag aliases use `get_all_aliases` (visible **and** hidden) — every valid
  spelling — which differs from command `aliases` (visible only). Intentional:
  agents need every accepted input form.
