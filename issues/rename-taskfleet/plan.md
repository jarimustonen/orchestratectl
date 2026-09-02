# Taskfleet rename implementation plan

This plan implements [Accepted ADR 0002](../../docs/decisions/0002-taskfleet-rename-migration.md). The feature remains in progress until the canonical release, ecosystem convergence, and 0.8.0 compatibility-removal gate are complete.

## Scheduling rules

- Every implementation slice runs in its own worktree. Sequence state resolution, state migration, release machinery, and CLI/skill snapshot edits.
- Preserve neutral state/JSON and stable `OCTL_*` protocol vocabulary. Never globally replace generated, vendored, history, or persisted event data.
- Repository validation uses repository-local binaries and disposable homes/Cargo prefixes/Homebrew prefixes. Never mutate the installed orchestratectl binary or bundled skills during repository work.
- No crate publish, tag push, GitHub rename, tap mutation, global installation, or source rename occurs before its ADR gate.

## Current-repository dependency chain

### R0 — Freeze identity inventory and 0.5.1 fixtures

**Depends on:** ADR 0002.

- Inventory packages/binaries, branded public variables, stable `OCTL_*` protocol/test variables, state/config paths, self-exec paths, skills/prompts/provenance, release scripts, URLs, action references, cargo-dist assets, and tap/formula ownership.
- Capture sanitized 0.5.1 homes for completed, non-terminal, pending-merge, config/profile, installed-skill provenance, and unknown-but-readable schema values.
- Classify each old-name occurrence as active identity, bounded compatibility, permanent safety/history, test fixture, or external convergence.
- Recheck canonical crates/repositories/tap names without treating availability as reservation.

**Gate:** no unidentified identity-bearing writer; fixtures validate on 0.5.1.

### R1 — Extract one linkable CLI dispatcher

**Depends on:** R0.

- Refactor binary `main` into a minimal callable dispatcher suitable for the future Taskfleet package and old CLI wrapper.
- Keep parser, command execution, output envelopes, state resolver, and error formatting shared.
- Add invocation-identity handling only for help/version/deprecation; never shell out to another binary or infer behavior from an unsafe `PATH` lookup.
- Test hidden self-exec paths (`supervise`, worker launch, merge/recovery, reattach) through the shared entry point.

**Gate:** current `orchestratectl` behavior unchanged; one dispatcher and one engine.

### R2 — Centralize dual-name inputs and bounded legacy-home adoption

**Depends on:** R1. **State-sensitive; sequence alone.**

- Implement the ADR home matrix: explicit new/old equivalence, conflict refusal, canonical-only, legacy-only adoption, fresh canonical default, and dual-populated refusal.
- Add `TASKFLEET_HOME/PROFILE/HARNESS/LOG`; retain old branded aliases through 0.7 with warn/equal/fail semantics.
- Add `.taskfleet.toml` canonical selection with old-only fallback and differing-dual refusal.
- Inventory `OCTL_*`; retain notify/worker protocol names and separate internal/test seams from public branded input.
- Route logging, doctor, skill provenance, subprocesses, config inspection, and every command through the same resolver.

**Gate:** exhaustive precedence matrix; no independent home/config resolution; 0.5.1 legacy home is adopted without writes or movement.

### R3 — Implement optional quiescent same-filesystem migration

**Depends on:** R2. **Hot state path; sequence alone.**

- Add dry-run and explicit migration commands.
- Require normalized exact paths, absent destination, external migration lock, no non-terminal run/live process/pending merge/held run lock/state writer, and normal reducer validation.
- Perform only whole-root same-filesystem atomic rename. Refuse cross-device movement.
- Record and verify migration without rewriting events, projections, ids, paths, sequence numbers, branches, or OIDs.
- Leave no symlink at the old path. Permanently detect and fail on recreated/dual roots.
- Define and test first-canonical-write rollback boundary.

**Gate:** event hashes, `applied_seq`, ids, OIDs, and merge behavior survive; active, dual-root, destination, symlink/path, fault, and cross-device cases fail safely.

### R4 — Rename canonical packages and command; add only the old CLI wrapper

**Depends on:** R1–R3.

- Rename active packages/source layout to `taskfleet` and `taskfleet-core`; exact-pin canonical dependencies.
- Make `taskfleet` the sole canonical Cargo binary.
- Add a thin `orchestratectl` compatibility package which links the Taskfleet dispatcher and owns the old Cargo binary name. Keep it outside any layout that causes duplicate target artifacts.
- Do not add an `octl-core` wrapper unless R0 finds a real external source dependent and the ADR is amended.
- Add same-version wrapper release metadata for every 0.6.x/0.7.x canonical release.

**Gate:** `cargo metadata` and package archives show exactly one engine, canonical packages, and an implementation-free old CLI wrapper; no duplicate bin ownership.

### R5 — Convert skills, prompts, provenance, docs, and contracts

**Depends on:** R4. **Snapshot-sensitive; sequence with R4/R6.**

- Change generated commands and new source references to `taskfleet`.
- Rename Taskfleet-owned skill identities where appropriate; keep generic `/worktree-*`, `/fan-out`, and `/stint-*` workflow names stable.
- Migrate provenance/managed-marker handling without deleting user-edited copies.
- Preserve stable `OCTL_*` notify/worker contracts.
- Update AGENTS/README/architecture/changelog/examples/templates and migration instructions; classify every retained old name.
- Update `OSS-RELEASE.md` for canonical packages, bounded old CLI wrapper, canonical tap, stub installer, and independent saga legs.

**Gate:** full insta review loop; isolated skill install/update/prune tests; classified search contains no unexplained active old identity.

### R6 — Rebuild crates.io and Shipshape release machinery

**Depends on:** R4–R5. **Release-sensitive; sequence alone.**

- Replace the hard-coded two-crate workflow with `taskfleet-core → taskfleet → orchestratectl` publication and per-dependent index visibility retry.
- Verify existing package/version by registry checksum, owners, dependency requirements, metadata, and source commit before reconciling it.
- Make repository/package identity in `scripts/shipshape-release.sh` data-driven and rerun its pinned 0.10.1 migration-build protocol tests.
- Update version hooks/snapshots and exact canonical/wrapper pins.
- Document partial-success resume/fix-forward and generated Homebrew empty-commit repair.

**Gate:** sealed dry-run plan and package archives list exact intended legs; held-tag exact-SHA protocol tests pass.

### R7 — Prepare cargo-dist and Homebrew topology

**Depends on:** R5–R6. **Distribution-sensitive; sequence alone.**

- Create (but do not yet activate) the canonical `homebrew-taskfleet` repository and verify a re-scoped `HOMEBREW_TAP_TOKEN` with a reversible test commit.
- Prepare the old tap's atomic migration commit: delete `Formula/orchestratectl.rb`, add full-identity `tap_migrations.json`, and prevent future generated formula writes. Do not push until canonical formula verification.
- Set cargo-dist 0.28.2 to the new tap, Taskfleet app/assets, and an old latest-installer migration stub; regenerate `release.yml`.
- Do not install an `orchestratectl` alias in Homebrew, shell installer, or archives.
- Prepare all exact GitHub URL/action/secret/runner/release-wrapper substitutions.

**Gate:** PR `dist plan` shows exact Taskfleet archives/checksums/installer/formula plus one non-installing stub, zero unintended old artifacts, one generated tap; disposable Homebrew plans are reviewed.

### R8 — Integrated pre-cut validation

**Depends on:** R7.

Run on the exact integrated commit:

- full Rust green gate, rustdoc, clean-PATH tests, CLI/skill snapshot review, `git diff --check`, and `issuectl doctor --json`;
- both-name Cargo-wrapper command suite: byte-identical stdout/JSON/JSONL and exit codes, stderr-only once-per-process deprecation;
- 0.5.1 legacy-home adoption, active-run completion before movement, optional migration, dual-root refusal, pending-merge, and rollback-boundary fixtures;
- disposable Cargo, archive, shell, fresh Homebrew, and old-receipt/tap flows;
- Shipshape contract/audit/plan and crates/GitHub/tap name rechecks.

**Gate:** immutable evidence attached to the issue. Any failed leg blocks repository rename and tag.

### R9 — Rename GitHub repository

**Depends on:** R8 and ADR GitHub gate.

- Rename `jarimustonen/orchestratectl` to `jarimustonen/taskfleet`; never recreate the old name.
- Immediately update local/CI remotes, exact URLs, action references, badges, settings, release-wrapper identity, and secrets.
- Verify the self-hosted macOS runner accepts a job in the renamed repository and canonical clone/fetch/push works.
- Re-run exact-SHA main CI after the identity substitutions.

**Gate:** canonical identity works without maintained references relying on redirects. Fix forward; do not routinely rename back.

### R10 — Cut and verify Taskfleet 0.6.0

**Depends on:** R9 and all ADR irreversible gates.

- Seal the Shipshape plan from synchronized clean main.
- Advance main to the exact bump commit; wait for exact-SHA green push CI.
- Resume the held tag only through `scripts/shipshape-release.sh`.
- Reconcile independently completing crates.io and cargo-dist legs; never imply cross-workflow ordering.
- Verify registry receipts, GitHub assets/checksums/stub, clean Taskfleet installs, embedded commit, and canonical formula in the new tap.

**Irreversible gate:** no direct `cargo publish`, bare resume with local tag, manual tag, retag, or version reuse.

### R11 — Activate and verify Homebrew migration

**Depends on:** verified R10 canonical formula.

- Push the reviewed old-tap commit deleting the old formula and adding cross-tap migration metadata.
- Add new-tap formula rename metadata only if the isolated recursive-resolution drill proves it necessary.
- Test fresh canonical install, old receipt `brew update/upgrade`, `brew migrate`, old tap-qualified install resolution, direct canonical install, and uninstall in disposable prefixes.
- If metadata is wrong, revert only the tap commit; do not alter published crates/releases.

**Gate:** old and fresh paths resolve to one canonical formula with no duplicate tap/formula ownership.

## Post-live cross-repository convergence

### E1 — Discover owners and active references

**Depends on:** verified R10–R11.

- Search maintained repositories for command/package names, env/config homes, URLs/actions, install commands, tap/formula identities, skills, telemetry adapters, and fleet units.
- Exclude generated/vendor/history from blind replacement and classify intentional compatibility references.
- Use Homebase fleet/status/doctor and repository guidance to discover, not guess, which repository/unit owns Haapa and intake-related deployment.

**Gate:** owner map records repository, path/unit, dependency channel, state location, and ordering.

### E2 — Converge one owning repository per worktree

**Depends on:** E1; parallelize only disjoint repositories.

For each owner:

- update canonical command, package, URLs, env/config, and installed skills;
- finish/quiesce old runs before replacing alias-free binary channels or moving state;
- run that repository's tests and machine-convergence policy;
- preserve explicit history/compatibility references and mark unreachable machines unverified.

### E3 — Verify ecosystem convergence

**Depends on:** all E2 worktrees.

- Repeat maintained-source/fleet searches.
- Verify no active old command/install/config/tap/exact URL remains in supported reachable integrations.
- Record unreachable/unknown installations honestly; they do not prove failure or block forever.

## Compatibility removal

### C1 — Maintain the bounded window

**Depends on:** R10.

- Through 0.7.x, publish same-version exact-pinned `orchestratectl` wrappers for canonical releases.
- Keep old branded input/config aliases and legacy-home adoption green; keep `OCTL_*` protocol stable.
- Announce 0.8 removal in 0.7.0 and support 0.7.0 for at least 30 days.

### C2 — Gate and cut 0.8.0

**Depends on:** date ≥ 2026-12-01, ≥90 days after verified 0.6.0, 0.7.0 age ≥30 days, E3 for supported reachable integrations, and every ADR sunset fixture.

- Stop new old-CLI wrapper releases.
- Remove old command/config/environment fallback from fresh Taskfleet releases; retain actionable removed-input errors.
- Require explicit migration of populated legacy homes before writes.
- Keep historical state/schema readers, split-root detection, migration receipts, GitHub redirects, old tap metadata, old registry artifacts, and required fixtures.
- Run the full integrated release gate and cut only through Shipshape.

### C3 — Close the feature

**Depends on:** verified C2.

- Record canonical, convergence, and removal commits/releases on `rename-taskfleet`.
- Confirm every acceptance criterion and only then close the feature.
