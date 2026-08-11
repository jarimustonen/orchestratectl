---
created: 2026-08-11
updated: 2026-08-11
type: bug
status: open
priority: high
---

# version_text snapshot stale on 0.1.5 — CI red on main

## Description

Failing job: `envelope_snapshots` (test `version_envelopes`) in the `CI` workflow, both ubuntu and macos, on every push to main (last green 2026-08-10 16:01; runs 31415861267, 31417120600 failed).

Root cause: the insta snapshot `crates/octl-cli/tests/snapshots/envelope_snapshots__version_text.snap` still asserts `orchestratectl 0.1.4`, but Cargo.toml was bumped to 0.1.5 for the v0.1.5 release cut (release was cut + published anyway, so the version is live while CI stays red).

Failing log lines:
```
-old snapshot
+new results
  1 │-orchestratectl 0.1.4
    1 │+orchestratectl 0.1.5
```
`snapshot assertion for 'version_text' failed in line 233` — panicked at insta-1.48.0 runtime.rs:719.

Fix: `cargo insta accept` in crates/octl-cli (update version_text to 0.1.5), commit, push. Follow-up: fold snapshot re-accept into the release-cut flow so a version bump can never leave main red.
