---
created: 2026-08-06
updated: 2026-08-06
type: bug
status: open
priority: high
---

# self-hosted hauis runner: git checkout fails with HTTP 400 (blocks mac binary + Homebrew release)

## Description


The `build-local-artifacts (aarch64-apple-darwin)` job of the cargo-dist **Release**
workflow (runs on the self-hosted `hauis` mac runner) fails at `actions/checkout@v4`:

```
fatal: unable to access 'https://github.com/jarimustonen/orchestratectl/': The requested URL returned error: 400
The process '/opt/homebrew/bin/git' failed with exit code 128
```

## Timeline
- **v0.1.1** Release (~08:10 today): **succeeded** — mac binary + Homebrew tap published.
- **v0.1.2** Release (~09:45): **failed** at checkout (400) — mac build never ran; `build-global-artifacts` / `host` / `publish-homebrew-formula` / `announce` all skipped.
- **v0.1.3** Release (~11:52): **failed identically** at checkout (400).

So the runner regressed between ~08:10 and ~09:45 today. Persistent, not transient.

## Impact
- crates.io is fine: `octl-core`/`orchestratectl` **0.1.3** published (and 0.1.2 before it).
- **GitHub Release prebuilt binaries + the Homebrew tap are STUCK at 0.1.1** — every release since fails to publish assets. `brew upgrade jarimustonen/orchestratectl/orchestratectl` still installs 0.1.1.

## Likely cause (needs on-machine inspection of `hauis`)
`/opt/homebrew/bin/git` on the runner gets HTTP 400 accessing github.com — smells like a
git http config / credential-helper / proxy or network-appliance issue on the runner host,
NOT a repo or workflow problem (same workflow succeeded at v0.1.1). Candidates to check on `hauis`:
- `git config --global --list` / `~/.gitconfig` http proxy or `http.extraHeader` leftovers
- a stale/ës malformed credential helper or `insteadOf` rewrite
- a network proxy/appliance in front of the runner now returning 400
- runner-agent version / token

## Fix path
Inspect + repair git access on the `hauis` runner, then **re-run the v0.1.3 Release workflow**
(`gh run rerun <id>` or re-push the tag) to publish the 0.1.3 mac binary + Homebrew formula —
no new version bump needed (crates.io already has 0.1.3).
