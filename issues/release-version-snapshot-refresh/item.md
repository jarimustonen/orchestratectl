---
created: 2026-08-14
updated: 2026-08-14
type: improvement
status: open
priority: normal
---

# Release mechanics: version bump must refresh version_* insta snapshots

## Description

During the v0.1.8 release, bumping the workspace version to `0.1.8` left the committed insta snapshots (`crates/octl-cli/tests/snapshots/envelope_snapshots__version_{text,json,jsonl}.snap`) pinned at `0.1.7`, so `version_envelopes` failed and turned `main` CI red *after* the release tag was already cut (release binaries were correct; only the snapshot fixture was stale). The local integrated gate had run *before* the bump and so didn't catch it. Fixed reactively in commit `0bf6a75`.

**Goal:** make this impossible to forget again. Two complementary options — implement the cheap one, and evaluate the guard:

1. **Documentation (do):** add an explicit "bump the workspace version → refresh the `version_*` snapshots (`cargo insta test --accept -p orchestratectl` or the sed equivalent) → re-run `cargo test --workspace`" step to the release mechanics, in whichever of `OSS-RELEASE.md` / `crates/octl-cli/CLAUDE.md` / the `/oss-release-cut` flow is the right home. Keep it consistent with how the CHANGELOG-finalize step is documented.
2. **Guard (evaluate, implement if cheap):** make the version snapshot resilient — e.g. an insta redaction/filter that normalizes the version string so the snapshot no longer encodes the literal version, OR a CI/pre-publish check that the `version_*` snapshots match `CARGO_PKG_VERSION`. Prefer removing the manual step over documenting it if the redaction is clean.

**Constraints:** touches release docs / CI / test config only — must NOT touch `skill.rs`, `supervise/*`, or `{harness,floor,pipeline}/*` (parallel worktrees own those this round). 

**Acceptance:** the release mechanics doc names the snapshot-refresh step; if the guard is implemented, deliberately bumping the version locally makes the test fail loudly (or auto-pass via redaction) as intended. Green gate.

