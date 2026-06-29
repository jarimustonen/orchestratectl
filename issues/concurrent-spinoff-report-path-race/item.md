---
created: 2026-06-29
updated: 2026-06-29
type: improvement
status: fixed
priority: normal
closed: 2026-06-29
---

# Concurrent spinoffs race on shared /tmp/node-report.json

## Description

The bundled `worktree-spinoff` and related SKILLs instruct the agent to write its terminal payload to `/tmp/node-report.json` before calling `orchestratectl run merge --report-file /tmp/node-report.json`. When two spinoffs run concurrently on the same machine they CLOBBER each other's report file — one of them merges with the wrong payload (different summary, wrong spinoff_proposals, etc.). Observed 2026-06-29 during the B-fix parallel batch when a sibling spinoff happened to write `/tmp/node-report.json` between this one's write and read.

Fix options:

1. **Run-unique path** — recommend `mktemp -t spinoff-report-XXXXXX.json` in every SKILL's "Closing (mandatory)" snippet. Cheap, AI-shaping only.
2. **Run-id path** — recommend `/tmp/node-report-${run_id}.json` so the path is deterministic per run and inspectable.
3. **stdin-based merge** — extend `orchestratectl run merge --report -` to read the payload from stdin so no temp file is needed.

Prefer (3) as a long-term API, ship (2) as the SKILL-side recommendation immediately.

Affects: `worktree-spinoff`, `worktree-research`, `worktree-bugfix`, `worktree-technical-decision`, `worktree-make-skill`, `worktree-orchestrated`, plus any future autonomous-merge SKILL.
