---
created: 2026-08-15
updated: 2026-08-15
type: bug
status: fixed
priority: high
labels: [ci]
closed: 2026-08-15
---

# CI red on main: rustdoc unresolved intra-doc link crate::spinoff::approve

## Description

`main` CI is red on the `docs` job (runs 31809296808 commit 21b3658-era, and
31800226501). rustdoc fails to resolve an intra-doc link and, because docs are
built with `-D warnings`/broken-link denial, the job exits 101.

## Failure
```
error: unresolved link to `crate::spinoff::approve`
error: could not document `taskfleet`
warning: build failed, waiting for other jobs to finish...
##[error]Process completed with exit code 101.
```
Job: `docs`.

## Root cause
A doc comment references `crate::spinoff::approve` via an intra-doc link, but that
path no longer resolves — most likely the `spinoff::approve` item was renamed,
moved, or removed (the run-kinds / subtractive cut on 2026-08-14 reshaped the
spinoff surface), leaving a stale `[crate::spinoff::approve]` reference behind.

## Fix
Find the dangling reference (`rg 'spinoff::approve' src/`) and either repoint it at
the item's new path or drop the link. Verify locally with
`cargo doc --no-deps --document-private-items` (same denial as CI) before pushing.
