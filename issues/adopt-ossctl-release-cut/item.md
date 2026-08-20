---
created: 2026-08-18
updated: 2026-08-20
type: task
reporter: jari
status: done
priority: normal
lane: release
lane_seq: 10
closed: 2026-08-20
commits:
- hash: 481d75b064dfff1f498d9e07c4028d26b4694a94
  summary: adopt resumable ossctl release cuts
---

# Adopt ossctl for cutting this project's releases

## Why

Releases here are still cut by hand: every cut needs a dedicated spinoff whose whole job is
to finalize the CHANGELOG, bump the workspace version, rewrite the `octl-core` pin, refresh
`Cargo.lock`, and regenerate the `version_*` snapshots. Two such spinoffs ran on 2026-08-18
alone (v0.4.0, v0.4.1).

`ossctl release plan --bump <level>` owns exactly that phase (version + intra-workspace pin
rewrites + `Cargo.lock` refresh + CHANGELOG finalize + any declared `bump_hook`), and ossctl
supports this multi-crate workspace. Adopting it removes a per-release spinoff from every
round.

## What to do

1. Wire `ossctl release plan --bump` into the release flow for the two-crate workspace
   (`octl-core` first, then `orchestratectl` with its `=<version>` pin).
2. **Settle the division of labour with the tag-triggered pipeline.** This repo publishes
   from CI on a `vX.Y.Z` tag (`publish-crates.yml`), and a local `cargo publish` is
   forbidden. ossctl must own the bump/changelog/tag phases without duplicating or bypassing
   the CI publish. Write the resulting flow into `AGENTS.md`'s release bullets, replacing the
   hand-cut sequence.
3. Wire the insta `version_*` snapshot regeneration through ossctl's `bump_hook`.
4. Adopt `ossctl release verify <run-id>` for post-publish verification, replacing any
   hand-rolled registry probe.

## Acceptance

The next release is cut through ossctl, and `AGENTS.md` describes that flow rather than the
manual one. crates.io publishes are permanent (yank-only), so the first ossctl-driven cut is
verified at least as carefully as a hand cut.

## Resolution

### 2026-08-20T09:59:02Z · @issuectl

Wiring validated with ossctl 0.9 non-mutating plan, simulated bump hook, two-round LLM review/assessment, and the exact green gate.
