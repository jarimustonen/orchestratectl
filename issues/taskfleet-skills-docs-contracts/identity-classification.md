# R5 residual old-identity classification

`check-residual-identity.py` performs a case-insensitive tracked-tree search for
`orchestratectl` and `octl` and fails on any occurrence outside these explicit
classes. Its current deterministic counts are in `identity-classification.tsv`.

- **Bounded compatibility:** the Cargo-only `orchestratectl` wrapper, old branded
  environment/config/home inputs, 0.5.1 state exclusion text, legacy skill names
  and ownership markers, invocation identity, and old tracing targets retained so
  0.6/0.7 users can keep existing log filters.
- **Permanent protocol/safety:** every `OCTL_*` exec/notification/test-control
  variable and telemetry contract id
  `orchestratectl.worker-telemetry-adapter`; historical schema/state readers and
  split-root safety paths.
- **Fixture/history:** 0.5.1 fixture bytes, compatibility tests, accepted ADRs,
  issue records, CHANGELOG entries, and the persisted prompt that launched this
  worktree. None are rewritten.
- **Generated/vendor:** Cargo.lock, insta snapshots, and cargo-dist's generated
  workflow. Inputs are changed only by their owning R6/R7 phase.
- **Deferred external/public convergence:** the current GitHub repository, old
  tap/formula and release scripts/workflows remain truthful until R7/R9/R11.
  The approved pre-cut block prohibits activating them here.

The active bundled skill templates contain no old identity except stable
`OCTL_*` protocol names. New generated worker prompts, report commands, source
references, diagnostics, examples, and telemetry endpoint argv use `taskfleet`.
