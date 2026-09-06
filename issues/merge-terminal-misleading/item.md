---
created: 2026-08-01
updated: 2026-08-03
type: bug
reporter: jari
status: fixed
priority: normal
closed: 2026-08-03
---

# run merge on terminal run fails with misleading merge_spawn_failed

## Description

Calling `taskfleet run merge <id>` on a run that has already reached a terminal state (e.g. auto-completed via a prior self-merge, or cancelled) fails with `merge_spawn_failed: invoke merge.sh (/var/folders/.../taskfleet-merge-XXXX.sh): No such file or directory (os error 2)`. The error message points at a missing temp script, but the real cause is that the run is already terminal and no merge should be attempted. Expected: a clear error code like `run_already_terminal` or `already_merged` with a message explaining that no merge action is needed. Observed at commit ab37a05 (0.1.0). Reproduction: create a spinoff, let it self-merge via `taskfleet run merge`, then invoke `taskfleet run merge <same-run-id>` again.
