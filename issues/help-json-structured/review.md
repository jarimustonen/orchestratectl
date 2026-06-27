# Multi-model review + assessment — structured `--help --output json`

Reviewers: gemini-3.1-pro-preview, gpt-5.5, claude-opus-4-7, deepseek-v4-pro
(via `/llm-review`). Raw transcript: `history/` (consult-llm group run).

## Decision table

| # | Finding | Consensus | Decision | Rationale |
|---|---------|-----------|----------|-----------|
| 1 | Hand-rolled argv pre-scan diverges from clap (flag-value-as-subcommand, short clusters, abbrev, aliases) | 4/4 | **SPIN-OFF** + partial FIX | Latent only: value-taking flags live solely on leaf commands (no child subcommands for a stray value to match), so the misclass cannot trigger for the current tree. Replacing with a clap lenient-parse resolver (`ignore_errors`) is the right architecture but a larger change → `help-json-clap-native-resolution`. Documented the limitation in `navigate`. |
| 2 | `--` end-of-options ignored | 4/4 | **FIXED** | Both `detect_json_help_request` and `navigate` now break on `--`. Test `double_dash_suppresses_json_help_detection`. |
| 3 | Top-level dumps entire recursive tree (firehose vs §14) | 4/4 | **SPIN-OFF** | The issue spec explicitly defined the recursive nested-`subcommands` shape and required a top-level whole-tree snapshot; changing to a shallow/`depth`-bounded shape contradicts the accepted scope. Real §14 tension → `help-json-depth-control`. |
| 4 | `deprecated: false` is misinformation; deepseek: "use `Arg::is_deprecated()`" | mixed | **REJECT (deepseek factual) / KEEP w/ spin-off (design)** | Verified against clap_builder 4.6.0 source: **no** `is_deprecated`/`get_deprecated` getter exists — deepseek's claim is false for our version. The issue explicitly asked for a `deprecated` marker; no flag is deprecated today, so `false` is currently *accurate*, not fabricated. Real deprecation convention → `help-json-deprecation-convention`. |
| 5 | Short-only flags silently dropped | 3/4 | **REJECT (per issue)** | The issue's "Defaults to use" says: flags with no `long` are skipped. By spec, not a bug. `name`-fallback noted for the metadata spin-off. |
| 6 | `index: unwrap_or(0)` is a silent lie | 3/4 | **FIXED** | Now `expect()` — `build()` guarantees a 1-based index; a bogus `0` would mis-sort. |
| 7 | `long_about` not deduped vs `about` (doc said "when distinct") | 1/4 | **FIXED** | Filtered out when equal to `about`. |
| 8 | Logging init before help → non-hermetic help payload | 1/4 | **FIXED** | Help interception moved before `init_logging`; emits with no warnings. |
| 9 | Hidden subcommands serialized without a marker | 1/4 | **FIXED** | `CommandNode.hidden` added (`Command::is_hide_set`). |
| 10 | Pre-parse interception bypasses validation (bogus subcommand → exit 0 root help) | 2/4 | **DOCUMENT + LOCK** | Acceptable for a help *query*; locked intentional via `unknown_subcommand_with_json_help_falls_back_to_root` + doc on `navigate`. Tightening rides with #1's clap-native resolver. |
| 11 | `--output` custom parser → empty `accepted_values` for the primary flag | 1/4 | **SPIN-OFF** | Real fidelity gap (jsonl/json/text/path not surfaced). Folded into `help-json-richer-arg-metadata`. |
| 12 | Missing metadata: flag aliases, `conflicts_with`/`requires`/`global`, value-delimiter, min/max counts, help_heading, positional `env`/`defaults` | 2/4 | **SPIN-OFF** | Larger v2 schema surface → `help-json-richer-arg-metadata`. |
| 13 | `multiple` conflates repeated-occurrence vs multi-value | 1/4 | **SPIN-OFF** | Split into `repeated` + arity in the metadata spin-off; current single bool is correct-but-coarse. |
| 14 | Brittle `starts_with("Create a new run")` test assertion | 2/4 | **ACCEPT (minor)** | Low value to change; the JSON-vs-text assertion is the real guard. Left as-is. |
| 15 | Snapshot brittleness (whole-tree churn; `version` regex over-broad) | 2/4 | **ACCEPT** | Whole-tree snapshot is the intended contract lock; revisit if #3 lands. `version` filter only matches a `"version":` key, currently root-only. |

## Verified facts

- clap 4.6 has **no** `Arg` deprecation getter (`grep` over `clap_builder-4.6.0/src/builder/arg.rs`). #4 deepseek rejected.
- `ignore_errors(true)`, `disable_help_flag(true)`, `try_get_matches_from_mut` all exist → the #1 clap-native resolver is feasible (spin-off).
- The current CLI has no value-taking flag at a noun (non-leaf) level besides global `--output`, which `navigate` handles explicitly → #1 is latent, not a present defect.

## Strongest reviewer

gpt-5.5 (most specific, file:line-anchored, correctly scoped the `--`/validation/firehose cluster). claude-opus-4-7 close second with the most accurate per-case argv tracing. deepseek lost points for the false `is_deprecated()` claim.
