---
created: 2026-08-20
updated: 2026-08-20
type: bug
reporter: pi
status: open
priority: high
lane: release
collision: [scripts/ossctl-release.sh]
---

# Release wrapper uses unsupported gh repo shorthand

## Description

## Description

The newly adopted `scripts/ossctl-release.sh cut <plan-id>` fails in its repository preflight before creating a release journal or mutating the repository because it invokes `gh repo view -R ...`. The installed GitHub CLI's `repo view` command does not support the `-R` shorthand.

## Reproduction

From a clean `main` with a valid sealed v0.5.0 plan:

```text
scripts/ossctl-release.sh cut <plan-id>
unknown shorthand flag: 'R' in -R
Usage: gh repo view [<repository>] [flags]
```

The process exits 1 in about one second. No bump commit, tag, or publish occurs.

## Acceptance Criteria

- Repository identity/preflight works with the supported `gh repo view` interface on the installed GitHub CLI and does not depend on an unsupported shorthand.
- Explicit repository targeting remains correct and neutral; do not accidentally query whichever repository happens to be ambient.
- Add deterministic wrapper coverage using a stubbed `gh` that matches the supported argument contract and rejects the old `-R` form.
- The full release-wrapper validation and repository green gate pass.
- Re-seal the release plan after the fix; the old content-addressed plan must not be reused against a changed HEAD.
