# Taskfleet clean-break implementation plan

This plan implements the amended [ADR 0002](../../docs/decisions/0002-taskfleet-identity.md).

## Binding rules

- Maintain one product, repository, package family, command, home, configuration
  surface, environment namespace, protocol namespace, skill catalog, and release
  topology.
- Do not preserve compatibility code or transition artifacts in maintained HEAD.
- Do not alter immutable registry/git history or any external repository.
- Do not release, tag, publish, install, or move real state from implementation
  worktrees.

## Implementation

1. Delete the secondary package and executable.
2. Simplify home and repository configuration resolution to `TASKFLEET_HOME`,
   `~/.taskfleet`, and `.taskfleet.toml` only; delete state movement machinery.
3. Canonicalize worker, notification, readiness, test, telemetry, and internal
   variables under `TASKFLEET_*`, plus Taskfleet tracing and adapter identities.
4. Remove renamed-skill movers, transition fixtures, receipts, warnings, and
   installer/tap transition artifacts.
5. Make Cargo, Shipshape, crates.io CI, cargo-dist, and Homebrew contracts publish
   only `taskfleet-core` then `taskfleet`, with `taskfleet` as the only binary.
6. Rewrite maintained docs, decisions, tests, snapshots, and issue records so
   paths and content contain no retired identity.
7. Record a deterministic tracked-source inventory proving zero references.

## Verification

- `git ls-files` and `git grep` strict retired-identity search
- `cargo metadata --locked --no-deps --format-version 1`
- deliberate insta snapshot review until no pending snapshots remain
- `cargo fmt --all --check`
- `cargo clippy --locked --workspace --all-targets -- -D warnings`
- `cargo nextest run --locked --release --workspace`
- `cargo test --locked --release --workspace --doc`
- `RUSTDOCFLAGS="-D warnings" cargo doc --locked --workspace --no-deps`
- release topology, publication fixture, authorization, and distribution tests
- tool-sensitive tests under a stripped `PATH`
- `cargo package --workspace --locked --no-verify` and archive inspection

Any required failure blocks completion. The issue closes only after all checks
pass and the implementation report names the exact verified commit.
