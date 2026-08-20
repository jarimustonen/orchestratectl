---
created: 2026-08-20
updated: 2026-08-20
type: bug
reporter: pi
status: fixed
priority: high
lane: release
collision: [scripts/ossctl-release.sh]
assignee: pi
closed: 2026-08-20
commits:
- hash: e1868ae
  summary: use supported positional gh repo view targeting with fail-closed stub coverage
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

## Decisions

### 2026-08-20T10:51:07Z · @pi

The previously sealed release plan is content-addressed to the pre-fix HEAD and must not be reused. The orchestrator must run the non-mutating wrapper plan command again from updated main before any later release cut.
