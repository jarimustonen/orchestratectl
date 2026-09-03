# ADR 0002 R8 integrated pre-cut validation

## Decision boundary

This evidence validates only the exact integrated pre-R9 source commit
`c3ef8b740ac531f12ce81c759ed209d178cf36bd` (tree
`b7d07d9df3308fb33afdfab892f949f46ef810d4`). It can authorize only R9's
source-repository rename. It expires as soon as repository identity changes.
R10 must repeat the full gate on its actual post-R9 candidate before any tag,
publish, hosted formula, or old-tap activation.

All production files came from that commit. Later commits in this run contain
only issue/evidence artifacts. No release cut/resume, tag, publish, GitHub source
rename, public tap mutation, global install, installed-skill change, or state
migration was performed.

## Result

**PASS, subject to the immutable evidence index and review below.** Every required R8 leg passed against the exact tested commit. The result authorizes only R9's source-repository rename; it does not authorize a release, tag, publication, tap activation, or R10/R11 action. Complete committed logs are deterministic sanitized transforms; their original machine-private byte hashes are retained in `outputs-summary.txt`, while the index binds the committed transformed bytes. The machine-readable authority is
[`evidence/index.json`](evidence/index.json); command details are in
[`evidence/command-manifest.json`](evidence/command-manifest.json).

## Exact source and CI

- Local `HEAD`, `origin/main`, and the tested SHA were identical at start.
- GitHub Actions run `33764612111`, workflow `303193881`, run number `259`, is a
  completed successful `push` run at the exact tested SHA.
- All eight required jobs succeeded; immutable job IDs and timestamps are in
  `evidence/r8-ci-jobs-summary.json`. The run had zero artifacts, recorded
  explicitly in `evidence/r8-ci-artifacts-summary.json`.
- The checked-in `ci.yml`, `Cargo.toml`, and `Cargo.lock` digests are pinned in
  `evidence/source-identity.json`.

## Coverage interpretation

The release-mode workspace suite executes 1,115 tests and directly includes:

- `dual_name_resolver.rs`: explicit/default roots, in-place 0.5.1 adoption,
  equivalent/conflicting variables, repository config, split truth, warning and
  filesystem-purity boundaries;
- `state_migration.rs`: terminal migration and byte preservation, active and
  pending-merge refusal, held locks/live workers/open descriptors, dual roots,
  destination/symlink faults, prepared/renamed crash recovery, and both sides of
  the first-canonical-write rollback boundary;
- `compat/orchestratectl/tests/compatibility.rs` plus
  `verify-command-parity.sh`: shared wrapper dispatch, byte-identical ordinary
  text/JSON/JSONL stdout, equal exits, parseable streams, bounded stderr-only
  deprecation, invalid input, help identity, and hidden-child suppression;
- `skill.rs`: old provenance/managed markers, hash-pinned migration, edited,
  unmanaged, stale, corrupt, partial-ownership, prune, and no-clobber cases;
- `package_graph.rs`, release scripts, and package archives: exact three-crate
  graph, one engine, implementation-free wrapper, exact pins, and five release
  legs.

The 0.5.1 fixture was re-fetched from published crates.io/GitHub artifacts. The
published release binary identifies version `0.5.1` and commit
`f0c52ab232706fb480a51bfd45f2171c6b7aa056`; baseline and current compatibility
verification both passed without protected-byte changes. Fixture and artifact
hashes are committed.

## Rust, snapshots, isolation, and release protocols

The full local gate passed: formatting; locked workspace/all-target clippy with
warnings denied; locked release nextest with `--no-fail-fast`; workspace
release doctests; rustdoc with warnings denied; and `git diff --check`.
The documented insta loop passed with no `.snap.new`; before/after snapshot
hashes were identical. A second release nextest run used an `env -i` home and a
small declared tool directory, excluding ambient user-installed harness tools.
Early setup attempts omitted macOS `xcrun`, invoked the nextest plugin without
its required `nextest` argv, omitted fixture commands `true` and `yes`, or
resolved shell-builtin `true` to a self-referential link; their failures are
excluded and disclosed in `evidence/isolation-diagnostics.json`. The final run
declared `xcrun`, but rustc still emitted a non-fatal SDK lookup `ENOENT`; the
warning is retained in the complete log rather than described as corrected.
That run nevertheless built every release test binary, exited 0, and passed all
1,115 tests. Nextest marked one pure schema round-trip test `LEAK` for delayed process exit. The ordinary full gate had no leak marker, no assertion was flaky, and residue checks found no candidate process. The marker moved between two process-free unit tests under load and is distinct from fixed prerequisite `@native-spawn-test-leaks`; `evidence/leak-disposition.json` records the disposition. This warning is disclosed rather than silently normalized. The registry and Shipshape fixture suites
also use credential-free, stripped PATHs and stubs for every side-effecting tool.

Every release protocol fixture passed: registry reconciliation, bump hook,
wrapper preflight, held-tag journal, real exact-0.10.1 protocol, version
snapshots, topology validation, and expected refusal by the still-blocked
activation gate. No cut or direct resume was invoked.

## Packages and install channels

`cargo package --workspace --no-verify` produced three hashed archives. The
wrapper archive contains only its manifest/metadata, legal/docs files, one
`src/main.rs`, and compatibility tests; implementation remains in `taskfleet`.
Canonical Cargo, raw-archive, and locally redirected generated-shell checks
install/run only `taskfleet`; a separate bounded Cargo-wrapper check installs
only `orchestratectl`. All attest the tested source commit and leave user paths
untouched.

cargo-dist 0.28.2 `generate --check`, plan validation, native artifact build,
archive/formula/shell/stub checks, old-tap patch application, and isolated
Homebrew resolution passed. Local executable installation is necessarily native
macOS ARM64; cargo-dist plan/check evidence validates the required Linux musl
ARM64/x86_64 target definitions, while R10's hosted build owns actual
cross-platform release artifacts. The fuller pre-live Homebrew drill uses a cloned
Homebrew prefix, real old 0.5.1 receipt, local canonical formula/archive, and
local-only tap commits. The clean bounded `verify-homebrew-prelive.sh` run passed the required old-receipt update/migration, canonical receipt-ownership assertion, upgrade, stale-link removal, uninstall, fresh canonical install, and final cleanup. This is deliberately labelled **pre-live**: no hosted Taskfleet formula exists until R10, and R11 owns public old-tap migration activation.

Several earlier diagnostics failed because the installed old tap had not fetched the new migration commit, commands were ambiguous while both taps were present, duplicate optional rename metadata scheduled the same migration twice, or receipt assertions used an incorrect path/name. None is passing evidence. `evidence/homebrew-diagnostics.json` records each superseded attempt; only the final successful clean run is authoritative.

The generated old latest-installer artifact exits 1, writes only the migration
message to stderr, points at the canonical installer, and leaves its disposable
home empty. The live old latest URL still serves the truthful 0.5.1 installer;
the new URLs are 404 before release. These are current facts, not failures of
pre-live output.

## Shipshape 0.10.1

The exact admitted source commit
`3e46568d6969701c5fea82fb134b62aa17121cbe` was built in disposable source and
target directories. Contract show/validate and audit passed with zero blocking
gaps. Wrapper command `scripts/shipshape-release.sh plan minor` sealed plan
`533b3868a611678433437f23d022d4cd4385e13b5c72ec3cdf2252cad1e4ce54`:
head SHA is exact, target version is 0.6.0, both exact-pin rewrites are present,
and targets are the three crates.io CI legs plus independent GitHub Release and
Homebrew cargo-dist legs.

The plan's warning that tag-triggered Cargo publication was not detected is
expected pre-R9 evidence, not permission to cut: the release workflow is
intentionally disabled and activation remains `blocked-r8-r9-r10`. R9 must
restore canonical repository identity/triggers; R10 must re-plan and require
that warning to disappear.

## Fresh public facts

At the recorded query times:

- crates.io candidate endpoints `taskfleet` and `taskfleet-core` returned 404;
  published `orchestratectl` and `octl-core` remain at 0.5.1;
- source repository `jarimustonen/orchestratectl` exists at the exact tested
  `main`; `jarimustonen/taskfleet` returns 404;
- canonical tap `jarimustonen/homebrew-taskfleet` remains the empty-tree proof
  commit `db12bb163e47617f0b941a35d3896b6ba0548892`;
- old tap remains `85ce830378f38cf17283efddd966d5754354e403` with formula blob
  `c7d02e0e61f16e347f01bed09473fa7b86b5034f`;
- Homebrew core formula API endpoints for both names return 404;
- the source repository still has the named tap secret metadata and an online,
  idle self-hosted macOS ARM64 runner (runner name sanitized).

404s are observations, never reservations.

## Limitations and residue

- CI produced no artifacts; local package/distribution hashes are therefore the
  pre-cut artifact evidence.
- `issuectl doctor` exits successfully with no parse/schema/reference/cycle
  failures. It still reports pre-existing generated AGENTS drift and the legacy
  unknown `deliverable` field on `arch-supervision-alternatives`; neither was
  rewritten in this focused evidence run.
- During script development, four version probes initially omitted explicit
  sandbox homes and each appended one identifiable JSONL dispatch line to the
  user's legacy log. Each exact line was removed immediately; no run/event/
  projection/config/skill bytes or state-home placement changed. No pre-probe
  log digest was captured, so complete byte restoration cannot be independently
  proven and file mtimes changed. Final channel scripts and every authoritative
  replacement run use isolated homes. This is a disclosed exploratory-probe
  incident, not passing gate evidence.
- Setup corrections (cargo-dist installs its executable as `dist`, a source-built
  crate lacks release build provenance, direct `cargo-nextest` needs its plugin
  argv, shell installer uses `INSTALLER_DOWNLOAD_URL`, and Homebrew migration
  requires qualified rename metadata) were corrected before authoritative
  reruns. They are not waived test failures.

Immediate post-test checks found no candidate process. The final residue gate
must still show the tested source production tree unchanged, only issue/evidence
changes in git, no candidate supervisor leaked by tests, no release/tag/public-
state mutation, and the unrelated rename worktree untouched.
