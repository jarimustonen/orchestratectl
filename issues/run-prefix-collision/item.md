---
created: 2026-08-17
updated: 2026-08-20
type: bug
status: fixed
priority: normal
lane: cli
closed: 2026-08-20
commits:
- hash: dda5e39
  summary: resolve worker runs by exact worktree ownership
---

# Run-ID prefix collisions can select the wrong owning run

## Description

## Problem

Two runs created concurrently can share the documented first 10 run-ID characters. In homebase stint 6, runs `01m08c08v5jxzfqf3r36n0sgzd` and `01m08c08v5422jae649kmwewy9` both produced branches beginning `wt/01m08c08v5-...`.

The bundled worktree merge/discovery recipe derives `short` from that branch prefix and runs:

```sh
ls -1 ~/.taskfleet/runs/ | grep -m1 "^${short}"
```

That selected the unrelated run. The worker avoided merging/reporting against the wrong run only by using exact node/branch lookup.

## Expected

Run discovery from inside a worktree must identify exactly one owning run even when concurrent ULIDs share the first ten characters. Prefer durable full-run metadata or an exact branch-to-node lookup rather than increasing a probabilistic prefix.

## Acceptance Criteria

- [x] The worker closing instructions and any helper implementation cannot select another run on prefix collision.
- [x] A regression test creates two runs with the same current short prefix and proves each worktree resolves its own full run ID.
- [x] Existing run-merge behavior remains compatible.

## Resolution

### 2026-08-20T08:17:37Z · @issuectl

Added run show --current exact canonical worktree/branch ownership resolution, replaced every bundled short-prefix closing recipe, and covered the observed colliding ULIDs with real linked-worktree tests.
