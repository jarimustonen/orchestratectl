---
created: 2026-08-10
updated: 2026-08-10
type: bug
reporter: claude-code
status: fixed
priority: high
commits:
- hash: 2beaec7
  summary: repair intra-doc links + time bump for RUSTSEC-2026-0009; accept help snapshot
closed: 2026-08-10
---

# CI red on main: cargo-deny (RUSTSEC-2026-0009), broken intra-doc links, macos doctests

## Description

## Summary

CI on `main` has been **red for several days** (GitHub emailed "Run failed: CI - main" for commits `df176f4` 2026-08-06, `355f064` 2026-08-08, `732b6d5` 2026-08-09, and the latest run 2026-08-10 is still failing). Three jobs fail; `rustfmt`, `clippy`, `test (ubuntu-latest)` and `msrv` pass.

Reference run: <https://github.com/jarimustonen/taskfleet/actions/runs/31356764835> (2026-08-10).

## Failing jobs & root causes

### 1. `cargo-deny` (advisories) — dependency vulnerability
```
error[vulnerability]: Denial of Service via Stack Exhaustion
  ID: RUSTSEC-2026-0009
  Advisory: https://rustsec.org/advisories/RUSTSEC-2026-0009
  A limit to the depth of recursion was added in v0.3.47. From this version, an error will be returned...
advisories FAILED, bans ok, licenses ok, sources ok
```
A transitive dependency is flagged by RUSTSEC-2026-0009 (fixed in the crate's v0.3.47+). Fix: bump the offending dependency (`cargo update` to pull ≥0.3.47), or, if no fix path is available yet, add a time-boxed advisory ignore in `deny.toml` with a tracking note.

### 2. `docs` (cargo doc, deny broken intra-doc links) — doc-link errors
`taskfleet-core` fails to document; representative errors:
```
error: public documentation for `plan` links to private item `tolerated_fields`
error: public documentation for `TOLERATED_OPTIONAL_FIELDS` links to private item `tolerated_fields`
error: public documentation for `TOLERATED_OPTIONAL_FIELDS` links to private item `ObjectShape`
error: public documentation for `UnsafeCwd` links to private item `is_safe_repo_relative`
error: could not document `taskfleet-core`
error: unresolved link to `Opus`
error: unresolved link to `TestId`
error: unresolved link to `ChunkOutcome::Committed` / `NoChange` / `Failed`
error: unresolved link to `CodeHarness` / `run_and_check` / `live_enabled` / `check_preconditions`
```
Fix: repair the intra-doc links — either make the linked items public, or drop the `[...]` link and use plain code spans, or fully-qualify the paths. Two classes: (a) public docs linking to private items, (b) unresolved links to nonexistent/renamed paths.

### 3. `test (macos-latest)` — job fails though unit tests pass
The unit suite passes (`test result: ok. 213 passed; 0 failed`). The macos job's failure is almost certainly **doc-tests** failing on the same broken intra-doc links as job 2 (macOS runs the full `cargo test` incl. doctests). Fixing job 2 likely turns this green; re-verify after.

## Suggested order
1. Fix the intra-doc links (clears `docs` and, very likely, `test (macos-latest)`).
2. Resolve RUSTSEC-2026-0009 (dependency bump, or documented `deny.toml` ignore).
3. Confirm a green CI run on `main`.

Filed from the GitHub "Run failed" email notifications during a mail sweep.
