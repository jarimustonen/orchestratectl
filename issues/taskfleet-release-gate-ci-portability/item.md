---
created: 2026-09-06
updated: 2026-09-06
type: bug
status: open
priority: high
related: ['@taskfleet-release-0-6-0']
lane: taskfleet-rename
lane_seq: 111
collision: [scripts/verify-release-github-policy.sh, .github/workflows/release.yml, .github/workflows/publish-crates.yml]
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

## Required outcome

1. Keep v0.6.0 immutable and never retag or reuse it.
2. Make the authorization/policy verifier portable to jq 1.6 and prove that its live GitHub API reads work with the credential actually supplied to every tag workflow.
3. Preserve structural fail-closed behavior and secret non-reachability from pull requests.
4. Add tests that execute the real filter with jq 1.6 and distinguish API/shape failures clearly.
5. Validate through an exact candidate PR and merged-main CI without tagging or publishing.
6. Document v0.6.0 as an unpublished burned tag and prepare a new patch release (v0.6.1) only through a newly sealed wrapper plan.

## Definition of Done

- [ ] Both authorization paths pass with the exact runner jq/tool/token topology used by tag workflows.
- [ ] Missing, malformed, inaccessible, or mismatched rulesets still fail closed.
- [ ] PRs cannot access release credentials or execute publication.
- [ ] Full green gate and exact-SHA CI evidence are recorded.
- [ ] No v0.6.0 artifact/package/formula was published and no tag was moved.
- [ ] The fix is ready for a fresh v0.6.1 wrapper transaction.
