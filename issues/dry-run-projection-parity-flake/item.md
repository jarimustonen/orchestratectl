---
created: 2026-08-10
updated: 2026-08-10
type: bug
reporter: jari
status: fixed
priority: normal
commits:
- hash: 50dc4f6
  summary: de-flake via (inode,mtime,size) fingerprint — inode-reuse under CI parallelism was the mechanism; test-detection flake not reducer bug; 100/100 green at 16 threads
closed: 2026-08-10
---

# Flaky: dry_run_projections_match_real_apply_writes fails only under CI parallelism (event.rs)

_Source: crates/octl-cli/tests/event.rs_

## Description

dry_run_projections_match_real_apply_writes (event.rs:963) failed on test (ubuntu-latest) for d43d984 — left (dry-run) = [discussions/d-parityaaaa.json, manifest.json], right (real apply) = [discussions/d-parityaaaa.json]. Passes 8/8 locally (isolation AND full event binary, release mode) and was GREEN on 137d5d9 (same test) — so it is an order/parallelism-dependent test-isolation flake, not a regression. Same category as notify-test-toctou-flake and the watchdog snapshot flake. Likely shared temp/run-dir state or a projection-set collection observing cross-test files under CI's higher parallelism. Fix: isolate the run dir / projection scan so the dry-run-vs-real-apply file set is deterministic regardless of sibling tests. Not release-blocking (all substantive CI jobs green; cleared on rerun).
