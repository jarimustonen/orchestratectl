---
created: 2026-08-06
updated: 2026-08-06
type: bug
status: fixed
priority: high
closed: 2026-08-06
---

# self-hosted hauis runner: git checkout fails with HTTP 400 (blocks mac binary + Homebrew release)

## Description


The `build-local-artifacts (aarch64-apple-darwin)` job of the cargo-dist **Release**
workflow (runs on the self-hosted `hauis` mac runner) fails at `actions/checkout@v4`:

```
fatal: unable to access 'https://github.com/jarimustonen/taskfleet/': The requested URL returned error: 400
The process '/opt/homebrew/bin/git' failed with exit code 128
```

## Timeline
- **v0.1.1** Release (~08:10 today): **succeeded** — mac binary + Homebrew tap published.
- **v0.1.2** Release (~09:45): **failed** at checkout (400) — mac build never ran; `build-global-artifacts` / `host` / `publish-homebrew-formula` / `announce` all skipped.
- **v0.1.3** Release (~11:52): **failed identically** at checkout (400).

So the runner regressed between ~08:10 and ~09:45 today. Persistent, not transient.

## Impact
- crates.io is fine: `taskfleet-core`/`taskfleet` **0.1.3** published (and 0.1.2 before it).
- **GitHub Release prebuilt binaries + the Homebrew tap are STUCK at 0.1.1** — every release since fails to publish assets. `brew upgrade jarimustonen/taskfleet/taskfleet` still installs 0.1.1.

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

## RESOLVED 2026-08-06

**Root cause:** the `hauis` **global** git config (`~/.gitconfig`) carried a leaked
`http.https://github.com/.extraheader = AUTHORIZATION: basic <stale ghs_ token>` plus two
leaked `url.https://github.com/.insteadof` rewrites (`git@github.com:`, `org-1272053@github.com:`)
— all values `actions/checkout` normally writes to the *local* repo config and removes in its
post step, but here they had leaked into `--global`. `http.extraheader` is **multi-valued**, so
each checkout sent BOTH the fresh per-job token AND the stale global one as two `Authorization`
headers → GitHub rejected the conflicting auth with **HTTP 400** at the checkout step, failing
the mac build (and thus the binary upload + Homebrew publish) for v0.1.2 and v0.1.3.

**Fix applied on `hauis`:**
```
git config --global --unset-all "http.https://github.com/.extraheader"
git config --global --unset-all "url.https://github.com/.insteadof"
```
Then re-ran the v0.1.3 Release workflow (`gh run rerun 31099042451 --failed`) → **all jobs green**:
mac build cleared checkout (1m26s), build-global-artifacts → host → publish-homebrew-formula →
announce all succeeded. GitHub Release v0.1.3 now carries the mac+linux binaries + installer, and
the Homebrew tap formula is at `version "0.1.3"`. crates.io was already at 0.1.3.

**Watch-item (recurrence):** the deeper question of WHY checkout leaked to `--global` on this
runner (normally it writes local; possible `HOME`/`persist-credentials` interaction) is not fully
root-caused. If binary/brew releases 400 again, re-apply the two `--unset-all` above first. A
permanent fix would investigate the runner's checkout config so the leak can't reoccur.
