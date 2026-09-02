# ADR 0002 — Rename orchestratectl to Taskfleet through a bounded compatibility migration

- **Status:** Proposed — blocked pending a complete required expert panel
- **Date:** 2026-09-02
- **Deciders:** Jari Mustonen (product name and canonical command); migration decision not yet accepted
- **Issue:** `rename-taskfleet`
- **Target releases:** `orchestratectl` bridge 0.5.2; canonical Taskfleet 0.6.0; compatibility removal in 0.8.0, no earlier than 2026-12-01

## Context

The product name **Taskfleet** and canonical command **`taskfleet`** are already decided. The remaining decision is how to move an already-published system without losing durable work or maintaining two products indefinitely.

The current identity crosses several independently durable or cached systems:

- crates.io packages `orchestratectl` and `octl-core`, both published through 0.5.1;
- the `orchestratectl` executable, scripts, generated worker prompts, and bundled skills;
- state, config, logs, and skill provenance under `~/.orchestratectl`, selected by `ORCHESTRATECTL_HOME`, plus `.orchestratectl.toml` repository config and other `ORCHESTRATECTL_*` variables;
- state schema v1, JSON envelopes/contracts, and historical source references;
- `jarimustonen/orchestratectl`, version tags, GitHub Releases, and cargo-dist assets;
- the `jarimustonen/homebrew-orchestratectl` tap and `orchestratectl` formula.

This is not an empty namespace. On 2026-09-02, crates.io reported non-zero downloads for both published packages, GitHub reported recent clones, and Homebrew exposed 0.5.1 from the old release URL. Existing runs may have live supervisors, checked-out worktrees, pending merge transactions, and prompts that still invoke `orchestratectl`. The append-only event log, `applied_seq`, locking, projection ordering, typed terminal outcomes, and OID-based merge recovery remain binding.

External constraints are asymmetric:

- crates.io names and versions are first-come-first-served and permanent. Published code cannot be deleted or overwritten; yanking only prevents new resolution while existing lockfiles continue to work.
- GitHub repository renames redirect ordinary web and Git clone/fetch/push traffic, but action references do not redirect, local remotes should be updated, and reusing the old name breaks redirects.
- cargo-dist 0.28 derives artifacts from package/binary identity. Its `bin-aliases` can create installer aliases (currently symlinks for shell and Homebrew), but aliases are not archive members.
- Homebrew supports formula renames through a tap's `formula_renames.json`, has `brew migrate`, and also supports tap-local aliases. Existing taps and installed formula receipts still require an upgrade test; a repository redirect alone is not a migration proof.

At the time of this decision, authoritative APIs returned 404 for the candidate crates.io package and GitHub repository/tap names. That is evidence of no visible current object, **not a reservation or guarantee of availability**. Availability must be rechecked at the irreversible action.

## Panel completion status

The required five-lens panel did not complete: the model assigned the Rust/crates.io and SemVer/release-engineering lens returned `No response from the model via API`, with no thread id or analysis. The panel workflow defines no retry allowance, and the run policy forbids inferring consensus from surviving roles. This draft therefore cannot be Accepted or merged. It is preserved as work for a fresh decision run that can execute a complete panel.

## Proposed decision (not accepted)

Use a **staged compatibility migration**. Taskfleet becomes the only maintained implementation and public identity at 0.6.0, while bounded aliases and migration readers carry users from the old identity. The transition is prepared by one final old-identity bridge release, then completed through one controlled distribution cut.

The compatibility code must route both names to the same command dispatcher, state resolver, schema implementation, and release source. It must not fork the engine, duplicate the event store, or allow old and new roots to receive concurrent writes.

### 1. Canonical identity and intentionally stable wire data

At canonical release 0.6.0:

- product, repository, primary package, executable, release assets, formula, tap, documentation, and new diagnostics use **Taskfleet** / **`taskfleet`**;
- the canonical Rust packages are `taskfleet` and `taskfleet-core`;
- source directories and Rust package/module references move to Taskfleet names where they are active implementation identity;
- new default user state is `~/.taskfleet`, selected by `TASKFLEET_HOME`;
- new repository config is `.taskfleet.toml`; new public configuration variables use `TASKFLEET_*`;
- generated skills and worker prompts invoke `taskfleet` and new provenance uses `taskfleet:<run-id>/...`.

Do **not** rename neutral persisted vocabulary merely for branding. Fields such as `run_id`, `node_id`, `schema_version`, event kinds, report fields, and envelope shape retain their wire spelling. Historical values containing `orchestratectl` remain readable data. State schema v1 stays v1 if byte compatibility is preserved; a schema bump is allowed only for an actual incompatible schema change, not for a product-name change.

### 2. A bridge release before the public cut

Release 0.5.2 from the old repository and old channels before renaming them. It remains branded as a deprecated transition release and adds only the migration capabilities needed to cross safely:

- recognition and diagnostics for the future Taskfleet root/config/environment names;
- a dry-run and explicit state-home migration command;
- quiescence and split-root detection;
- deprecation guidance that names the future install commands;
- compatibility fixtures proving that 0.5.2 and the candidate Taskfleet build read the same state schema and merge transactions.

The bridge is not the rebrand and does not silently relocate state. Existing 0.5.x users may continue running from `~/.orchestratectl` until they deliberately quiesce and migrate.

### 3. State and config migration is explicit, quiescent, and fail-closed

A migration command moves the complete resolved home as one unit: runs, events, projections, config, logs, supervisor metadata, and installed-skill provenance. It follows these rules:

1. Resolve and display the exact source and destination. Explicit `TASKFLEET_HOME` wins only when the legacy variable is absent. If both old and new variables are set to different normalized paths, fail. Equivalent paths are accepted with a deprecation warning.
2. Refuse when both roots contain managed data. Never merge two event stores automatically and never choose the newer-looking directory.
3. Acquire a migration lock outside both roots and require a quiescent source: no non-terminal run, live supervisor/worker, pending merge transaction, held run lock, or state-writing command. Older binaries do not honor the new lock, so the operator must stop them; process and lock checks fail closed.
4. Validate every run under the normal shared/exclusive lock APIs before movement. Do not rewrite event logs, manifests, run ids, branch names, worktree paths, OIDs, or sequence numbers.
5. On the normal same-filesystem path, rename the whole directory atomically. If an explicitly supported cross-filesystem move is implemented, copy to a temporary destination, fsync files/directories, validate hashes and run projections, then atomically promote it. Otherwise refuse and instruct the operator to choose a same-filesystem destination.
6. Write a versioned migration marker with source, destination, timestamp, and pre/post verification result. Retain a read-only backup until the canonical release and post-migration verification pass; never treat that backup as a second readable root.
7. After migration, all command aliases resolve the canonical root. If a legacy root is later detected beside it, refuse writes and explain recovery.

The Taskfleet resolver permanently detects a populated legacy default root when the canonical root is absent and reports the migration path instead of silently creating an empty `~/.taskfleet`. This permanent guard prevents old work from appearing lost; it is not continued support for writing the old layout.

Repository configuration follows the same no-guess rule: during compatibility, read `.taskfleet.toml`, or fall back to `.orchestratectl.toml` with a warning. If both exist, require semantic equality or an explicit operator choice; never layer them silently. External repositories are converted only in their own post-release worktrees.

### 4. Compatibility is broad enough for in-flight work but has a fixed sunset

The canonical `taskfleet` package and installers also provide an `orchestratectl` executable alias through 0.7.x. Both names enter the same binary/library dispatcher and therefore share state, locking, behavior, output schemas, and version. Cargo builds include the compatibility target; cargo-dist installer aliases cover installer layouts, and archive contents are tested explicitly rather than assumed from `bin-aliases`.

The old crates receive bounded compatibility releases:

- `orchestratectl` is a thin deprecated wrapper over the canonical CLI entry point;
- `octl-core` is a deprecated re-export of `taskfleet-core` where source compatibility is practical;
- both may be released alongside 0.6.x and 0.7.x solely for migration and receive no independent features;
- no old-name crate release is made at 0.8.0 or later.

This leaves permanent registry artifacts, as crates.io requires, but not permanent dual implementations.

Old `ORCHESTRATECTL_HOME`, `ORCHESTRATECTL_PROFILE`, `ORCHESTRATECTL_HARNESS`, and `ORCHESTRATECTL_LOG` are aliases through 0.7.x. A differing old/new pair is an error; an old-only value warns. Worker-control variables and the worker telemetry contract receive equivalent dual-read/new-write treatment through 0.7.x, with exact names fixed in the implementation inventory. The old command and environment aliases are removed in 0.8.0, **not before 2026-12-01** and only after the removal gates below pass.

Old state/schema readers, migration markers, historical strings, and legacy-root detection do not expire. Data compatibility is not the same thing as a public command alias.

### 5. GitHub and distribution cross in one controlled maintenance window

After bridge adoption and after the canonical implementation is green on the exact commit:

1. Rename `jarimustonen/orchestratectl` to `jarimustonen/taskfleet`; never reuse the old repository name.
2. Immediately update and verify local/CI remotes, repository metadata, secrets, badges, action references, release wrapper identity checks, and all exact URLs. GitHub redirects are a fallback, not the configured steady state.
3. Rename `jarimustonen/homebrew-orchestratectl` to `jarimustonen/homebrew-taskfleet` (subject to the final availability recheck), preserve its history, add `formula_renames.json` mapping `orchestratectl` to `taskfleet`, and configure cargo-dist to publish only the Taskfleet formula. Do not maintain two generated formula repositories.
4. Regenerate, never hand-edit, cargo-dist's workflow after package/binary/tap changes.
5. Push the canonical 0.6.0 tag only through the repository's exact-SHA, green-main, resumable Shipshape wrapper after that wrapper and `OSS-RELEASE.md` have been migrated and revalidated.
6. Publish in dependency order: `taskfleet-core`, `taskfleet`, then bounded legacy wrappers as declared by the sealed release plan. GitHub Release and Homebrew remain CI-delegated legs of the same tag.

If a same-tag four-crate transaction cannot be made safely resumable, publish the canonical core/CLI first and wrappers second from the same immutable source commit with distinct journaled steps. A wrapper failure must not cause a second canonical version or retagging.

### 6. Compatibility removal gate

Remove command/config/environment/package compatibility in 0.8.0 only when all are true:

- the date is on or after 2026-12-01;
- canonical crates, release assets, shell installer, and Homebrew formula have been verified from clean environments;
- an old Homebrew installation has successfully followed the documented formula/tap migration in an isolated Homebrew environment;
- a 0.5.1 fixture and a bridge fixture have migrated with byte/invariant verification and completed `run show`, `event tail`, `run wait`, `run merge`, and cleanup under `taskfleet`;
- no non-terminal run or pending merge transaction remains in a legacy root on maintained machines;
- installed bundled skills have been refreshed and all maintained repositories have zero active old-name invocations outside explicit compatibility/history fixtures;
- the post-release cross-repository convergence phase is complete, including discovered Homebase/intake/Haapa owners;
- release rollback/fix-forward runbooks and a retained migration backup have been exercised.

If a gate fails, defer 0.8.0 rather than silently extending an undocumented alias. The compatibility contract remains explicit until the gate passes.

## Ordered migration phases

| Phase | Purpose | Exit gate |
|---|---|---|
| **0. Inventory and fixtures** | Freeze the old/new identity inventory; capture 0.5.1 state, config, active-run, skill, Cargo, archive, and Homebrew fixtures. Recheck candidate names without claiming reservation. | Reproducible fixtures and zero unidentified identity-bearing write paths. |
| **1. Bridge 0.5.2** | Add migration diagnostics/tooling under the old identity; preserve current behavior. | Full green gate, isolated migration drills, exact-SHA CI, bridge published and verified on all old channels. |
| **2. Canonical internals** | Rename packages/modules/CLI entry point; keep one dispatcher; add bounded old wrappers and dual-read/new-write config handling. Preserve wire schema. | Both executable names pass the same contract suite against one isolated root; no duplicate engine/store implementation. |
| **3. State migration proof** | Exercise quiescence, atomic move, split-root refusal, backup, rollback-before-write, old fixture reads, and merge recovery. | Fault-injection tests pass and hashes/sequences/OIDs remain unchanged. |
| **4. Distribution preparation** | Update contract, publish workflow, Shipshape hook/wrapper, cargo-dist config/workflow, docs, crate order, asset names, and tap migration metadata. | Dry runs and clean install/upgrade tests pass; exact irreversible plan is sealed. |
| **5. GitHub/tap rename window** | Rename the repositories and update exact URLs/remotes/secrets/references. | New canonical URLs work; expected redirects work; no workflow/action/tap step depends on an unverified redirect. |
| **6. Canonical 0.6.0 release** | Cut one gated release and publish/verify canonical crates, binaries, assets, installers, and formula plus bounded wrappers. | Shipshape verification reconciles every declared registry; smoke tests pass from clean homes. |
| **7. Ecosystem convergence** | Search dependent repositories and modify each through its owning worktree. Discover Homebase/intake/Haapa ownership before edits. | Maintained-source scan is clean except intentional migration/history references; machine convergence policy passes. |
| **8. Compatibility removal** | At 0.8.0 and after the date/gates above, remove active aliases and legacy wrapper packages from the workspace. Keep historical readers/guards. | Old invocation fails with a clear migration message or is absent as documented; canonical paths remain green. |

## Compatibility and deprecation matrix

| Surface | 0.5.2 bridge | Taskfleet 0.6.x–0.7.x | Taskfleet ≥0.8.0 |
|---|---|---|---|
| Product/docs | orchestratectl, announcing migration | Taskfleet canonical; old name marked deprecated | Taskfleet only outside history/migration docs |
| Command | `orchestratectl` | `taskfleet` canonical; `orchestratectl` same-dispatch alias with warning | alias removed |
| crates.io CLI | `orchestratectl` | `taskfleet` canonical; bounded thin `orchestratectl` wrapper | no new old-name releases |
| crates.io core | `octl-core` | `taskfleet-core` canonical; bounded re-export wrapper | no new old-name releases |
| Default home | `~/.orchestratectl` | explicit migration to `~/.taskfleet`; one writable root | `~/.taskfleet`; permanent legacy-root safety detection |
| Home env | `ORCHESTRATECTL_HOME` plus migration awareness | `TASKFLEET_HOME` canonical; old alias warns; conflicts fail | old alias removed; migration marker/guard retained |
| Other public env | old names | `TASKFLEET_*` canonical; old aliases warn; conflicts fail | old aliases removed |
| Repo config | `.orchestratectl.toml` | `.taskfleet.toml` canonical; old fallback warns; dual conflict fails | old fallback removed after cross-repo gate |
| State/events/JSON | schema v1 | neutral wire names unchanged; historical branded values readable | unchanged readers remain |
| Skills/prompts | old installed copies | new Taskfleet catalog; provenance-aware prune/update; old commands work during refresh | Taskfleet catalog only; modified user files preserved |
| GitHub | old repository | renamed repository; old URL redirects but is not configured | new URL only; old name never reused |
| cargo-dist/assets | old app/assets | Taskfleet app/assets; installer alias tested; generated workflow | Taskfleet only |
| Homebrew | old tap/formula | renamed tap + `taskfleet`; formula rename/migration and command alias | Taskfleet formula only; rename metadata retained |

## Irreversible-action gates

No irreversible step is inferred from a previous success.

1. **Bridge tag:** clean synchronized main, complete repository green gate, migration fixtures green, sealed release plan, exact-SHA main CI green, then the existing held-tag/resume protocol.
2. **State move:** exact paths displayed, conflict/symlink/path validation, external migration lock held, all runs quiescent, no pending merge, source validated and backed up. Any uncertainty refuses the move.
3. **First canonical crate publish:** crates.io API and `cargo search/info` rechecked immediately; ownership/token and package contents verified by dry-run; package/version/core pin/tag match; exact-SHA CI green. A 404 is not treated as a reservation.
4. **Repository/tap rename:** candidate names rechecked; redirects and action-reference exception understood; all exact URL substitutions prepared; local remotes and CI secrets verified immediately afterward. Never recreate the old repository names.
5. **Canonical tag:** GitHub and tap already canonical; generated cargo-dist plan lists only intended Taskfleet artifacts/formula; Shipshape plan lists every crate leg in dependency order; no direct local publish or manual tag push.
6. **Alias removal:** every compatibility-removal gate above passes. Otherwise postpone the breaking release.

## Rollback boundaries

- **Before the bridge tag:** ordinary code rollback; no public migration contract exists.
- **After the bridge tag, before state movement:** bridge artifacts are permanent. Fix forward with another old-name patch if needed; do not delete/retag.
- **During state migration, before the first canonical write:** restore the verified backup only while quiescent and only through the migration tool. The migration marker records this eligibility.
- **After any canonical-root event append/config/provenance write:** do not move the backup back over newer state. The rollback boundary has crossed; recover or fix forward in the canonical root using event-store repair rules.
- **After a GitHub/tap rename:** prefer fix-forward. Renaming back is not a routine rollback because it changes redirects and cached URLs.
- **After the first canonical crate or tag is published:** identity is irreversible. Yank only a broken version, never reuse a version/tag, and publish a corrected Taskfleet patch through the same gate.
- **After old aliases are removed:** restore them only by a new explicit compatibility decision and release; do not silently reintroduce split behavior.

## Exact verification criteria

The migration is complete only when all of the following are recorded against immutable commits/releases:

1. `cargo metadata` identifies canonical packages `taskfleet` and `taskfleet-core`; exact internal version pins match the workspace version. Legacy packages, while present, contain only wrappers/re-exports and deprecation metadata.
2. `taskfleet version --output json` names the build/version/schema correctly. During compatibility, invoking the same build as `orchestratectl` produces equivalent machine payloads except an allowed structured deprecation warning.
3. Contract tests run every public noun/verb needed by bundled skills under both command names against one temporary `TASKFLEET_HOME`; concurrent aliases cannot produce split roots or sequence duplication.
4. Migration tests start from byte-captured 0.5.1 and 0.5.2 homes. After migration, hashes of event logs and semantic manifests match, `applied_seq` equals the durable log state, run/node IDs and OIDs are unchanged, and split-root/live-run/pending-merge/fault-injection cases refuse safely.
5. An in-flight old prompt can finish through the old alias after the home migration, and its `run merge` transaction is recovered/recorded exactly once before teardown.
6. Config precedence tests cover new-only, old-only, equivalent dual values, conflicting dual values, neither value, dual repository files, and custom paths. No case silently selects between differing roots.
7. Skill installation tests prove the Taskfleet catalog, byte/provenance tracking, safe pruning, preservation of user-edited copies, and no global mutation during repository tests.
8. The standard Rust green gate, documentation gate, version snapshots, clean-PATH tool-sensitive tests, package dry-runs, and cargo-dist PR plan all pass on the exact canonical commit.
9. crates.io shows the intended canonical versions owned by the expected account; `cargo install taskfleet` in a disposable Cargo home runs `taskfleet` and, during compatibility, the old alias. Existing lockfile fixtures using old core continue to build for the declared window.
10. GitHub canonical clone/fetch/push and Release URLs work. Old ordinary repository URLs redirect, no maintained action reference relies on that redirect, and the old repository name has not been reused.
11. Release assets, checksums, installer script, manifest, and formula are Taskfleet-named and install a binary whose embedded commit equals the tagged commit.
12. In clean and old-upgrade disposable Homebrew environments, `brew install jarimustonen/taskfleet/taskfleet`, formula rename/migration, `brew upgrade`, `brew uninstall`, and command aliases behave as documented without two owned formulas fighting over the same binary.
13. Shipshape verification reports every declared canonical and compatibility registry leg reconciled; no direct local `cargo publish` occurred.
14. A post-release maintained-source search records every remaining `orchestratectl`, `octl`, `.orchestratectl`, and `ORCHESTRATECTL_*` occurrence as either intentional compatibility/history or a separately owned convergence item. Homebase/intake/Haapa ownership is evidenced, not assumed.
15. Before 0.8.0, the compatibility-removal gate is rerun and attached to the release record.

## Consequences

### Positive

- Existing work remains visible and migrates without rewriting the event history or bypassing merge safety.
- In-flight prompts and automation have a bounded route across the rename.
- Taskfleet becomes canonical in one release, while old crates become frozen compatibility artifacts rather than a second product.
- Registry, GitHub, cargo-dist, and Homebrew changes are ordered around their actual irreversible boundaries.
- The end state has one engine, one writable root, one generated distribution path, and one maintained identity.

### Negative / accepted

- Two transition releases and temporary wrapper packages add release and test complexity.
- The home move requires a deliberate quiescent maintenance step; live runs delay migration.
- Old crates and historical strings remain publicly visible forever because registry/history integrity is more important than cosmetic erasure.
- A short controlled interval exists after the GitHub rename and before canonical package verification. Release preparation must minimize it and provide fix-forward instructions.
- Users who skip the bridge need a documented direct migration path from 0.5.1; Taskfleet must detect their old root rather than appearing empty.

## Rejected alternatives

### Hard cut: rename everything at once with no compatibility surface

Rejected. It would make existing state appear missing under a new default home, strand in-flight workers whose prompts invoke the old command, and turn cached package/tap URLs into simultaneous failures. GitHub redirects do not cover crates.io, executables, environment variables, action references, or Homebrew receipts. A hard cut is smaller in source but unsafe at the durable and automation boundaries.

### Packaging-only rebrand: change public packages/command but retain old storage and vocabulary indefinitely

Rejected. It avoids immediate migration work by making `~/.orchestratectl`, old variables, old internal packages, and old operational scripts permanent. That creates the indefinitely split identity the rename is meant to end and makes every future explanation distinguish brand from implementation. Neutral wire fields and historical values remain for compatibility, but active defaults and maintained internals move.

### Permanent dual publication and dual writable homes

Rejected. Publishing both complete CLIs indefinitely doubles release/security obligations, while reading and writing both roots introduces ambiguous ownership and event-log divergence. Compatibility packages are thin, bounded, and share one implementation; roots are never merged automatically or written concurrently.

### Automatic first-run state move

Rejected. A first-run heuristic cannot prove that old supervisors or pre-bridge binaries have stopped writing, and a partially populated destination is ambiguous. Migration is explicit, quiescent, validated, and reversible only before the first canonical write.

## References

- `issues/rename-taskfleet/item.md`
- `issues/rename-taskfleet/plan.md`
- `docs/decisions/0001-thin-supervisor-vs-harden.md`
- `AGENTS.md`, state-integrity invariants
- `Cargo.toml`, `crates/octl-{cli,core}/Cargo.toml`
- `crates/octl-cli/src/home.rs`, `crates/octl-core/src/schema.rs`
- `OSS-RELEASE.md`, `dist-workspace.toml`
- `.github/workflows/publish-crates.yml`, `.github/workflows/release.yml`
- `scripts/shipshape-release.sh`
- GitHub Docs, “Renaming a repository” (read 2026-09-02)
- Cargo Book, “Publishing on crates.io” and manifest target documentation (read 2026-09-02)
- Homebrew documentation and `brew migrate` help/source (read 2026-09-02)
- cargo-dist 0.28 configuration reference, `bin-aliases` and tap settings (read 2026-09-02)
- Required expert panel: thread map and synthesis in `history/2026-09-02-panel-taskfleet-rename.md` (gitignored working artifact)
