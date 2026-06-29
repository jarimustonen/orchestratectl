---
created: 2026-06-29
updated: 2026-06-29
type: bug
status: open
priority: normal
---

# Doc-link cleanup: octl-core has ~15 broken intra-doc links (CI doc job fails)

## Description

`cargo doc --workspace --no-deps` under `RUSTDOCFLAGS=-D warnings` (the CI doc job's setup) fails with ~15 errors in `crates/octl-core`. All are "public documentation links to private item" — public APIs reference private helpers like `find_prior_with_key`, `for_each_event_probe`, `PhysicalLineReader`, `reject_symlink`, `is_canonical_disc_or_proposal_body`, `apply_event`.

Reproduce:

```
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps
```

Approach: for each link, decide whether the target wants to be `pub(crate)` exposed for docs, kept private with the link removed, or replaced with a prose reference. Don't blanket-allow the lint — these links are real "what should the reader read next" signals that lose meaning if they silently rot.

The two trivial cases in `octl-cli` (`Lifecycle::Autonomous` in `supervise/cleanup.rs`, `Child::try_wait` in `supervise/watchdog.rs`) were already qualified by commit `40e2add`. This issue tracks the remaining `octl-core` ones.

Surfaced 2026-06-29 while preparing v0.1.0.
