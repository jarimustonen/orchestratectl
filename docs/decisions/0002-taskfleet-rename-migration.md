# ADR 0002 — Rename orchestratectl to Taskfleet through bounded compatibility

- **Status:** Accepted
- **Date:** 2026-09-02
- **Deciders:** Jari Mustonen (product identity); five-lens technical panel and this worker (migration contract)
- **Issue:** `rename-taskfleet`
- **Target releases:** Taskfleet 0.6.0; compatibility removal in 0.8.0, no earlier than 2026-12-01

## Context

The product name **Taskfleet** and canonical command **`taskfleet`** are fixed. The current identity is already durable and published:

- crates.io packages `orchestratectl` and `octl-core` exist through 0.5.1;
- the executable, generated prompts, installed skills, scripts, and diagnostics say `orchestratectl`;
- state defaults to `~/.orchestratectl`, public configuration uses `ORCHESTRATECTL_*`, and repository selection uses `.orchestratectl.toml`;
- append-only state can contain live supervisors, pending OID-based merge transactions, and generated commands which execute later;
- GitHub, cargo-dist releases, and the `jarimustonen/homebrew-orchestratectl` tap are independently cached identities.

On 2026-09-02 crates.io reported 327 downloads for `orchestratectl` and 410 for `octl-core`. The only reported reverse dependency of `octl-core` was `orchestratectl` itself. The candidate `taskfleet` and `taskfleet-core` package endpoints returned 404. These observations show real use and no visible candidate package; they do not reserve either name.

crates.io identities and versions are permanent. GitHub repository redirects do not cover every consumer, including action references. Homebrew distinguishes same-tap formula renames from cross-tap migrations. cargo-dist does not make one alias mechanism uniform across Cargo installs, Homebrew, shell installers, and raw archives.

State integrity remains binding: event bytes, `applied_seq`, locks, projections, run/node ids, OIDs, pending merges, and typed terminal outcomes cannot be rewritten or split merely to improve branding.

## Panel evidence and reconciliation

Five full independent positions were completed for state architecture, Rust/crates.io release engineering, CLI automation compatibility, GitHub/Homebrew/cargo-dist distribution, and maintainability/rollback. DeepSeek's first maintainability call failed once with HTTP 503; the permitted fresh replacement position used `gpt-5.6-sol` and completed. The other roles used `gemini-3.1-pro-preview`, `gpt-5.6-sol`, `claude-fable-5`, and `claude-opus-5`. All five then cross-reviewed a provisional synthesis. The synthesis and thread map are recorded in the gitignored `history/2026-09-02-panel-taskfleet-rename-retry.md`.

The panel agreed on bounded compatibility, one maintained implementation, unchanged wire data, direct 0.5.1 support, fail-closed root/config conflicts, a new canonical tap plus a permanent old migration stub, and fix-forward release boundaries. It disagreed on bridge publication, physical home movement, old crate cadence, and command aliases in binary channels. This ADR resolves those disagreements by preferring the smallest path that preserves data and supported automation:

- no separate 0.5.2 bridge: Taskfleet 0.6.0 directly adopts or migrates 0.5.1 state;
- existing populated legacy homes are adopted in place through 0.7.x; physical movement is explicit and optional during that window, so no symlink or stale-writer routing is needed;
- only the old CLI crate receives a compatibility wrapper; no `octl-core` wrapper is published without new external-dependent evidence;
- the old executable remains supported through the Cargo wrapper, not through new Homebrew/shell/archive artifacts whose ownership and link behavior conflicts with existing installations;
- the old tap becomes static migration metadata; only the new tap receives generated formulae.

## Decision

Use a **bounded staged compatibility migration**. Taskfleet 0.6.0 establishes the canonical identity while directly supporting existing 0.5.1 state and selected old inputs. Compatibility is deliberately channel-specific and ends for newly produced artifacts at 0.8.0 after the gates below.

### 1. Canonical identity and stable protocol

At 0.6.0:

- product, repository, primary package, executable, release assets, formula, documentation, and new diagnostics use **Taskfleet** / **`taskfleet`**;
- canonical Rust packages are `taskfleet` and `taskfleet-core`;
- fresh state defaults to `~/.taskfleet`, selected by `TASKFLEET_HOME`;
- repository selection uses `.taskfleet.toml` and branded public variables use `TASKFLEET_*`;
- generated skills and prompts invoke `taskfleet`; new provenance uses `taskfleet:<run-id>/...`.

Do not rename neutral persisted or automation protocol vocabulary. State schema v1, event kinds, JSON envelope fields, `run_id`, `node_id`, report fields, and historical values remain unchanged. Existing `OCTL_*` worker, notification, readiness, and test/control contracts are inventoried and retained under their existing spelling unless a separate decision proves they are product branding rather than protocol. A product rename alone never bumps the state schema.

### 2. One implementation and bounded old CLI package

The canonical `taskfleet` package exposes the minimal linkable CLI dispatcher used by its `taskfleet` binary. The deprecated `orchestratectl` package contains only an `orchestratectl` entry point and deprecation glue linked to that dispatcher. It never shells out to `taskfleet` from `PATH`, duplicates the engine, or owns a second state resolver.

For every supported Taskfleet 0.6.x and 0.7.x release, publish a same-version `orchestratectl` wrapper from the same immutable source commit with an exact canonical dependency. This gives Cargo users a normal same-package upgrade path and keeps generated old commands functional without two Cargo packages competing to own one binary. Starting at 0.8.0, no new old-name wrapper is published.

Do not publish an `octl-core` re-export wrapper now. crates.io showed no external reverse dependency. Existing `octl-core` versions remain available for historical lockfiles. Reconsider only if implementation inventory finds a real external source consumer whose migration requires a bounded re-export.

### 3. Deterministic state-home adoption and optional movement

During 0.6.x–0.7.x, every command uses one centralized resolver:

1. If `TASKFLEET_HOME` and `ORCHESTRATECTL_HOME` normalize to different paths, fail. If equivalent, use the one path and warn.
2. A new-only explicit home is used. An old-only explicit home is used with a warning.
3. With no explicit home:
   - populated canonical root only: use `~/.taskfleet`;
   - populated legacy root only: adopt `~/.orchestratectl` in place and warn;
   - neither populated: create/use `~/.taskfleet`;
   - both contain managed data: refuse state reads that require one truth and refuse all writes. Never merge or choose by timestamp.

Adoption in place is bounded compatibility, not the end state. It avoids moving state during the public identity cut and lets old 0.5.1 binaries and in-flight prompts finish against the same legacy root. Taskfleet must not introduce state writes during this window that make the supported 0.5.1 fixture unreadable without an explicit schema decision.

The optional `state migrate` operation moves a resolved legacy home only when the operator chooses:

- display exact normalized source/destination and require destination absent;
- acquire an external migration lock and require quiescence: no non-terminal run, live supervisor/worker, pending merge transaction, held run lock, or state-writing command;
- validate every run through normal lock/reducer APIs;
- support only a same-filesystem atomic whole-directory rename initially; refuse cross-filesystem moves rather than implementing an unproven copy protocol;
- write the migration receipt outside the moved root, then validate the destination without rewriting events, projections, ids, branches, paths, OIDs, or sequence numbers;
- leave no symlink or writable alias at the old path. A stale old process may recreate it, but permanent dual-root detection makes Taskfleet fail closed instead of routing an old writer into the canonical store.

A quiescent rename-back is permitted only before the **first canonical write**, meaning any event append, projection repair, config write, skill-provenance write, supervisor metadata write, or migration-state mutation in the destination. After that boundary, state rollback is forbidden; repair or fix forward in the canonical root.

At 0.8.0, a still-populated legacy home must be explicitly migrated before state-writing commands proceed. Legacy-root and split-root detection remain permanent.

### 4. Public config and automation compatibility

Through 0.7.x:

- `TASKFLEET_HOME`, `TASKFLEET_PROFILE`, `TASKFLEET_HARNESS`, and `TASKFLEET_LOG` are canonical;
- old `ORCHESTRATECTL_*` counterparts remain input aliases: old-only warns, equal dual values warn, differing dual values fail;
- `.taskfleet.toml` is canonical; old-only `.orchestratectl.toml` falls back with a warning; differing dual files fail rather than layer;
- stable `OCTL_*` worker and notify protocol variables retain their spelling and behavior;
- machine stdout, JSON, JSONL, exit codes, and event streams are unchanged by invocation identity. Deprecation warnings are stderr-only, at most once per process, and never interleaved into JSONL.

Starting at 0.8.0, old branded variables and repository config no longer supply values. Their presence remains an actionable error whenever ignoring them could select another home/config. Historical readers and safety detection do not expire.

### 5. Command compatibility is channel-specific

The canonical binary channels ship `taskfleet`. They do not install a new `orchestratectl` alias:

- **Cargo:** `cargo install orchestratectl` receives the bounded same-dispatch wrapper through 0.7.x; `cargo install taskfleet` installs only `taskfleet`.
- **Homebrew:** the Taskfleet formula installs `taskfleet` only. Native tap/formula migration handles package identity, but no old executable symlink is added that could collide with a linked old keg.
- **Shell installer and raw archives:** new Taskfleet artifacts install/contain `taskfleet` only. Existing old binaries are not silently removed. Operators quiesce old work, refresh skills/automation, and then remove the old binary explicitly.

Thus an in-flight old prompt is completed before replacing an old Homebrew/shell/archive installation or before physical state movement. Cargo-wrapper users may exercise the old command throughout the window. This explicit channel break is safer than a nominal alias whose ownership, archive coverage, or PATH precedence differs by installer.

The old `releases/latest/download/orchestratectl-installer.sh` URL would otherwise fail when 0.6.0 becomes latest. Ship a small compatibility stub at that asset name through 0.7.x which prints the canonical installer URL and exits non-zero; it never installs or mutates state.

### 6. GitHub, cargo-dist, and Homebrew topology

- Rename `jarimustonen/orchestratectl` to `jarimustonen/taskfleet` before the canonical tag. Never recreate/reuse the old repository name. Update remotes, exact URLs, action references, badges, secrets, and identity checks instead of configuring redirects as steady state.
- Create `jarimustonen/homebrew-taskfleet` as the sole generated formula destination.
- Retain `jarimustonen/homebrew-orchestratectl` permanently as a static migration stub. After the canonical formula is live and verified, atomically delete `Formula/orchestratectl.rb` and add `tap_migrations.json` mapping the old formula to the canonical full formula identity. Add `formula_renames.json` in the new tap only if the isolated Homebrew drill proves recursive rename metadata is required.
- Before the canonical tag, verify `HOMEBREW_TAP_TOKEN` can write the new tap, the self-hosted macOS runner accepts jobs under the renamed repository, and cargo-dist 0.28.2's PR plan lists exactly the intended Taskfleet artifacts, stub installer, and one canonical formula with zero unintended old artifacts.
- Keep cargo-dist pinned at 0.28.2 for the rename. Regenerate its workflow; do not hand-edit generated release identity.

### 7. Release is a resumable saga

A pushed canonical tag starts independently completing crates.io and cargo-dist/GitHub/Homebrew legs. The release is not atomic and prose must not imply cross-workflow chronology.

Within crates.io, publish and reconcile:

1. `taskfleet-core`;
2. `taskfleet` after the exact core is index-visible;
3. `orchestratectl` after the exact Taskfleet package is index-visible.

Every step queries the registry and verifies version, checksum, owners, dependency requirements, metadata, and source commit before treating an existing artifact as success. GitHub Release and Homebrew are separate receipt-bearing legs which may finish earlier or later. Shipshape verification declares completion only after every configured registry reconciles.

Do not publish locally, retag, reuse a version, or infer success from Cargo's “already exists” text. If any permanent leg succeeds and another fails, resume the missing idempotent leg from the same commit where possible or fix forward with a new patch. The generated Homebrew job's empty-commit failure mode needs a documented manual formula-commit repair path.

### 8. Compatibility sunset

Taskfleet 0.8.0 may stop newly shipping, publishing, testing, and supporting old command/config/environment/package compatibility only when all are true:

- date is on or after 2026-12-01 and at least 90 days after verified 0.6.0 availability;
- 0.7.0 has been publicly available for at least 30 days and carried the announced removal notice;
- clean Cargo, archive, shell, and Homebrew Taskfleet installs pass;
- an old Homebrew receipt/tap installation completes the documented migration in an isolated prefix;
- 0.5.1 legacy-root adoption, explicit same-filesystem migration, split-root refusal, and rollback-before-write fixtures pass;
- bundled skills and generated prompts use `taskfleet` and maintained reachable integrations have converged; unreachable machines are recorded as unverified, not treated as proof or an indefinite veto;
- no known supported integration still depends on active aliases.

Previously published crates and installed 0.6/0.7 binaries remain available and may continue to run. Permanent history/safety surfaces remain: state/schema readers, split-root and removed-input detection, GitHub redirects, old tap migration metadata, published releases, and registry artifacts.

## Compatibility matrix

| Surface | orchestratectl 0.5.1 | Taskfleet 0.6.x–0.7.x | Fresh Taskfleet ≥0.8.0 |
|---|---|---|---|
| Product/docs | orchestratectl | Taskfleet canonical; old name only in migration/deprecation | Taskfleet outside history/safety docs |
| Canonical command | `orchestratectl` | `taskfleet` | `taskfleet` |
| Old command via Cargo | current package | same-dispatch `orchestratectl` wrapper, same-version release | no new wrapper; historical versions remain |
| Old command via new Homebrew/shell/archive | n/a | not shipped; old installation must finish work before replacement | not shipped |
| CLI crate | `orchestratectl` | `taskfleet` canonical + bounded CLI wrapper | `taskfleet` only for new releases |
| Core crate | `octl-core` | `taskfleet-core`; no old wrapper without new dependent evidence | `taskfleet-core` |
| Fresh default home | `~/.orchestratectl` | `~/.taskfleet` | `~/.taskfleet` |
| Existing legacy home | writable | adopted in place or explicitly migrated | explicit migration required; permanent detection |
| Home env/config | `ORCHESTRATECTL_*`, `.orchestratectl.toml` | `TASKFLEET_*` / `.taskfleet.toml` canonical; old aliases/fallback warn; conflicts fail | old inputs do not supply values and trigger actionable errors where safety-relevant |
| `OCTL_*` protocols | existing wire contract | unchanged | unchanged unless separately versioned |
| State/events/JSON | schema v1 | unchanged wire and historical bytes | historical readers remain |
| Skills/prompts | old installed commands | new Taskfleet catalog; old unmanaged copies must be refreshed before alias-free channel replacement | Taskfleet catalog |
| GitHub | old repository | renamed canonical repository; old redirect retained, action refs updated | canonical repository; old name never reused |
| Releases/assets | old names | Taskfleet assets; old latest-installer URL is a non-installing migration stub | Taskfleet assets; stub retirement separately evidenced |
| Homebrew | old generated tap/formula | new generated tap/formula; old tap static cross-tap migration stub | same; migration metadata permanent |

## Ordered phases and irreversible gates

| Phase | Work | Exit / irreversible gate |
|---|---|---|
| **0. Inventory** | Freeze all identity-bearing paths, variables, commands, package consumers, URLs, skills, and fixtures. | No unidentified writer or distribution identity; candidate names rechecked but not considered reserved. |
| **1. Shared dispatcher and resolver** | Introduce linkable dispatcher; centralized dual-name config and bounded legacy-home adoption. | 0.5.1 fixtures and both-name stdout/exit-code contracts green; no source rename yet. |
| **2. Migration proof** | Implement optional quiescent same-filesystem move, permanent split-root detection, and rollback boundary. | Event bytes/sequences/OIDs unchanged; live/split/cross-device cases refuse safely. |
| **3. Canonical packages and wrapper** | Rename active packages/binary; add only the thin old CLI wrapper; update skills/docs/contracts. | Package archives show one engine; exact pins and old/new command behavior verified. |
| **4. Distribution preparation** | New tap/token, old-tap migration commit staged, GitHub/url updates prepared, cargo-dist regenerated and planned. | Runner/token/plan/Homebrew disposable drills green; exact action list sealed. |
| **5. GitHub rename** | Rename repository and immediately update remotes/settings/references. | Canonical clone/push and CI work; old ordinary URLs redirect; action refs use canonical identity. Treat as fix-forward. |
| **6. Canonical 0.6.0 tag** | Use only the exact-SHA green-main Shipshape wrapper. | **Irreversible:** canonical names rechecked, crates and independent distribution legs reconciled; never retag/reuse. |
| **7. Homebrew migration activation** | After formula verification, replace old formula with static cross-tap migration metadata. | Old receipt/install/migrate/upgrade/uninstall drill passes; revert metadata only if needed. |
| **8. Ecosystem convergence** | One owning worktree per maintained dependent repository; discover Homebase/intake/Haapa ownership. | Reachable maintained sources use canonical identity; unknown/unreachable cases recorded. |
| **9. Compatibility removal** | After date/version/evidence gates, remove active aliases and wrapper publication in 0.8.0. | Permanent readers/guards/metadata retained; fresh installs are Taskfleet-only. |

## Irreversible-action gates

1. **First canonical crate:** recheck names immediately; inspect archives, exact pins, owner/token, and immutable commit; exact-SHA CI green.
2. **GitHub rename:** all exact substitutions and action references prepared; runner/secrets understood; never reuse old name.
3. **Canonical tag:** cargo-dist plan and Shipshape plan list only intended legs; use held-tag/resume wrapper, never manual publish/tag.
4. **Homebrew migration activation:** canonical formula already live; old formula deletion plus migration metadata is one reviewed commit; disposable old-upgrade drill passes.
5. **State move:** exact paths, quiescence, locks, destination absence, same filesystem, source validation. Any uncertainty refuses.
6. **Compatibility removal:** every sunset criterion passes; otherwise defer 0.8.0.

## Rollback boundaries

- Before GitHub rename/publication: normal code rollback.
- After GitHub rename: fix forward at canonical identity; renaming back is not routine rollback.
- After `taskfleet-core` or any canonical version publishes: that name/version is permanent; resume or publish a new patch, yanking only a materially broken version.
- Before a physical home move: continue using the sole legacy root.
- After atomic move but before first canonical write: quiescent rename-back is allowed.
- After first canonical write: never overlay/rename back stale state; repair or fix forward in canonical root.
- Homebrew migration metadata can be reverted quickly, but published crates/releases are not rolled back with it.
- After compatibility removal: reintroduction requires a new explicit decision and release.

## Verification

Before the canonical tag, record against immutable commits:

1. `cargo metadata` and package archives identify `taskfleet-core`, `taskfleet`, and the implementation-free `orchestratectl` wrapper with exact dependencies.
2. Every public command under canonical and wrapper names has identical stdout, JSON/JSONL, and exit codes; deprecation is stderr-only and stream-safe.
3. 0.5.1 completed, non-terminal, and pending-merge fixtures adopt in place without byte changes; active work finishes before optional movement.
4. Migration preserves event hashes, `applied_seq`, ids, branches, OIDs, and merge recovery; live, dual-root, destination-present, and cross-device cases refuse.
5. Config precedence covers new-only, old-only, equivalent dual, conflicting dual, neither, custom homes, and dual repository files.
6. Stable `OCTL_*` notify/worker contracts are unchanged; generated skills/prompts use `taskfleet`; user-edited installed skills are preserved.
7. Full Rust green gate, docs gate, clean-PATH tests, snapshots, package dry-runs, and cargo-dist PR plan pass.
8. crates.io receipts verify names, versions, owners, checksums, metadata, and dependencies; no local publish occurs.
9. Canonical GitHub clone/fetch/push/release URLs and expected old redirects work; maintained action refs do not rely on redirects.
10. Clean Taskfleet installs work in disposable Cargo, shell, archive, and Homebrew homes; embedded commit equals tag.
11. Old Homebrew receipt/tap migration, upgrade, direct canonical install, uninstall, and old-tap command resolution work without duplicate formula ownership.
12. `releases/latest/download/orchestratectl-installer.sh` produces only the documented migration message and performs no installation.
13. Shipshape reconciles every configured registry leg, including manual Homebrew repair if its generated push was not retry-safe.
14. Maintained-source search classifies every residual old identity as compatibility, safety, history, or separately owned convergence work.

## Consequences

### Positive

- New users receive a coherent Taskfleet identity immediately.
- Existing durable state is not moved during the release cut and is never rewritten.
- Cargo automation has a bounded old command path without duplicate implementations or binary ownership conflicts.
- Homebrew uses its native cross-tap migration mechanism while preserving the old identity against squatting.
- The final maintained system has one engine, one canonical state path for migrated/fresh users, one generated tap, and one public product.

### Negative / accepted

- Existing users can temporarily operate from a legacy-named home through 0.7.x.
- Homebrew, shell, and archive users do not receive a new old-command alias; they must finish old work and refresh automation before replacement.
- Three crates publish during the compatibility window, and release legs can partially complete.
- Registry artifacts, old URLs, migration metadata, historical strings, and safety readers remain visible permanently.

## Rejected alternatives

### Hard cut

Rejected because changing the command, home, inputs, packages, and distribution simultaneously would make existing state appear missing and break persisted prompts/scripts without a migration interval.

### Packaging-only rebrand

Rejected as an end state because retaining old packages, active defaults, and operational vocabulary forever creates permanent dual identity. Bounded legacy-home adoption is used only as a transition safety mechanism.

### Mandatory bridge release

Rejected because Taskfleet must support direct 0.5.1 migration anyway. A second old-identity release adds permanent artifacts and another release transaction without a proven prerequisite.

### Immediate physical home move or automatic first-run move

Rejected because movement during the public cut adds state risk and cannot prove old writers are gone. Existing roots are adopted in place; movement is explicit and quiescent.

### Legacy-path symlink

Rejected because it routes stale 0.5.1 writers into the canonical store and requires unproven bidirectional write/locking compatibility. Leaving paths separate plus permanent split-root refusal fails closed.

### Permanent old crates or old core wrapper

Rejected. The old CLI wrapper is bounded; no external `octl-core` dependent was observed. crates.io history remains without creating a second maintained product.

### Binary aliases in all channels

Rejected because Cargo, Homebrew, shell installers, and archives have incompatible ownership and coverage. A documented channel-specific contract is safer than nominal uniformity.

### Renaming the old Homebrew tap

Rejected because GitHub redirects do not model Homebrew's local tap identity and can create duplicate taps. A new canonical tap plus permanent static old migration stub uses Homebrew's native mechanism.

## References

- `issues/rename-taskfleet/item.md`
- `issues/rename-taskfleet/plan.md`
- `docs/decisions/0001-thin-supervisor-vs-harden.md`
- `AGENTS.md` state-integrity invariants
- `Cargo.toml`, `crates/octl-{cli,core}/Cargo.toml`
- `crates/octl-cli/src/home.rs`, `crates/octl-core/src/schema.rs`
- `OSS-RELEASE.md`, `dist-workspace.toml`
- `.github/workflows/publish-crates.yml`, `.github/workflows/release.yml`
- Homebrew 6.0.21 `formulary.rb`, `migrator.rb`, and tap-migration tests (read 2026-09-02)
- crates.io API package and reverse-dependency responses (read 2026-09-02)
- complete panel: `history/2026-09-02-panel-taskfleet-rename-retry.md`
