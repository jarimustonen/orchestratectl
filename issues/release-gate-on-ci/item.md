---
created: 2026-08-17
updated: 2026-08-17
type: chore
status: open
priority: high
lane: skills
lane_seq: 30
---

# CI must gate the release, not run alongside it

## Description

Two failures with one root cause, both observed in stint 4 (2026-08-17): **the local green
gate runs on macOS, but a whole class of defect only manifests on Linux — and the release
was published before CI had reported on the commit it contained.**

## Observed

`tmux-stub-etxtbsy-flake` is an ETXTBSY (`ExecutableFileBusy`) failure: Linux refuses to
exec a file while any process holds a write descriptor to it. macOS does not enforce this
at all. So:

1. A fix for it was written and its worker's full local gate passed (`fmt`, `clippy`,
   `cargo test --workspace`, `doc`). The orchestrator's integrated gate on merged `main`
   also passed — 26 suites, 0 failures.
2. On that evidence the round proceeded to publish **v0.2.2** to crates.io and push the
   `v0.2.2` tag.
3. CI on the same commit then went **red**: the fix was incomplete, and two sibling tests
   still failed with ETXTBSY on `test (ubuntu-latest)`.

The release was not harmed — the defect is test-only and the shipped binary was unaffected
— but that was luck, not process. crates.io publishes are **permanent (yank-only)**, so the
ordering must not depend on luck.

## Root cause

The green gate documented in `AGENTS.md` is defined entirely as local commands. It is
authoritative for portable defects and **silently blind** to platform-specific ones. Nothing
in the release sequence consults CI, which is the only signal that covers Linux, the MSRV
floor, and `cargo-deny`.

## The local publish was redundant in the first place

Investigated during the same handoff: **`.github/workflows/publish-crates.yml` already
publishes both crates to crates.io from CI**, tag-triggered on `v[0-9]+.[0-9]+.[0-9]+*`, in
dependency order, using the repo's `CARGO_REGISTRY_TOKEN` secret. The local `cargo publish`
duplicated it. The CI job reported green only because it deliberately tolerates *"already
exists on the crates.io index"* as success — so the redundancy was invisible.

That reframes the fix from "remember to check CI before publishing" to **"stop publishing
locally at all"**, which is structural rather than a rule an agent must remember. This is
already recorded in `AGENTS.md` (the "DO NOT `cargo publish` locally" bullet, 2026-08-17).

## Expected

With local publishing removed, **the tag push becomes the single irreversible act**, so it
is the step to gate:

```bash
gh run watch "$(gh run list --branch main --limit 1 --json databaseId -q '.[0].databaseId')" --exit-status \
  && git push origin "vX.Y.Z"
```

`--exit-status` is load-bearing: a red run exits non-zero, so `&&` never reaches the tag
push. (Same discipline as the existing `AGENTS.md` rule about never piping a command whose
exit status gates an `&&` chain.)

**The remaining hole, and the real deliverable of this issue:** `publish-crates.yml`
self-verifies with only `cargo build --release` — **not the test suite**. So a tag pushed
onto a red commit still publishes. Gating at tag-push time is a convention; gating inside
the workflow is enforcement. Prefer the latter.

## Acceptance criteria

- [ ] `publish-crates.yml` runs the test suite (at minimum `cargo test --workspace --locked`,
      ideally the same gate CI applies to `main`) **before** any publish step, so a tag on a
      red commit cannot publish. This is the load-bearing item.
- [ ] `AGENTS.md` records CI-green-on-the-tagged-commit as a precondition of the tag push.
      *(Done 2026-08-17 — verify it still matches the workflow.)*
- [ ] The `gh run watch --exit-status` snippet is documented in the release mechanics
      (`OSS-RELEASE.md` and/or `AGENTS.md`) so it is copy-pasteable.
      *(Done 2026-08-17 in `AGENTS.md`; `OSS-RELEASE.md` still owes it.)*
- [ ] `OSS-RELEASE.md`, the `/oss-release` path, and any bundled release guidance stop
      describing a local two-crate `cargo publish` and reflect the tag-triggered CI flow.

## Notes

Filed at Jari's request during the stint-4 handoff, after he asked "is there an easy way to
fix this?" about the publish ordering. Laned to `skills` because the largest part of the
deliverable is documentation / release-mechanics prose; the one code change is a workflow
file, which collides with nothing else in the DAG.

**Deliberately NOT in scope:** documenting that the local gate runs on macOS and is blind to
Linux-only failure classes. Jari's call at the same handoff — the machines are moving to
Linux shortly, so the blind spot resolves itself and a note about it would be stale on
arrival. The ETXTBSY incident that exposed it is recorded in `TODO.md` as a KEY LEARNING for
context.
