# Taskfleet R1–R8 implementation DAG specification

This is the intended materialization source for ADR 0002's pre-publication implementation, not a claim about current scheduler state. Each issue body is self-contained and is intended to become `type: task`, `status: open`, related to (not a child of) the unscheduled parent feature `@rename-taskfleet`, in one conservative lane `taskfleet-rename` with collision token `repository-identity`. The lane deliberately serializes state, package, snapshot and release surfaces even where dependency edges would otherwise permit overlap.

The R0 worker policy permits new issues only through unlaned `issuectl intake file`; it forbids this worker from applying the lane metadata below. Therefore exact open/laned materialization requires a later authorized human lane-or-close disposition. Until that occurs this document is a specification, not an executable DAG, and R0 cannot claim the original task's accepted/open/laned materialization complete.

| R | Slug | `blocked_by` | `lane_seq` | `collision` |
|---|---|---|---:|---|
| R1 | `taskfleet-shared-dispatcher` | `@taskfleet-rename-inventory` | 20 | `repository-identity` |
| R2 | `taskfleet-dual-name-resolver` | `@taskfleet-shared-dispatcher` | 30 | `repository-identity` |
| R3 | `taskfleet-state-migration` | `@taskfleet-dual-name-resolver` | 40 | `repository-identity` |
| R4 | `taskfleet-package-wrapper` | `@taskfleet-shared-dispatcher`, `@taskfleet-dual-name-resolver`, `@taskfleet-state-migration` | 50 | `repository-identity` |
| R5 | `taskfleet-skills-docs-contracts` | `@taskfleet-package-wrapper` | 60 | `repository-identity` |
| R6 | `taskfleet-release-machinery` | `@taskfleet-package-wrapper`, `@taskfleet-skills-docs-contracts` | 70 | `repository-identity` |
| R7 | `taskfleet-distribution-topology` | `@taskfleet-skills-docs-contracts`, `@taskfleet-release-machinery` | 80 | `repository-identity` |
| R8 | `taskfleet-integrated-validation` | `@taskfleet-distribution-topology` | 90 | `repository-identity` |

## R1 — Extract one linkable CLI dispatcher

**Title:** Extract the shared Taskfleet CLI dispatcher

Refactor the current binary entry point into one linkable dispatcher used later by the canonical Taskfleet binary and bounded old CLI wrapper. Keep parser, execution, envelopes, state resolution and error formatting shared. Add explicit invocation identity only where help/version/deprecation require it. Hidden self-exec paths (`supervise`, `run-worker`, merge/recovery, reattach and doctor fix) must use the current executable/shared entry point, never a PATH lookup or a second engine.

**Acceptance:** current `orchestratectl` stdout/JSON/JSONL/exit behavior is unchanged; one dispatcher owns command execution and takes explicit invocation identity without unsafe PATH or argv-name inference; self-exec tests cover every hidden path; full Rust gate and snapshots pass. No package, binary, home, repository or distribution rename occurs.

## R2 — Centralize dual-name inputs and legacy-home adoption

**Title:** Add the Taskfleet dual-name resolver and legacy-home adoption

Implement the ADR home/config/input matrix from the frozen 0.5.1 fixtures. Add canonical `TASKFLEET_HOME`, `TASKFLEET_PROFILE`, `TASKFLEET_HARNESS`, `TASKFLEET_LOG` and `.taskfleet.toml`; retain old branded aliases/fallback through 0.7 with old-only/equal warnings and differing-value refusal. With no explicit home, use canonical-only, adopt legacy-only in place, create fresh canonical when neither is populated, and refuse dual-populated roots. Route logs, doctor, config, skills/provenance, subprocesses and every command through this one resolver. Preserve all `OCTL_*` spellings.

**Acceptance:** define managed/populated roots, lexical/path equivalence, case sensitivity, relative/nonexistent paths, inaccessible and symlink roots, and explicit-home split-root behavior; exhaustive environment/home/repository-config matrix; normalized equivalent paths accepted with warning; split truth refuses reads requiring one root and every write. Resolver/conflict selection occurs before logging or any filesystem write; help is filesystem-pure, conflict warnings are stderr-only/once per top-level invocation/JSONL-safe, and hidden self-exec children do not repeat them. A published 0.5.1 process and the new reader/writer interoperate on one adopted legacy root under the documented operator-exclusion limit. Fixture state/config/provenance bytes do not change (logs are isolated); no physical movement or source/package rename.

## R3 — Implement optional quiescent state migration

**Title:** Implement quiescent same-filesystem Taskfleet state migration

Add dry-run and explicit migration commands. Require exact normalized source/destination, absent destination, external migration lock, same filesystem and quiescence: no non-terminal run, live supervisor/worker, pending merge, held run lock or state-writing command. Validate runs through normal lock/reducer APIs, atomically rename the whole root, write/verify the outside receipt and leave no symlink/alias. Permanently fail on recreated/dual roots. Define first canonical write and permit rename-back only before it.

**Acceptance:** define an outside receipt location/state machine, durable ordering/fsync and fail-closed recovery; add bounded/nonblocking per-run lock checks and state explicitly that future locks cannot fence every old 0.5.1 process or open descriptor, so operator-enforced exclusion is required where automatic proof is impossible. Migration logging stays outside source/destination until resolution; log creation is an explicit first-canonical-write boundary. Fixture event hashes, `applied_seq`, ids, OIDs, branches and pending transaction semantics survive. Runtime builders cover active/stale old processes, open descriptors, pending merge, dual roots, destination, symlink/path, held lock, crash points, receipt faults and cross-device refusal; rollback boundary is tested; no public identity mutation.

## R4 — Rename packages/command and add the old CLI wrapper

**Title:** Create canonical Taskfleet packages and the bounded old CLI wrapper

Rename active packages/layout to `taskfleet-core` and `taskfleet`, exact-pin the canonical core, and make `taskfleet` the sole canonical binary. Add an implementation-free `orchestratectl` compatibility package/binary linked to the shared dispatcher, outside layouts that could produce duplicate target artifacts. It emits stderr-only once-per-process deprecation while preserving machine stdout/JSON/JSONL and exits. Do not publish an `octl-core` wrapper absent an ADR amendment and real external dependent.

**Acceptance:** `cargo metadata`, normalized manifests, target graphs, `cargo package --list` and extracted package archives show one engine, canonical packages and one thin old wrapper with exact dependency; the wrapper is explicitly excluded from cargo-dist. Both command names pass parity/self-exec tests, including signals, logging/current-executable behavior and suppression of deprecation warnings in hidden supervisor/run-worker/retry/reattach/doctor children; wrapper metadata supports same-version 0.6/0.7 releases; doctor recognizes canonical and compatibility checkouts correctly; no GitHub/tap rename, publish, tag or install.

## R5 — Convert skills, prompts, provenance, docs and contracts

**Title:** Convert Taskfleet skills, prompts, provenance and repository contracts

Make new generated commands/source refs use `taskfleet`; rename only Taskfleet-owned skill identities while keeping generic workflow skill names. Migrate Claude/Codex markers and pi schema-v3 provenance by recorded hashes, preserving edited/user-owned files and readable old records. Update AGENTS, README, architecture/security/contribution docs, examples, templates, telemetry prose and `OSS-RELEASE.md`; retain stable `OCTL_*` protocols and classify every residual old identity.

**Acceptance:** full insta review loop; isolated install/update/prune/orphan/provenance tests include unchanged, edited, unmanaged, stale, corrupt and partial old/new ownership while preserving user bytes; generated prompt headings/commands close via the exact canonical run id; telemetry contract id `orchestratectl.worker-telemetry-adapter` and stable `OCTL_*` remain unchanged; skill example extraction validates canonical commands instead of silently finding zero; classified case-insensitive search has no unexplained active old name; no global skill install or distribution mutation.

## R6 — Rebuild crates.io and Shipshape release machinery

**Title:** Rebuild Taskfleet registry and Shipshape release machinery

Replace the hard-coded two-crate workflow with `taskfleet-core` → `taskfleet` → `orchestratectl`, waiting for each exact dependency to become index-visible. Reconcile an existing package/version only after checksum, owners, dependency requirements, metadata and source commit match. Make repository/package identities in the pinned Shipshape 0.10.1 wrapper data-driven while preserving held-tag exact-SHA gates, deterministic version hooks and protocol tests. Document partial-success resume/fix-forward and Homebrew empty-commit repair.

**Acceptance:** package archives and sealed dry-run plan contain exactly three intended crates legs and independent distribution legs; exact pins/version snapshots pass; all release-wrapper protocol tests pass; the current error-text inference that “already exists” means success is removed in favor of checksum/owner/dependency/metadata/source receipts; side-effecting tools are stubbed and credentials absent in pre-cut tests; no local publish, tag, GitHub rename, tap change or install.

## R7 — Prepare cargo-dist and Homebrew topology

**Title:** Prepare Taskfleet cargo-dist and Homebrew topology

Prepare, but do not activate, the canonical `homebrew-taskfleet` repository/token proof and the old tap's atomic migration commit. Configure cargo-dist 0.28.2 for Taskfleet app/assets, one canonical formula and a non-installing old latest-installer stub; regenerate `release.yml`. Prepare exact repository URL/action/secret/runner/release-wrapper substitutions. New Homebrew/shell/archive channels must not ship an `orchestratectl` alias.

**Acceptance:** the only allowed public mutations are creation of the empty canonical `homebrew-taskfleet` repository and one reversible token-proof commit; record receipts and leave the old tap untouched. cargo-dist PR plan machine-checks exactly one distributed app (`taskfleet`), canonical archives/checksums/installer/formula plus one non-installing old installer stub, and zero old wrapper binaries/formulae/assets; only one generated tap target; disposable Homebrew plans reviewed. No old-tap activation, canonical publication, GitHub source-repository rename, release tag or install.

## R8 — Run integrated pre-cut validation

**Title:** Produce immutable integrated Taskfleet pre-cut evidence

On one exact integrated commit run the full Rust/clean-PATH/docs/snapshot/issue gates; both-name command parity; 0.5.1 terminal/active/pending/unknown/config/provenance adoption; optional migration/refusal/rollback cases; disposable Cargo/archive/shell/Homebrew flows; Shipshape contract/audit/plan; and fresh crates/GitHub/tap checks. Record immutable command outputs, hashes and commit identity on the issue.

**Acceptance:** every ADR pre-cut leg passes on the same commit and a committed evidence index records command manifest, toolchain, output hashes, CI/artifact identifiers and exact SHA; side-effecting commands are stubbed/credential-isolated and every mutation destination is sandboxed. Any failure blocks repository rename. R8 evidence authorizes R9 only and expires when R9 changes repository identity; R10 requires a full post-R9 exact-SHA integrated rerun on its actual candidate. R8 performs no additional GitHub/tap mutation, publish, tag, global install or real-state migration; local Homebrew simulations are labelled pre-live, while R10/R11 own hosted formula and cross-tap proof.

## Deferred irreversible work

Do not create spawnable R9/R10 issues before R8 passes. The unscheduled `@rename-taskfleet` parent and `plan.md` retain GitHub rename, canonical 0.6.0 cut and subsequent Homebrew activation (R11), ecosystem convergence and 0.8 compatibility removal. Those are materialized only at their evidence gates.
