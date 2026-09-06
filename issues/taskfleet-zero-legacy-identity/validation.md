# Validation

The maintained-source identity inventory is executable and deterministic:

```sh
./scripts/check-canonical-identity.sh
```

It scans every tracked path and all tracked text, case-insensitively, for both
retired identity stems. The accepted result is exactly zero path hits and zero
content hits. Build output and git object history are outside `git ls-files` and
`git grep`, matching the issue boundary.

Release inventory is independently pinned by
`release/taskfleet-release.json`, `release/taskfleet-distribution.json`,
`scripts/validate-release-topology.sh`, and
`scripts/validate-distribution-topology.sh`: two canonical Cargo packages, one
binary, two ordered registry legs, and canonical GitHub/Homebrew distribution.

Validated in the implementation worktree on 2026-09-06:

- canonical identity inventory: passed with zero tracked path/content hits;
- `cargo fmt --all --check` and locked all-target clippy with warnings denied: passed;
- locked release-profile workspace nextest: 1,066 tests passed;
- locked release-profile doctests and warning-denied rustdoc: passed;
- focused version, telemetry, run-worker, package-graph, envelope, and structured
  help suites: passed with snapshots reviewed and no `.snap.new` files;
- release topology, distribution topology/policy, crates.io fixture,
  authorization, held-tag, and release-wrapper scripts: passed;
- cargo-dist 0.28.2 `dist generate --mode ci --check`: passed;
- Shipshape contract validation and audit: passed;
- selected native materialization, all-kind spawn, and worker-root tests passed
  under a stripped environment containing only the declared toolchain and system
  tools. Cargo disclosed expected macOS `rust-objcopy` dynamic-library warnings
  caused by the stripped loader path; compilation and all selected tests passed.

From clean implementation commit `a357c0c`, `cargo package --locked --workspace
--no-verify` produced exactly the two canonical archives (30 and 176 files), and
the exact admitted Shipshape 0.10.1 build at
`3e46568d6969701c5fea82fb134b62aa17121cbe` passed its held-tag protocol gate,
including isolated local resume. The fixture reported production remote
untouched. No release, tag, publish, install, state movement, tap mutation, or
external repository edit was performed.
