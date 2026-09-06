---
created: 2026-08-20
updated: 2026-08-20
type: bug
reporter: pi
status: fixed
priority: high
lane: release
collision: [scripts/ossctl-bump-hook.sh]
closed: 2026-08-20
commits:
- hash: d242b18
  summary: make release snapshot bump hook succeed
---

# Bump hook fails after updating version snapshots

## Description

## Description

The first real ossctl-driven v0.5.0 cut reached the engine bump phase in a clean checkout, updated all three expected version snapshots, and then failed because `scripts/ossctl-bump-hook.sh` returned exit 1. The hook had only been validated through simulation before adoption; the real engine path exposed the defect.

Journal `01M0FD8FSTMGYG8YTV92WMWC87` was abandoned. It contains only `run_created`, `phase_entered:bump`, and `phase_completed:bump outcome=failed`; `bump` is null, published/delegated/tag state is empty, no local `v0.5.0` tag exists, and nothing was published.

## Reproduction

A clean, sealed minor-bump cut runs:

```text
INSTA_UPDATE=always cargo test --locked -p taskfleet --test envelope_snapshots
updated snapshot ...version_text.snap
updated snapshot ...version_json.snap
updated snapshot ...version_jsonl.snap
```

The hook then exits 1 and ossctl reports `bump_hook failed (1)`. The error output ends immediately after the three update messages, so determine whether the insta invocation itself returns non-zero on changed snapshots or a following hook check fails without surfacing its own diagnostic.

## Acceptance Criteria

- Reproduce the hook in an actual clean temporary checkout whose manifests/pin/lock/changelog are bumped from 0.4.1 to 0.5.0, not only against unchanged snapshots.
- The hook exits 0 when exactly the three expected version snapshots change and all snapshot contents pass `check-version-snapshots.sh`.
- It still exits non-zero for `.snap.new`, unrelated tracked/untracked changes, a missing expected snapshot, or a version snapshot that was not regenerated.
- Add deterministic regression coverage for the changed-snapshot success path and failure guards.
- The full repository green gate passes.
- Do not create/push a real tag or publish anything from the fix worker. A fresh content-addressed release plan is required after landing.

## Resolution

### 2026-08-20T11:43:29Z · @issuectl

Fixed the silent successful-regeneration exit status, retained fail-closed guards, and added clean-checkout regression coverage.
