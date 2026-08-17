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

## Expected

The release sequence should make CI a **hard gate** rather than a parallel activity: push
the release commit, wait for CI to go green on it, and only then publish and tag. The
mechanism is cheap:

```bash
git push && \
  gh run watch "$(gh run list --branch main --limit 1 --json databaseId -q '.[0].databaseId')" --exit-status && \
  cargo publish -p octl-core --locked   # ... then orchestratectl, then the tag
```

`--exit-status` is the load-bearing part: it exits non-zero on a red run, so the `&&` chain
breaks and no publish happens. Note the existing `AGENTS.md` warning about never piping a
command whose exit status gates an `&&` chain — the same discipline applies here.

## Acceptance criteria

- [ ] `AGENTS.md` records CI-green-on-the-release-commit as a **precondition of publishing**,
      alongside the existing green-gate and `--locked` rules, with the reasoning inline.
- [ ] `AGENTS.md` records that the local gate runs on macOS and therefore cannot see
      Linux-only failure classes (ETXTBSY named as the worked example), so a green local run
      is not evidence for that class.
- [ ] The concrete `gh run watch --exit-status` snippet is documented in the release
      mechanics (`OSS-RELEASE.md` and/or `AGENTS.md`) so it is copy-pasteable.
- [ ] The `/oss-release` path and any bundled release guidance reflect the same ordering.

## Notes

Filed at Jari's request during the stint-4 handoff, after he asked both "should this be an
issue somewhere?" (the macOS blind spot) and "is there an easy way to make sure?" (the
publish ordering). Laned to `skills` because the deliverable is documentation//skill-surface
prose, not supervisor or run-state code.
