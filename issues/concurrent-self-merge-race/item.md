---
created: 2026-08-01
updated: 2026-08-01
type: bug
reporter: jari
status: fixed
priority: normal
closed: 2026-08-01
---

# concurrent spinoff self-merges race on target worktree cleanliness

## Description

Three parallel spinoffs (independent kind: spinoff runs) attempted to self-merge into main via `orchestratectl run merge <id>` within seconds of each other. Two merged cleanly; the third failed with `Self-merge blocked by uncommitted changes in the target worktree`. The main worktree had no uncommitted changes from the user's perspective — the third merge apparently saw transient state from another merge in progress. Expected: concurrent self-merges either serialize cleanly (queue) or produce a clearer error indicating another merge is holding the target. Observed at commit ab37a05 (0.1.0), macOS/darwin. Reproduction: fire three `orchestratectl run create --kind spinoff` runs with the same source branch, each with a prompt that ends in `orchestratectl run merge <id>`. Race probability increases with number of concurrent merges and how close in time they land.
