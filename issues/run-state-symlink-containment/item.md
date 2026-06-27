---
created: 2026-06-27
updated: 2026-06-28
type: improvement
status: in-progress
priority: normal
epic: orchestratectl-mvp
related: ['@core-path-traversal-id-validation']
---

# octl-core: symlink/TOCTOU containment for run state dirs

## Description

Spin-off from core-path-traversal-id-validation /llm-review (gpt-5.5, opus).

Id validation prevents traversal via id components, but RunPaths still follows filesystem symlinks: if an attacker can replace <run>/nodes (or discussions/spinoffs, or the run dir itself) with a symlink to /elsewhere, writes land outside the run dir. This is a different threat class (attacker-controlled filesystem vs attacker-supplied id) and was explicitly out of scope for the parent issue.

Decide the threat model and, if in scope, add containment: reject symlinked run subdirs via symlink_metadata before read/write, or O_NOFOLLOW / openat2(RESOLVE_BENEATH|RESOLVE_NO_SYMLINKS) on Linux. At minimum document the assumption that the state root is a trusted, per-user 0700 directory.
