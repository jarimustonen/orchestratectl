---
created: 2026-07-25
updated: 2026-07-25
type: bug
status: open
priority: normal
---

# Second of two back-to-back 'run create' calls left supervisor-less (pid null, no worker node)

## Description

Observed during a /stint round (kunnollavauhtiin-monorepo, orchestratectl 0.1.0). Two 'orchestratectl run create --kind spinoff --headless ... --output jsonl' calls were issued back-to-back in one shell (create A; echo ===; create B).

Observed: create A printed its success envelope (supervisor pid set, alive). create B's envelope was NEVER printed to stdout, yet the run WAS created — it appeared in 'run list' with status pending, node_count 0, and supervisor {pid: null, alive: false}. 'run show B' confirmed no supervisor and no worker node. 'orchestratectl run reattach B' spawned the supervisor (new pid) and the worker node appeared; the run then proceeded normally to done+merged.

Expected: either 'run create' fully spawns the supervisor before returning (and prints its envelope), or it fails loudly. A silently supervisor-less pending run is confusing — it looks stuck and only 'run reattach' (non-obvious) recovers it.

Uncertain whether root cause is a race between rapid successive 'run create' calls or an artifact of backgrounding both in one shell; reattach recovery worked cleanly. Filing as low-severity friction with the exact repro. Env: macOS, headless session (detached 'headless' tmux).
