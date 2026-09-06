---
created: 2026-09-06
updated: 2026-09-06
type: bug
status: fixed
priority: high
related: ['@taskfleet-release-0-6-0']
lane: taskfleet-rename
lane_seq: 111
collision: [scripts/verify-release-github-policy.sh, .github/workflows/release.yml, .github/workflows/publish-crates.yml]
closed: 2026-09-06
closed_by: pi
commits:
- hash: 8ad34689a4a23e3d354b37aafcf5099c0b6c448c
  summary: fix release gate credentials and diagnostics
- hash: a8ab5d0e3475f83dbf116cbe71300316308b195f
  summary: pin jq 1.6 fixture and pass exact candidate CI
---

# Release gate fails on CI jq and workflow token

## Summary

The public v0.6.0 tag was authorized and pushed by the pinned release wrapper at commit `57f6dfb83401694399b363de5d3aa88e4541a22c`, after exact-main CI `34016341659` passed. Both tag-triggered publication workflows then failed closed in `scripts/verify-release-github-policy.sh` before any package, asset, release, or formula publication.

## Observed failures

- GitHub-hosted Linux runners use jq 1.6, where `--arg include ...` collides with jq's `include` keyword. The filter fails to compile at `.conditions.ref_name == {exclude:[],include:[$include]}`.
- The self-hosted macOS runner uses jq 1.8.2 and parses the filter, but the workflow-scoped `GITHUB_TOKEN` does not produce the same live ruleset proof as the maintainer token; the policy verifier exits fail-closed.
- crates workflow: `34016740702`, failed gate job `101441707888`.
- cargo-dist workflow: `34016740704`, failed build jobs `101441745244`, `101441745248`, and `101441745351`.
- Shipshape journal: `01M1TNW3SMN0XA347D1MG4518R`.

## Root cause evidence

- The Linux failure is a jq parser incompatibility: jq 1.6 treats `include` as a reserved module keyword, so the filter variable `$include` never compiles. The production filter now uses `$ref_pattern` and is exercised unchanged in an Ubuntu 22.04 container reporting `jq-1.6`.
- `GET /repos/jarimustonen/taskfleet/rulesets/22234415` is publicly readable, but GitHub omits the privileged `bypass_actors` field from non-administrator responses. That is the shape returned with the workflow `GITHUB_TOKEN`; it is why jq 1.8 parsed the filter on the self-hosted macOS runner and then returned false without an API error.
- GitHub's ruleset endpoint requires repository **Administration: read** to return bypass actors. `GITHUB_TOKEN` has no grantable Administration permission, so changing workflow `contents` permissions cannot fix the redaction.
- A sanitized 2026-09-06 read using the Homebase SOPS-managed `HOMEBREW_TAP_TOKEN` returned HTTP 200 and exposed the required bypass-actor array for ruleset `22234415`; the token value was passed only through process environment and was neither printed nor persisted. See `credential-ruleset-read.json`.
- The generated cargo-dist workflow is tag-only and scopes that credential to its authorization step. The crates workflow scopes it to a dedicated `release-authorization` job guarded by `github.event_name == 'push'`; `publish-core` directly needs that job. Manual package inspection receives no credential, and neither publication workflow has a pull-request path to it.

## Required outcome

1. Keep v0.6.0 immutable and never retag or reuse it.
2. Make the authorization/policy verifier portable to jq 1.6 and prove that its live GitHub API reads work with the credential actually supplied to every tag workflow.
3. Preserve structural fail-closed behavior and secret non-reachability from pull requests.
4. Add tests that execute the real filter with jq 1.6 and distinguish API/shape failures clearly.
5. Validate through an exact candidate PR and merged-main CI without tagging or publishing.
6. Document v0.6.0 as an unpublished burned tag and prepare a new patch release (v0.6.1) only through a newly sealed wrapper plan.

## Acceptance Criteria

- [x] Both authorization paths pass with the exact runner jq/tool/token topology used by tag workflows.
- [x] Missing, malformed, inaccessible, or mismatched rulesets still fail closed.
- [x] PRs cannot access release credentials or execute publication.
- [x] Full green gate and exact-SHA CI evidence are recorded.
- [x] No v0.6.0 artifact/package/formula was published and no tag was moved.
- [x] The fix is ready for a fresh v0.6.1 wrapper transaction.

## Validation

- Exact candidate: `a8ab5d0e3475f83dbf116cbe71300316308b195f`.
- Same-repository PR: [#3](https://github.com/jarimustonen/taskfleet/pull/3).
- Exact-SHA CI: [34018842931](https://github.com/jarimustonen/taskfleet/actions/runs/34018842931), green across hosted Linux/macOS, self-hosted ARM64 macOS, jq 1.6 release topology, MSRV, clippy, docs, deny, snapshots, and tests.
- The jq fixture passed against checksum-pinned `jq-1.6` (`af986793…a124c44`) and local jq 1.8.2. API failure, malformed shape, privilege-redacted shape, and policy mismatch fixtures each produced a distinct non-secret diagnostic and failed closed.
- Homebase's SOPS-managed release credential read both live rulesets with HTTP 200 and exposed `bypass_actors`; `credential-ruleset-read.json` contains only sanitized metadata.
- cargo-dist 0.28.2 `generate --check`, exact plan/topology validation, the pinned Homebrew 6.0.21 disposable distribution drill, actionlint structure, all release authorization/wrapper/publish fixtures, and the exact Shipshape 0.10.1 migration protocol passed.
- Full local gate passed: fmt, clippy warnings-as-errors, release nextest, doctests, and rustdoc warnings-as-errors. The all-workspace release nextest suite also passed with the stripped declared PATH.
- Postflight: all three crates remain without v0.6.0; no v0.6.0 GitHub Release exists; canonical and old tap heads remain `db12bb163e47617f0b941a35d3896b6ba0548892` and `85ce830378f38cf17283efddd966d5754354e403` respectively.
- No tag, authorization ref, ruleset, tap, registry, installation, or release journal was mutated by this fix.

## Resolution

### 2026-09-06T07:26:15Z · @pi

jq 1.6 portability, administration-readable tag-gate credentials, fail-closed diagnostics, exact cargo-dist generation, and candidate CI 34018842931 are verified; v0.6.0 remains unpublished and immutable.
