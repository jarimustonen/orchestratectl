# R7 distribution preparation and later substitutions

## Frozen R7 pre-R9 topology

- cargo-dist is pinned to 0.28.2 and distributes exactly one app, `taskfleet`.
- The only generated formula destination is
  `jarimustonen/homebrew-taskfleet`; the old tap remains static and unchanged.
- Canonical archives contain only the `taskfleet` executable. The generated
  shell installer and formula install only `taskfleet`.
- `orchestratectl-installer.sh` is the sole old-name release artifact. It prints
  the canonical latest-installer URL, states that it changed nothing, and exits
  1. It exists only for the old `releases/latest/download/...` compatibility
  URL through 0.7.x.
- The generated workflow uses `HOMEBREW_TAP_TOKEN` once and routes
  `aarch64-apple-darwin` through the `macOS` label on the receipted self-hosted
  ARM64 runner, rather than matching every self-hosted machine. Linux remains on
  `ubuntu-22.04`.
- cargo-dist is generated with `dispatch-releases = true`: there is no tag-push
  trigger, and manual dispatch defaults to the non-publishing `dry-run` value.
  The generated plan-job dependency invokes `taskfleet-release-gate.yml`; every
  non-dry publishing path must pass `scripts/verify-release-activation.sh`
  before artifact builds, hosting, or tap-secret use. Because cargo-dist's host
  job accepts skipped build dependencies, a rejected non-dry dispatch also uses
  its narrowly scoped `actions: write` permission to cancel the complete run.
  cargo-dist 0.28.2's parallel `host --steps=create` planning call was separately
  run without GitHub
  credentials and changed neither public release nor tag-ref digests; its receipt
  records that it only produced a local manifest. R9 must deliberately
  restore tag dispatch after canonical identity and runner validation. Release
  execution is independently blocked by
  `release/taskfleet-release.json: activation=blocked-r8-r9-r10`. The source
  repository currently has no tag ruleset (receipted honestly); safety therefore
  does not claim one and instead uses the generated trigger posture plus both
  workflow and wrapper activation gates.

## Exact R9 substitutions

R7 deliberately keeps source-hosting coordinates truthful while the public
source repository is still `jarimustonen/orchestratectl`. R9 must make this one
identity transaction after renaming the GitHub repository:

| Surface | Pre-R9 (current) | Post-R9 required |
|---|---|---|
| `release/taskfleet-release.json.repository` | `jarimustonen/orchestratectl` | `jarimustonen/taskfleet` |
| Cargo workspace `repository` / `homepage` | `https://github.com/jarimustonen/orchestratectl` | `https://github.com/jarimustonen/taskfleet` |
| cargo-dist plan hosting owner/repo | `jarimustonen` / `orchestratectl` | `jarimustonen` / `taskfleet` |
| generated installer/archive URLs | old source repository (truthful now) | canonical source repository |
| cargo-dist trigger | workflow dispatch; default `dry-run`; no tag trigger | set `dispatch-releases = false`, set distribution trigger to `tag-push`, keep release/distribution activation blocked for R10, regenerate, and verify the trigger |
| R7 posture assertions | Rust/shell checks require dispatch-only, inert secret, blocked state | update both checks to require canonical tag-push, inert secret, and `prepared-blocked-r10` state |
| old installer stub URL | canonical future URL; artifact cannot publish in R7 posture | unchanged; URL resolution remains deferred until R10 publishes the first canonical release |
| release-wrapper expected repo | data-driven from release topology | changes with the topology row above |
| Actions workflow action references | third-party action names unchanged | third-party action names unchanged |
| Homebrew checkout destination | `jarimustonen/homebrew-taskfleet` | unchanged |
| Homebrew credential | inert `HOMEBREW_TAP_TOKEN` after bounded owner proof | keep inert in R9; R10 installs and proves the least-privilege token before release activation |
| macOS runner selector | `macOS` label on current self-hosted ARM64 runner | unchanged unless a unique Taskfleet label is provisioned; run renamed-repo acceptance job |
| `origin` fetch/push URLs | old source repository | canonical source repository |

After those substitutions, rerun cargo-dist 0.28.2 `generate`, `plan` and the R7
machine assertions. R9 proves the repository-scoped runner but leaves the tap
credential inert and release activation blocked for R10. Do not rely on GitHub
redirects for maintained action, installer, badge, Cargo metadata or
release-wrapper coordinates.

## Public mutation boundary and receipts

On 2026-09-02 the authenticated owner `jarimustonen` created public repository
`jarimustonen/homebrew-taskfleet` (GitHub repository id `1355125556`). For the
bounded proof, the source repository's `HOMEBREW_TAP_TOKEN` secret was assigned
the same authenticated owner credential used by an HTTPS push. That procedure
pushed one parentless, empty-tree commit to `main`:

- commit: `db12bb163e47617f0b941a35d3896b6ba0548892`
- tree: `4b825dc642cb6eb9a060e54bf8d69288fbee4904` (Git's empty tree)
- files/formulae after proof: zero

The empty-tree root commit is the reversible token-write proof: it leaves no
formula/content active and its sole ref can be removed without restoring any
repository bytes. API receipts prove the secret update time, commit/ref/tree, repository
ownership and empty final contents; the local assignment-and-push procedure is
the authentication linkage (GitHub does not disclose a secret value in API
receipts). After proof, the live Actions secret was replaced with an inert random
value, so the broad owner credential is not left available to Actions. R9 keeps
that inert value; R10 must install and prove a least-privilege token before
release activation. No release, asset, tag, crate, source-repository rename,
formula, old-tap commit, Homebrew installation, user tap, installed Taskfleet
binary, or installed skill was created or changed during R7.

The old-tap patch under `old-tap-migration/` is local preparation only. R11 owns
its public push after canonical formula verification.
