---
created: 2026-08-15
updated: 2026-08-15
type: bug
status: open
priority: normal
epic: lifecycle-architecture-review
---

# Detached-HEAD / no-branch worktree can lose committed work on non-merge teardown

## Description

Follow-up from /llm-review of non-merge-teardown-dirty-worktree (openai). cleanup_node's committed-work protection on the non-merge path is source-relative against manifest.source_branch AND uses n.branch as the ref; the git branch -d backstop also relies on a named branch. If a worktree is on a DETACHED HEAD (or n.branch is None / stale) with commits not reachable from source, and the tree is clean: the dirty-worktree guard passes, non-force git worktree remove succeeds (clean tree), and there is no branch to delete -> the detached commits become unreachable and can be pruned = data loss. Agents normally work on named wt/* branches so this is an edge, but it is real and pre-existing. Also: when manifest.source_branch is unrecorded, the source-relative committed-work check is skipped entirely; only the ambient-HEAD-relative -d backstop protects a named branch. Proper fix needs HEAD inspection: resolve the worktree's actual HEAD oid, compare source..HEAD (not just source..recorded-branch), reject a branch/metadata mismatch, and preserve when HEAD or source cannot be verified. Out of scope for the localized uncommitted-work fix.
