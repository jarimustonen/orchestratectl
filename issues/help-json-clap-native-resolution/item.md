---
created: 2026-06-27
updated: 2026-06-28
type: task
reporter: jari
status: in-progress
priority: normal
epic: orchestratectl-mvp
labels: [review-spinoff, help-json]
---

# Resolve --help --json subcommand path via clap lenient-parse instead of hand-rolled argv scan

## Description

crate::help::navigate + detect_json_help_request hand-scan std::env::args to find the subcommand path and output spec. Correct for the current command tree (value-taking flags exist only on leaf commands, so a flag value can never be mistaken for a subcommand) but fragile: a value-taking flag added at a noun level, short-flag clusters (-vh), abbreviated subcommands (infer_subcommands), or non-canonical aliases could diverge from clap's real parse. clap 4.6 exposes ignore_errors(true) + disable_help_flag(true) + try_get_matches_from_mut, which resolves the path and the --output value exactly as clap would (also handles -- for free). Replace the hand scan with a lenient clap parse that walks ArgMatches::subcommand(); keep snapshot tests green. Also enables tightening unknown-subcommand handling to error instead of falling back to root help. Surfaced 4/4 in the help-json-structured review (issues/help-json-structured/review.md).
