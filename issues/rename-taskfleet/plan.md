# Taskfleet rename implementation breakdown

This plan implements [ADR 0002](../../docs/decisions/0002-taskfleet-rename-migration.md). The feature remains open until the canonical release, cross-repository convergence, and compatibility-removal gate are complete.

## Scheduling principles

- Sequence slices that touch state resolution, state schemas, release machinery, or the CLI/skill snapshot surface. Do not parallelize hot files named in `AGENTS.md`.
- Every slice uses a worktree. The orchestrator does not edit implementation code or mutate the installed binary/skills.
- Repository tests use isolated homes, Cargo homes, prefixes, and Homebrew test roots. Invoke only repository-local binaries.
- No crate publish, tag push, GitHub rename, tap mutation, or global install occurs before its explicit ADR gate.
- Preserve neutral state/JSON wire names. A branding replacement is never allowed to rewrite event history or generated/history/vendor content blindly.

## Dependency-ordered current-repository slices

### R0 — Freeze identity inventory and migration fixtures

**Depends on:** ADR 0002.

- Produce a checked inventory of executable/package names, environment variables (including worker control variables), default and repository paths, log targets, config keys, skills, contracts, release scripts, exact URLs, snapshots, and documentation.
- Capture sanitized 0.5.1 home fixtures for: completed run, non-terminal run, pending merge transaction, pi skill provenance, config/profile selection, and legacy/unknown schema values.
- Add package/archive/cargo-dist plan fixtures and an old Homebrew receipt/tap fixture.
- Classify every old-name occurrence as active identity, bounded compatibility, neutral/historical wire data, test fixture, or post-release external convergence.

**Gate:** no unidentified identity-bearing write path; fixtures validate under 0.5.1.

### R1 — Introduce a name-neutral shared CLI entry point

**Depends on:** R0.

- Refactor the current binary main into one callable dispatcher used by both executable targets.
- Parameterize display identity and structured deprecation warning without changing command behavior or envelope schema.
- Add same-build equivalence tests for text/JSON/JSONL and hidden self-exec paths (`supervise`, `run-worker`, reattach, generated recovery commands).

**Gate:** no duplicate dispatcher/engine; current `orchestratectl` remains behaviorally unchanged.

### R2 — Implement dual-name configuration resolution

**Depends on:** R1.

- Inventory and add `TASKFLEET_*` canonical names for home, profile, harness, log, build provenance, and worker-control/telemetry variables.
- Centralize alias precedence: new-only wins, old-only warns, equivalent dual values warn, conflicting dual values fail.
- Add `.taskfleet.toml` support with old-only fallback and dual-file conflict/equality handling.
- Ensure logging, doctor, skill provenance, subprocess environments, generated prompts, and config inspection use the same resolver.

**Gate:** exhaustive precedence matrix; no command resolves a home independently.

### R3 — Build the quiescent home migration

**Depends on:** R2. **Hot files/state correctness: sequence alone.**

- Add dry-run and explicit migration actions suitable for the old 0.5.2 bridge and canonical binary.
- Add the external migration lock, active process/run/pending-merge checks, split-root refusal, symlink/path validation, source validation, whole-root atomic rename, marker, backup, and rollback-before-first-write semantics.
- Permanently detect a populated legacy root so Taskfleet never silently presents an empty state.
- Reuse `LockedRun` and projection/event validation; do not write projections directly or rewrite events.
- Add crash/fault injection around every move/promotion boundary.

**Gate:** 0.5.1 fixtures preserve event bytes, sequences, IDs, branches, OIDs, and merge behavior; all uncertain/live/split cases fail closed.

### R4 — Prepare and cut old-identity bridge 0.5.2

**Depends on:** R3.

- Add migration/deprecation documentation and direct 0.5.1-to-Taskfleet recovery instructions.
- Update release fixtures/snapshots for 0.5.2 without changing canonical distribution identity yet.
- Run the full repository green gate and isolated migration drills.
- Use `scripts/shipshape-release.sh` only; verify old crates, old GitHub assets, and old Homebrew formula.

**Irreversible gate:** ADR bridge-tag gate. This is the last ordinary old-identity release before canonicalization.

### R5 — Rename canonical Rust packages, source layout, and executable

**Depends on:** verified R4.

- Rename active packages/source directories to `taskfleet` and `taskfleet-core`; update repository/homepage/description metadata and exact canonical dependency pin.
- Make `taskfleet` the primary executable. Add the old executable as a thin same-dispatch compatibility target through 0.7.x.
- Add thin old package wrappers/re-exports with explicit deprecation metadata and no independent implementation.
- Update internal diagnostics, tracing targets where operationally useful, help/version metadata, test binary lookups, and build-provenance variables.
- Keep state schema/event/report/envelope field spellings unless independently incompatible.

**Gate:** `cargo metadata` and package dry-runs show the intended four-package transition graph; wrappers contain no engine logic.

### R6 — Rename bundled skills, prompts, contracts, and provenance

**Depends on:** R5. **CLI snapshot surface: sequence with R5/R7.**

- Rename Taskfleet-owned skill identities such as overview/run helpers where appropriate; keep generic `/worktree-*`, `/fan-out`, and `/stint-*` names stable.
- Change generated commands and new source references to `taskfleet`.
- Migrate skill provenance records and pruning logic without deleting user-edited copies. Old installed skills must remain executable through the command alias during the window.
- Revise the worker telemetry contract with bounded old/new environment aliases; keep DTO field compatibility.
- Run the full insta acceptance/review loop and the explicit catalog pin test.

**Gate:** isolated Claude/pi/Codex install, upgrade, divergence, and prune tests; no global skill writes.

### R7 — Convert repository documentation and machine contracts

**Depends on:** R5–R6.

- Update `AGENTS.md`, nested guidance, README, changelog, architecture/ADR index, code comments, examples, contract docs, demo scripts, and issue-facing templates.
- Preserve old names only in ADR/history, compatibility tests, deprecation instructions, and the explicit bridge path.
- Update `OSS-RELEASE.md` targets to canonical packages/tap while declaring bounded wrapper legs for 0.6.x–0.7.x.
- Update Shipshape bump hooks and version snapshot checks for the new package set.

**Gate:** classified repository search has no unexplained old-name occurrence.

### R8 — Rebuild release and cargo-dist machinery

**Depends on:** R5–R7. **Release-sensitive: sequence alone.**

- Update dependency-ordered crates.io workflow for `taskfleet-core`, `taskfleet`, and bounded wrappers with idempotent retry semantics.
- Migrate hard-coded repository/package checks in `scripts/shipshape-release.sh`; rerun and record the held-tag protocol tests before accepting changed assumptions.
- Change cargo-dist app/tap/bin-alias settings, regenerate `.github/workflows/release.yml`, and inspect planned asset/formula names.
- Ensure old command coverage for Cargo installs and archives is explicit; do not assume installer-only `bin-aliases` covers archives.
- Add clean disposable Cargo/shell/Homebrew install and old-upgrade scripts.

**Gate:** release dry-runs, cargo-dist PR plan, exact package list, asset list, formula content, and wrapper behavior are reviewed and green.

### R9 — Integrated pre-cut validation

**Depends on:** R8.

Run on the integrated exact commit:

- the full green gate from `AGENTS.md`, including docs and clean-PATH tool-sensitive tests;
- complete CLI/skill snapshot review;
- 0.5.1 and 0.5.2 state migrations, split/live/pending-merge/fault tests, in-flight old-prompt completion, and rollback-before-write drill;
- disposable Cargo install/uninstall/upgrade and generated archive/installer checks;
- disposable old and fresh Homebrew flows;
- `git diff --check`, `issuectl doctor --json`, Shipshape contract/audit, and canonical name availability rechecks.

**Gate:** attach immutable outputs to the issue/release record. Any failed required leg blocks repository/tap rename and publish.

### R10 — Controlled GitHub and tap rename window

**Depends on:** R9 and explicit ADR irreversible gates.

- Recheck `jarimustonen/taskfleet` and `jarimustonen/homebrew-taskfleet` immediately.
- Rename the main repository; never recreate `jarimustonen/orchestratectl`.
- Update local/CI remotes, exact URLs, secrets, badges, action references, and repository settings; verify canonical push/clone and expected ordinary redirects.
- Rename the tap, add `formula_renames.json` (`orchestratectl` → `taskfleet`), update cargo-dist target, and validate an old local tap/receipt path.

**Gate:** canonical repositories and exact CI references work without relying on redirects. If a candidate name is unavailable, stop and return to the product owner; do not improvise another identity.

### R11 — Canonical Taskfleet 0.6.0 release

**Depends on:** R10.

- Seal and inspect the Shipshape plan from clean synchronized main.
- Advance main to the bump commit and wait for CI on that exact SHA.
- Resume the held tag only through `scripts/shipshape-release.sh`.
- Verify canonical crates, wrappers, GitHub assets/checksums/installers, tap formula, and embedded commit.
- Exercise fresh and old-upgrade installations in disposable environments and record registry reconciliation.

**Irreversible gate:** ADR canonical crate/tag gates. Never run local `cargo publish`, bare Shipshape resume with a local tag, or manual tag push.

## Post-release cross-repository convergence

### E1 — Discover owners and active references

**Depends on:** verified R11.

- Search maintained repositories for executable/package names, env/config paths, Git URLs/actions, install commands, Homebrew formula/tap, skills, telemetry adapters, fleet units, and service configuration.
- Exclude generated, vendored, build, and historical material from blind replacement; classify intentional historical references.
- Use Homebase fleet/status/doctor and repository documentation to identify the owner of each machine-level deployment. Specifically discover, do not guess, which Homebase/intake repository and fleet unit owns Haapa.
- File or update work only in the owning repository; this worker does not assign foreign work from here.

**Gate:** owner map with repository, path/unit, current dependency channel, and migration order.

### E2 — Converge one owning repository per worktree

**Depends on:** E1; parallelize only truly disjoint repositories.

For each owner:

- update canonical command, package, URLs, env/config names, and installed skills;
- migrate its state only through the Taskfleet migration tool and only after its runs are quiescent;
- run that repository's own tests and machine-convergence policy;
- preserve explicit compatibility/history references and document any deferred machine.

An unreachable machine is unverified, not converged.

### E3 — Verify ecosystem convergence

**Depends on:** all E2 worktrees.

- Repeat maintained-source and fleet searches.
- Verify no active old command, old package install, old config/env, old tap, or exact old GitHub URL remains outside declared compatibility fixtures.
- Confirm no non-terminal run or pending merge remains in a legacy root.
- Attach results to `rename-taskfleet`.

## Compatibility removal release

### C1 — Keep the window healthy through 0.7.x

**Depends on:** R11.

- Maintain wrappers and aliases only as forwarding surfaces.
- Fix canonical Taskfleet first; wrappers receive only the minimum forwarding/version update.
- Exercise old invocation and migration fixtures on each 0.6.x/0.7.x release.

### C2 — Remove active compatibility in 0.8.0

**Depends on:** E3, date ≥ 2026-12-01, and every ADR removal criterion.

- Remove old executable/config/environment aliases and old package wrappers from the active workspace/release contract.
- Keep historical state/schema readers, migration markers, legacy-root safety detection, formula rename metadata, ADRs, changelog, and fixtures needed to prove compatibility.
- Re-run full release and migration gates, then cut 0.8.0 through Shipshape.

### C3 — Close the feature

**Depends on:** verified C2.

- Record canonical, convergence, and removal commits/releases on `rename-taskfleet`.
- Confirm acceptance criteria and close the feature only now.
