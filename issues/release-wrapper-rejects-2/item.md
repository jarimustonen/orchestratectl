---
created: 2026-08-21
updated: 2026-08-21
type: bug
reporter: jari
status: in-progress
priority: high
related: ['@release-wrapper-rejects', '@adopt-ossctl-release-cut']
lane: release
lane_seq: 10
collision: [scripts/ossctl-release.sh]
---

# Release wrapper rejects ossctl 0.10 until pre-tag protocol is revalidated

_Source: scripts/ossctl-release.sh_

## Description

The autonomous v0.5.0 recovery reached the repository's release wrapper after the full integrated green gate, push, commit-verified local deploy, skill install, and doctor (1131 ok / 0 warn / 0 fail). The first fresh planning command failed closed before creating a plan:

```text
$ scripts/ossctl-release.sh plan minor
ossctl 0.9.x required; found 0.10.0 (revalidate the pre-tag protocol before widening this range)
```

Installed ossctl is 0.10.0, commit `a35b9917fc65a6354fe855b7c956521b47669907`, at `~/.cargo/bin/ossctl`. There are no in-flight release journals and no v0.5.0 remote tag/publication. The two prior v0.5.0 journals remain abandoned and must not be resumed.

This is the wrapper's intended fail-closed behavior, not permission to change the version predicate mechanically. Revalidate ossctl 0.10's full release protocol and journal/read surfaces before admitting it.

## Scope

- Compare ossctl 0.10's `release plan/cut/show/list/resume/verify` JSON and phase/tag semantics with every assumption in `scripts/ossctl-release.sh`.
- Verify that the pre-push safety hook still holds the local release tag before any irreversible remote publication and produces evidence the wrapper can validate.
- Verify bump commit/tag coordinates, held-journal classification, checkpoint persistence, exact-main-SHA CI gate, resume behavior, and registry verification.
- Update the accepted version predicate and protocol assertions only where 0.10 evidence proves compatibility; preserve or strengthen fail-closed checks for unknown versions/states.
- Update wrapper comments, tests, and repository release policy documentation if the supported protocol changes.
- Do not run a real release cut, push a release tag, publish, install/uninstall global tools, or resume either abandoned v0.5.0 journal from the worker.

## Acceptance criteria

- `scripts/ossctl-release.sh plan minor` can run with ossctl 0.10.0 and emits a fresh sealed plan only after all existing contract/readiness/clean-tree checks pass.
- The wrapper rejects unsupported future versions and every held-tag near-miss that is not proven safe.
- End-to-end wrapper tests exercise the real ossctl 0.10 journal shape through the held pre-tag checkpoint without pushing a real remote release tag.
- The 0.10 release protocol assumptions are documented and reviewed, not inferred from a version number alone.
- Full repository green gate and `/llm-review` + `/assess-findings` pass before merge.

## Reproduction

1. Install ossctl 0.10.0.
2. On clean synchronized `main`, run `scripts/ossctl-release.sh plan minor`.
3. Observe the version-gate rejection before plan creation.

## Quick Test

Run the wrapper's protocol test suite against ossctl 0.10.0, then run a non-publishing `scripts/ossctl-release.sh plan minor` from a clean fixture/repository state. The production orchestrator will create and cut the real fresh plan only after this issue lands and the integrated gate is rerun.
