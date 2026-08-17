---
created: 2026-08-17
updated: 2026-08-17
type: bug
reporter: agent
status: fixed
priority: high
closed: 2026-08-17
---

# Doctor skill test depends on host tools

## Description

Main CI run 32060854209 on commit `4f38174` failed on Ubuntu and macOS in `forced_full_install_prunes_retired_dag_companion_from_all_mirrors`. The test asserted the global `doctor` exit status even though it only cared about three `skill.orphan.*` warning records. Bare CI runners lack hard dependencies such as tmux, workmux, and issuectl, so unrelated `dep.*` failures correctly made doctor exit 1.

Commit `4f38174` only replaced `push_str(format!(...))` with `writeln!`; it did not cause the failure. A stripped-PATH reproduction showed 42 ok, 3 expected orphan warnings, and 3 unrelated dependency failures.

This is the second CI-red this round and the third consecutive round with a CI-red that a green local gate missed. The documented local green gate in `AGENTS.md` uses `cargo test --workspace` in debug mode on a fully equipped developer machine, while CI uses `cargo nextest --locked --release` on a bare runner. The gate documentation should be reconsidered by a human, but that product/documentation decision is outside this focused test fix.

## Reproduction

Run the release nextest case with a PATH containing the Rust toolchain plus system git, but excluding tmux, workmux, and issuectl. Before the fix, the orphan records are present as warnings but the test fails on `doctor.status.success()` because the dependency checks are failures.

## Quick Test

Run `cargo nextest run --locked --release --workspace` and repeat the targeted test with the stripped PATH. The test must retain all three orphan-warning assertions and all post-install pruning assertions.

## Acceptance Criteria

- [x] The release nextest suite passes.
- [x] The targeted test passes with tmux, workmux, and issuectl absent from PATH.
- [x] The test still verifies all three orphan warnings and all three mirror-pruning outcomes.

## Resolution

### 2026-08-17T19:49:39Z · @issuectl

Fixed by asserting the targeted doctor check records rather than the environment-dependent aggregate exit status; stripped-PATH release nextest reproduction is green.
