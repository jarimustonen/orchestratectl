---
created: 2026-08-11
updated: 2026-08-11
type: bug
status: open
priority: normal
---

# version_envelopes snapshot stale after bump to 0.1.5

## Description

## What

CI on main is red. Snapshot test `version_envelopes` fails because the version was bumped to 0.1.5 but the insta snapshot still records 0.1.4.

## Failing log

```
test version_envelopes ... FAILED
Snapshot: version_text
Source: crates/octl-cli/tests/envelope_snapshots.rs:233
-old snapshot
+new results
- orchestratectl 0.1.4
+ orchestratectl 0.1.5
```

## Root cause

Release v0.1.5 was cut and published, but the insta snapshot at `crates/octl-cli/tests/snapshots/envelope_snapshots__version_text.snap` was not re-recorded.

## Fix

Run `cargo insta review` (or `cargo test -- --accept`) to accept the new snapshot and commit the updated snap file.

## Ref

- Run: https://github.com/jarimustonen/orchestratectl/actions/runs/31456616587
