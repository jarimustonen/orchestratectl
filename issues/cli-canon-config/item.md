---
created: 2026-08-16
updated: 2026-08-16
type: improvement
status: done
priority: normal
labels: [cli-canon, tooling]
lane: cli-canon
lane_seq: 10
closed: 2026-08-16
closed_by: claude
---

# cli-canon: §8 config path / config show --json

## Description


Filed by the `stack-cli-alignment` CLI-surface normalisation (homebase epic), phase 1.
Source: homebase `issues/cli-alignment-audit/analysis.md` (2026-08-10 audit) + live
re-verification 2026-08-16. Canon: `AGENTS-AI-FIRST-CLI.md`. This is a **fix** issue
(the audit + review only recommend); laned in `cli-canon` for a future `/stint-start`.

**Gap (§8) — no `config path` / `config show --json`.**

An agent cannot ask "where does the effective config live" or "why is this value what it
is". This is the family's single most consistent miss (7/7 tools ✗ in the audit).

**Do:** add a `config` subcommand — `config path` (print the effective config file path)
and `config show --json` (effective config values + their source/provenance). Non-mutating,
`--json` envelope like the rest of the surface.

**Current state (evidence):** `orchestratectl config` → unrecognized subcommand.

## Comments

### 2026-08-16T17:26:55Z · @claude

Closed at stint-3 Phase 1 without code: live re-verification shows the §8 gap is already closed by v0.2.0. `orchestratectl config path` and `config show` both emit the canon envelope, and `config show` carries per-key provenance (`source: file`). The issue body's 'unrecognized subcommand' evidence predates the stint-1 config-subcommand landing.
