# R10 final release receipts

This immutable bundle closes ADR 0002 R10 truthfully: v0.6.0 is a burned, unpublished coordinate and v0.6.1 is its successful fix-forward. No secret values or concrete disposable paths are retained.

## Receipts

- `public-state.json` — fresh crates.io, GitHub Release/assets, refs, tap/formula, archive, shell-installer, and inert-stub reconciliation.
- `workflows.json` — exact Taskfleet release and Shipshape verifier source-fix workflow/job IDs and conclusions.
- `journal-v0.6.0.json` / `journal-v0.6.1.json` — sanitized terminal Shipshape projections.
- `homebrew-install-result.txt` — fresh canonical install/uninstall result from a disposable non-`/tmp` Homebrew prefix.
- `index.json` — byte sizes and SHA-256 checksums for every other file in this directory.

## Provenance and reproduction

Public evidence was collected with:

```sh
issues/taskfleet-release-0-6-0/evidence/final/collect-public-evidence.py <output-directory>
```

The collector uses a meaningful `taskfleet-r10-evidence/1.0` User-Agent, uses the current `gh` token only to avoid GitHub's low anonymous read limit (the value is never printed or retained), expects HTTP 404 for every v0.6.0 package and release, downloads every v0.6.1 crate and release asset, and exits nonzero on any mismatch. It uses only temporary download/install homes, which are deleted and are not recorded.

The independent Homebrew proof was run with:

```sh
issues/taskfleet-release-0-6-0/evidence/final/verify-homebrew-install.sh \
  > issues/taskfleet-release-0-6-0/evidence/final/homebrew-install-result.txt
```

It creates its prefix under `$HOME/Library/Caches` rather than `/tmp`, clones Homebrew into that prefix, isolates `HOME` and cache, disables auto-update and analytics, bounds each Homebrew command to 300 seconds, explicitly trusts the canonical tap without a prompt, and removes the complete root on every exit. It never uses the system Cellar/taps or installs globally.

Workflow receipts came from `gh run view <id> --repo <repository> --json …`; journals came from `shipshape release show <run-id> --json` with only terminal state, phases, tags, verification, and sequence watermarks retained. Tap heads and formula bytes were re-read from public GitHub endpoints. No credential values are present.

Verify the committed bundle without network access:

```sh
issues/taskfleet-release-0-6-0/evidence/final/verify-evidence.sh
```
