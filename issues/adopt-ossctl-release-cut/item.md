---
created: 2026-08-18
updated: 2026-08-18
type: task
reporter: jari
status: open
priority: normal
---

# Re-evaluate ossctl-driven release cutting now that the multi-crate blocker is done

## Description

## Why now

`AGENTS.md` has said since the 0.1.x era that ossctl "can NOT yet cut this project's
releases (multi-crate workspace unsupported)", pointing at `release-rust-workspace-multicrate`
in `~/Sources/ossctl`. **That blocker's status is now `done`** (observed 2026-08-18), so the
standing claim in our own operating policy is stale and we have been hand-cutting releases
on an assumption that may no longer hold.

Two releases were cut by hand on 2026-08-18 alone (v0.4.0, v0.4.1), each requiring a
dedicated spinoff whose entire job was: finalize the CHANGELOG, bump the workspace version,
rewrite the `octl-core` pin, refresh `Cargo.lock`, regenerate the `version_*` snapshots.

`ossctl release plan --bump <level>` advertises **exactly** that set: "compute the new version
from the current manifest version + this semantic level and seal a bump phase (version +
intra-workspace pin rewrites + Cargo.lock refresh + CHANGELOG finalize + any declared
`bump_hook`)". If that works here, a per-release spinoff disappears from every round.

## What to determine

1. Can `ossctl release plan` / `cut` actually handle this two-crate workspace end-to-end
   (`octl-core` first, then `orchestratectl` with its `=<version>` pin)? Verify against the
   real repo, read-only first.
2. How does it interact with our **tag-triggered** pipeline? Our releases publish from CI on
   a `vX.Y.Z` tag (`publish-crates.yml`), and `AGENTS.md` forbids a local `cargo publish`.
   An ossctl cut must not duplicate or bypass that: determine whether ossctl can own the
   bump+changelog+tag phases while CI keeps owning the publish, and write down the resulting
   division of labour.
3. Does the insta `version_*` snapshot regeneration fit ossctl's `bump_hook`? If yes, that is
   the mechanism to wire it through.
4. `ossctl release verify <run-id>` reconciles against registry state — adopting it removes
   the hand-rolled crates.io probe (and with it the `User-Agent` trap now documented in
   `AGENTS.md`).

## Acceptance

Either (a) ossctl is adopted for the bump/changelog/verify phases, `AGENTS.md`'s release
bullets are rewritten to describe the new flow, and the next release is cut that way; or
(b) a concrete, recorded reason it still cannot, with the specific gap filed upstream in
`~/Sources/ossctl` and `AGENTS.md`'s claim corrected to state the *real* current limitation
rather than the stale one.

Do not adopt it blindly: crates.io publishes are permanent (yank-only), so the first
ossctl-driven cut must be verified at least as carefully as a hand cut.
