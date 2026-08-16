---
created: 2026-08-16
updated: 2026-08-16
type: improvement
status: in-progress
priority: normal
labels: [cli-canon, tooling]
lane: cli-canon
lane_seq: 20
---

# cli-canon: §10 version payload supported_schemas

## Description


Filed by the `stack-cli-alignment` CLI-surface normalisation (homebase epic), phase 1.
Source: homebase `issues/cli-alignment-audit/analysis.md` (2026-08-10 audit) + live
re-verification 2026-08-16. Canon: `AGENTS-AI-FIRST-CLI.md`. This is a **fix** issue
(the audit + review only recommend); laned in `cli-canon` for a future `/stint-start`.

**Gap (§10) — `version` payload missing `supported_schemas`.**

`version --output json` has `commit` + `skills[]` but no `supported_schemas`, so drift
detection is only partial.

**Do:** add `supported_schemas: [N,…]` to the `version` payload. (Minor: `version` should
also accept the global `--json`, not only `--output json`.)

**Current state (evidence):** `version --output json` has commit+skills[] but no supported_schemas; `version --json` errors (only --output).
