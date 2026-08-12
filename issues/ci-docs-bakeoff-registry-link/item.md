---
created: 2026-08-12
updated: 2026-08-12
type: bug
status: in-progress
priority: high
labels: [ci]
---

# CI docs job red: unresolved intra-doc link `bakeoff::registry`

## Description

The `docs` job (cargo doc) fails on `main` (run 31509845355, commit 9c01fba, and every run since), blocking a green CI.

## Failure
```
error: unresolved link to `bakeoff::registry`
error: could not document `orchestratectl`
##[error]Process completed with exit code 101.
```
`cargo doc` runs with `-D warnings` (or `RUSTDOCFLAGS=-D rustdoc::broken_intra_doc_links`), so the broken intra-doc link is a hard error.

## Root cause
A doc comment contains a `[bakeoff::registry]` intra-doc link that no longer resolves — the `registry` item under `bakeoff` was renamed/moved/removed, or the path is wrong from the linking module.

## Fix
Grep for `bakeoff::registry` in doc comments and either fix the path to the current item or escape it as plain code (`` `bakeoff::registry` `` with backtick-only, no brackets). Then confirm locally with:
```
RUSTDOCFLAGS="-D rustdoc::broken_intra_doc_links" cargo doc --no-deps
```
