---
created: 2026-08-18
updated: 2026-08-18
type: feature
reporter: jari
status: open
priority: normal
---

# doctor cannot detect that it is validating stale skills with a stale binary

## Description

## Observed (twice in one session, 2026-08-17)

`orchestratectl doctor` reported **0 warn / 0 fail** while running a binary that was several
commits behind the repo, validating bundled skills that the same stale binary had installed.

Sequence: a worker ran `cargo install --path crates/octl-cli` from inside its worktree,
which replaced `~/.cargo/bin/orchestratectl` and recorded the worktree as the install source.
When that worktree was torn down the binary later disappeared entirely. `~/.cargo/bin`
precedes `/opt/homebrew/bin` on PATH, so invocations silently fell through to the **Homebrew
tap build from an older release**. That stale binary then reinstalled its own (pre-migration)
bundled skills over the corrected ones, undoing part of the round's work.

Throughout, `doctor` was green — correctly, by its own contract: the stale binary's skills
matched the stale binary. Skill-sync is internally consistent; nothing in doctor's model
notices that the *binary itself* is not the one the operator thinks they are running.

## Why this is worth fixing

The failure is silent and self-concealing: every check the operator would naturally run to
detect it (does it run? what version? does doctor pass?) returns a reassuring answer. The
only thing that caught it was comparing the binary's build commit against `git rev-parse HEAD`
by hand. A version string is insufficient — the stale binary reported a plausible version.

## Suggested direction

`doctor` should surface the running binary's build commit (it is already in
`version --output json` as `.data.commit`) as part of its report, so a stale install is
visible without having to suspect it in advance. Deciding whether it is *stale* needs a
reference point doctor does not inherently have; options worth weighing:

- Report the commit unconditionally as an informational check (cheap, no false positives).
- When invoked from inside a git repo that is this project, compare against `HEAD` and warn
  on mismatch. Note a legitimate mismatch is common (working on a branch, or deliberately
  running a released binary), so this must warn, not fail.

Prior art worth copying: `ossctl release plan` has an `--allow-stale-binary` escape hatch,
i.e. the same hazard class is already modelled in a sibling tool.

## Interim mitigation (already landed)

`AGENTS.md`'s deploy bullet now requires asserting
`orchestratectl version --output json | jq -r .data.commit` equals `git rev-parse HEAD` after
`cargo install`, and workers are prohibited from global installs (they build and run
`./target/release/orchestratectl` from their own worktree). This issue is about making the
tool disclose it rather than relying on the operator remembering the check.
