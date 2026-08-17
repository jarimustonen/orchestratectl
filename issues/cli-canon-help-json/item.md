---
created: 2026-08-16
updated: 2026-08-17
type: improvement
status: in-progress
priority: normal
labels: [cli-canon, tooling]
lane: cli-canon
lane_seq: 60
commits:
- hash: 2becf633ca1828c6eccf8c58f5dbd61a6bb03a5
  summary: 'fix(help): honor global --json with --help'
- hash: 8c9745824f58665d8411548adca6af3afe60a701
  summary: 'fix(help): keep shorthand resolver lint-clean'
---

# cli-canon: §14 --help --json machine-readable help

## Description


Filed by the `stack-cli-alignment` CLI-surface normalisation (homebase epic), phase 1.
Source: homebase `issues/cli-alignment-audit/analysis.md` (2026-08-10 audit) + live
re-verification 2026-08-16. Canon: `AGENTS-AI-FIRST-CLI.md`. This is a **fix** issue
(the audit + review only recommend); laned in `cli-canon` for a future `/stint-start`.

**Gap (§14) — `--help --json` not implemented.**

Text `--help` is fine (clap gives drill-down for free), but there is no machine-readable
help payload. Family-wide near-gap.

**Do:** support `--help --json` emitting a structured help document — subcommands, flags,
`examples[]`, and env-var mappings — so an agent can discover the surface without scraping
prose.

**Current state (evidence):** text help only; no `--help --json` payload.
