---
created: 2026-08-20
updated: 2026-08-20
type: bug
reporter: pi
status: fixed
priority: high
lane: release
collision: [scripts/ossctl-release.sh]
closed: 2026-08-20
commits:
- hash: '8735041'
  summary: accept proven ossctl 0.9 held-tag journal
---

# Release wrapper rejects held tag checkpoint

## Description

## Description

The ossctl release wrapper correctly blocked the first real v0.5.0 tag push with its temporary pre-push hook, but then rejected the expected checkpoint because its state assertion requires `.data.state.current_phase == "tag"`. ossctl 0.9.0 records the failed tag phase as `phases[tag].outcome = "failed"` and clears `current_phase` to null.

Release journal `01M0FG88NAKBJ7Y3QNFZEHRM4K` completed bump, dry-run, build, and CI delegation for all four targets. It created local tag v0.5.0 at the journalled bump commit, the safety hook rejected the push, remote tag absence was verified, and no target was published. The wrapper then exited 2 at `cut did not stop at the expected pre-push checkpoint`. The journal was abandoned and the unpublished local tag deleted.

## Reproduction

With ossctl 0.9.0, the held-tag state is:

```json
{
  "current_phase": null,
  "phases": [{"phase":"tag","outcome":"failed"}],
  "tags": {"v0.5.0":{"created_local":true,"pushed_remote":false}}
}
```

The wrapper currently requires `current_phase == "tag"`, so it cannot reach `advance_main_to_bump` or the exact-SHA CI gate even though the pre-push hold worked exactly as intended.

## Acceptance Criteria

- Recognize the actual ossctl 0.9.0 journal shape for an intentionally held tag without accepting unrelated tag failures.
- Require positive evidence from the hook marker, expected remote identity, absent remote tag, exact local tag/bump coordinates, `tag created_local=true`, `pushed_remote=false`, and the expected tag-phase outcome.
- Remain fail-closed for missing marker, wrong remote, unexpected phase/event history, absent/moved local tag, or any remote tag.
- Add an end-to-end stubbed release journal regression matching the real `current_phase:null` state.
- Run the full green gate. Do not create/push a real release tag or publish from the worker; a fresh plan is required.

## Resolution

### 2026-08-20T12:29:09Z · @issuectl

Recognize only the real null-current-phase tag failure when the hook marker, exact journal/event history, local tag and bump coordinates, canonical remote, and remote absence all agree. Added stripped-PATH end-to-end negative coverage.
