# R11 public Homebrew migration evidence

This immutable, secret-free bundle closes ADR 0002 R11 only. It independently re-read the public old and canonical taps after the conductor's normal old-tap push, then exercised every supported migration/install path against those public remotes. It does not authorize or perform dependent-repository migration, installed-skill replacement, real-user state movement, release changes, tag changes, or tap writes.

## Result

All paths converged on the sole formula `jarimustonen/taskfleet/taskfleet` at v0.6.1. Every executed Taskfleet runtime reported embedded commit `7e93bd6195fbaf6de0b43d9161228ae2373ab5d1`. No `orchestratectl` executable or alias remained, one physical canonical formula/rack owned each final installation, every final uninstall emptied its disposable Cellar, and every disposable root was removed. After a receipt migration, `brew list --formula` truthfully projects both the historical receipt name `orchestratectl` and canonical name `taskfleet`; the captured filesystem inventory proves this is not duplicate ownership: only the `taskfleet` rack, v0.6.1 directory, formula file, and binary exist.

Homebrew's trust boundary has two truthful outcomes:

- When the canonical destination was already explicitly trusted, `brew update` consumed `tap_migrations.json`, moved the old 0.5.1 keg to the canonical rack, and `brew upgrade` installed 0.6.1. A subsequent explicit `brew migrate orchestratectl` was a successful no-op because update had already consumed the migration.
- Without destination trust, `brew update` refused to auto-tap the third-party destination and printed the exact safe instructions; `brew upgrade` preserved the old keg. After explicitly tapping/trusting the public canonical destination, `brew migrate orchestratectl` moved the receipt and `brew upgrade` reached 0.6.1.

## Artifacts

- `public-state.json` — exact public repository IDs, heads, parent/tree identities, complete inventories, old migration bytes/hash, canonical formula bytes/hash/version/URLs/checksums, and the one-formula ownership proof.
- `homebrew-paths.json` — sanitized exact old/new receipts, resolution/info records, version responses, trust-boundary behavior, commands and isolation/cleanup result for fresh canonical, old-qualified, automatic old-receipt, explicit migration, direct canonical reinstall, and final uninstall paths.
- `collect-public-state.py` — network collector and strict public-state assertions.
- `verify-homebrew-paths.sh` — bounded public-remote Homebrew path test.
- `verify-evidence.sh` — offline checksum, semantic, sanitization, receipt and runtime verifier.
- `index.json` — byte lengths and SHA-256 checksums for every other artifact.

## Provenance and isolation

Public GitHub reads use a meaningful `taskfleet-r11-evidence/1.0` User-Agent. A current GitHub token is supplied at runtime only to avoid anonymous API limits and Homebrew's interactive credential-helper lookup; no value is printed or retained. Each Homebrew path creates a distinct prefix under the user's non-temporary cache directory, with isolated `HOME`, cache, prefix, Cellar and taps. The disposable Homebrew clone shares only read-only Git objects from the system Homebrew repository. Every Homebrew command is non-interactive and bounded to 300 seconds. Taps are freshly cloned from their public HTTPS remotes and pinned/checked before use. Analytics and install cleanup are disabled. The system Homebrew Cellar, taps, receipts, installed binaries/skills, and real user state are never read as test state or modified.

The historical old-receipt fixtures clone the actual public old tap first, prove its live head, then check out the exact public parent `85ce830378f38cf17283efddd966d5754354e403` solely inside that disposable tap to install its truthful 0.5.1 formula. `brew update` must then fetch and reach the actual public migration head `20a70f463e699af5ddba6f6455c20a183c496ca5`.

Recollect public state (networked):

```sh
GH_TOKEN="$(gh auth token)" ./collect-public-state.py public-state.json
```

Re-run the destructive-looking but fully disposable Homebrew drill (networked):

```sh
./verify-homebrew-paths.sh homebrew-paths.json
```

Verify the committed bundle without network access:

```sh
./verify-evidence.sh
```

This evidence authorizes only post-live dependent-repository **owner discovery** (ADR 0002 E1). It does not authorize blind replacement, repository convergence, machine deployment, skill/binary installation, or state migration.
