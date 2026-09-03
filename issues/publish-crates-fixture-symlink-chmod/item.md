---
created: 2026-09-02
updated: 2026-09-03
type: bug
status: fixed
priority: normal
provenance: other
provenance_detail: Observed during ADR 0002 R8 exact-SHA integrated validation
source_ref: orchestratectl:01m1hw78dn6qee8an2ysbfdns9/validation-failure:publish-crates-fixture-symlink-chmod
originating_run: 01m1hw78dn6qee8an2ysbfdns9
originating_run_kind: spinoff
lane: taskfleet-rename
lane_seq: 85
related: ['@rename-taskfleet']
collision: [repository-identity]
closed: 2026-09-03
commits:
- hash: 829c842
  summary: avoid chmod on fixture tool symlinks
---

# publish-crates fixture chmods symlinked system tools on Linux CI

## Description

## Observed occurrence

Main CI run `33678068490` for exact commit `fa04841ad74c0ea935cc8c81a83a90a917678853` failed in job `100407635030` (`version-snapshots`) while running `./scripts/test-publish-crates.sh`.

The test creates `$tmp/bin/*` entries as symlinks to host tools and then runs `chmod +x "$tmp/bin/"*`. On GitHub's Linux runner, `chmod` dereferences those symlinks and attempts to change permissions on system executables such as `/usr/bin/awk`, `/usr/bin/bash`, and `/usr/bin/jq`. The runner rejects that with `Operation not permitted`, causing exit 1 before the registry protocol assertions run. The same script passes on macOS because the host permission behavior differs.

Evidence: https://github.com/jarimustonen/orchestratectl/actions/runs/33678068490/job/100407635030

## Impact

`main` is red on the exact integrated R7 commit, so ADR 0002 R8 cannot authorize the GitHub repository rename. The release protocol fixture is not portable to the Linux CI environment it is intended to gate.

## Likely correction

Make only generated stub files executable, rather than applying `chmod` to the symlinked prerequisite tools. Re-run exact-SHA main CI and then restart R8 integrated evidence from the corrected immutable commit.

## Resolution

### 2026-09-03T10:12:58Z · @issuectl

Verified the publish fixture in normal and stripped-PATH environments. The full fmt, clippy, nextest, doctest, and rustdoc gate passed after one retry of an unrelated timing-sensitive materialization test.
