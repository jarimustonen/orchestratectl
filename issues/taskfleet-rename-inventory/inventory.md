# Taskfleet rename identity inventory (R0)

Frozen against repository commit `c9b161d3b6235adde9ed7db3e480757f6240ae0f` and published orchestratectl 0.5.1 evidence on 2026-09-02. This is a classification map, not a replacement list. R1–R8 must update only the class assigned here and must preserve historical/wire values.

## Search boundary and classification

The tracked tree was first searched case-sensitively for `orchestratectl`, `octl`, `octl-core`, `ORCHESTRATECTL_*`, `OCTL_*`, old repository/tap URLs, installer/assets and self-exec forms. Review found that this missed mixed-case writers such as `# Orchestratectl run context`, so the final freeze uses the case-insensitive, deterministic `check-identity-inventory.py`. Its generated `identity-occurrences.tsv` classifies every matched token by path, line, class and implementation owner; `--check` fails when source or the ledger drifts. The initial broad search found 1,243 non-history lines and 293 issue files; the final occurrence ledger is authoritative rather than those exploratory counts. `Cargo.lock`, cargo-dist's `.github/workflows/release.yml`, and insta snapshots are generated but tracked: change their inputs and regenerate/review them; never blind-edit or globally replace them. `issues/`, `CHANGELOG.md`, accepted ADRs and gitignored `history/` are historical records, not rename targets except for new forward-looking plan links.

Classes used below:

- **A — active canonical identity:** becomes Taskfleet in R4–R7.
- **B — bounded compatibility:** old spelling remains deliberately through 0.7 and is removed/gated in 0.8 per ADR 0002.
- **P — permanent protocol/safety/history:** must remain readable or retain its spelling; a product rename is not authority to change it.
- **F — frozen fixture/test expectation:** update only with the owning implementation and review generated output.
- **G — generated/vendor/history:** regenerate or retain; never blind-replace.
- **E — external convergence:** separately owned after the canonical release.

## Packages, binaries and Rust identity

| Surface | Current writer/readers | Class | Migration owner |
|---|---|---|---|
| Workspace paths `crates/octl-cli`, `crates/octl-core`; Cargo packages/binary `orchestratectl`, `octl-core`; exact `octl-core = "=0.5.1"` | root and crate `Cargo.toml`, `Cargo.lock`, package metadata/tests, CI publish workflow | A, except old CLI package B | R4 creates `taskfleet-core` → `taskfleet` and a link-only same-version `orchestratectl` wrapper. No old core wrapper. |
| Clap command/help/version product strings and `CARGO_BIN_EXE_orchestratectl` | `src/cli.rs`, `help.rs`, `output.rs`, integration tests/snapshots | A/F | R1 extracts the dispatcher; R4 gives invocation identity; R5 accepts reviewed snapshots. |
| Build provenance `ORCHESTRATECTL_GIT_COMMIT` | `build.rs` writer; `cli.rs`/doctor readers | A (private build key) | R4/R5 may rename internally; it is not a user input or persisted protocol. |
| Library namespace `octl_core`, temp prefixes and human diagnostics | all CLI/core source and tests | A | R4/R5, with output parity/deprecation exceptions documented explicitly. |
| Tracing targets `octl_core::*` / `orchestratectl::*` consumed through `ORCHESTRATECTL_LOG` filters | `cli.rs` and tracing call sites | B | Preserve old filters through 0.7 or implement explicit compatible filtering in R2/R4; do not silently rename targets as private symbols. |
| Doctor checkout/package identity (`is_orchestratectl_checkout`) | `doctor/checks/binary.rs` | A/B semantic reader | R4 must recognize canonical checkout and supported old/wrapper checkout without changing binary-commit diagnostics accidentally. |
| Published `orchestratectl` and `octl-core` versions/lockfiles | crates.io and historical Cargo state | P/B | Registry history is permanent; wrapper publication is B through 0.7; R6 owns new publication. |

There is exactly one current executable owner (`crates/octl-cli`), one core implementation and no external reverse dependency of `octl-core` visible on crates.io other than `orchestratectl` itself. R4 must not create duplicate target binary ownership.

## Home, config, logs and branded public inputs

| Identity | Writers/readers | Class | Required handling |
|---|---|---|---|
| `ORCHESTRATECTL_HOME`, default `~/.orchestratectl`, `runs/`, idempotency, logs and `state/pi-installed-skills.json` | `src/home.rs` is the primary resolver; `cli.rs` also derives the log fallback; config, doctor, idempotency, run, skill and supervisor modules consume it | B/P safety | R2 replaces all independent resolution with one resolver. Adopt a sole populated legacy root; fail on divergent explicit values or dual populated roots. R3 alone moves it. |
| `config.toml` under the home | `src/config/*`, harness/profile resolver, config/doctor tests | B | Remains the selected home's user config; old home adoption makes it reachable. |
| `.orchestratectl.toml` repository selection | `harness/profile.rs` reader and run/config tests | B | `.taskfleet.toml` canonical; old-only fallback warns; differing dual files fail. |
| `ORCHESTRATECTL_PROFILE`, `ORCHESTRATECTL_HARNESS`, `ORCHESTRATECTL_LOG` | profile/select/CLI logging and config inspection | B | Add `TASKFLEET_*`; old-only/equal warn, differing values fail through 0.7. |
| Paths embedded in manifests/events (`source_repo`, `worktree_root`, `worktree_path`) | state schema/reducer and run/supervisor paths | P | Historical payload bytes and paths are not branding substitutions and are never rewritten by migration. |

The current duplication in `home.rs` and `cli.rs` is an identified writer risk; R2's gate must prove logging, doctor, skills, subprocesses and every command use the same resolver.

## Stable `OCTL_*` protocol and control seams

These names are intentionally **not** changed by the rename.

- **Public worker telemetry P:** `OCTL_RUN_ID`, `OCTL_NODE_ID`, `OCTL_ATTEMPT`. Contract owners are `contracts/worker-telemetry-v1/*`; writers are profile launchers in `run/spawn.rs`; readers are the external adapter and telemetry tests. The contract discriminator `orchestratectl.worker-telemetry-adapter` is also P and remains byte-stable in v1; command argv/prose in the same files is A for R5.
- **Public notification/attention P:** `OCTL_RUN_ID`, `OCTL_STATUS`, `OCTL_SUMMARY`, `OCTL_RUN_KIND`, `OCTL_RUN_TITLE`, `OCTL_AWAITING_INPUT`, `OCTL_AWAITING_INPUT_JSON`. Writer is `supervise/notify.rs`; generated skill consumers and schema/help text are compatibility obligations.
- **Private self-exec/readiness P until separately versioned:** `OCTL_READINESS_FD` (`run/supervisor_readiness.rs`, `run/supervisor_spawn.rs`, `supervise/mod.rs`). It crosses exec boundaries inside one release.
- **Internal/test seams F:** `OCTL_CREATE_SH`, `OCTL_MERGE_SH`, `OCTL_SUPERVISE_BIN`, `OCTL_READY_WAIT_MS`, `OCTL_PID_FILE_WAIT_MS`, `OCTL_DEATH_GRACE_SECS`, `OCTL_AWAITING_INPUT_GRACE_SECS`, `OCTL_CHILD_SPAWN_DEADLINE_SECS`, `OCTL_NO_WORKER_GRACE_SECS`, `OCTL_STILLBORN_LIST_GRACE_SECS`, `OCTL_WATCHDOG_GRACE_SECS`, `OCTL_AGENT_RESPAWN_MAX_FAILURES`, `OCTL_AGENT_RETRY_MAX_ATTEMPTS`, `OCTL_AGENT_RETRY_BACKOFF_SECS`, `OCTL_TMUX_RETRY_BACKOFF_MS`, `OCTL_IDEMPOTENCY_PUBLISH_WAIT_MS`, and `OCTL_TEST_*`. They are not branded public configuration; retain them to avoid needless test/control churn and because ADR 0002 defaults all `OCTL_*` to stable.

## Self-execution and subprocess boundaries

| Boundary | Current behavior | Class / owner |
|---|---|---|
| Detached supervisor | `run/supervisor_spawn.rs` uses `current_exe()` then `supervise`; reattach and child spawn share it | P mechanism, A diagnostics; R1/R4 must keep it in-process and invocation-safe. |
| Worker shim | `run/spawn.rs` materializes a launcher which executes the selected harness through hidden `run-worker`; retry consumes recorded argv | P state semantics; R1/R4 test both binary names without PATH lookup. |
| Merge | `run/merge.rs` embeds/materializes `scripts/merge.sh`, records OID transaction, then executes it; `OCTL_MERGE_SH` is test-only | P transaction/state, A temp/diagnostic names; R1/R4/R5. |
| Doctor fixes | `doctor/fix.rs` uses `current_exe()` to invoke skill installation | A/B and user-data-sensitive; R2/R5 must route identity/provenance safely. |
| External tools | typed wrappers invoke git/tmux/workmux; notify executes `sh -c`; profile argv executes user-owned pi/Claude | Neutral; do not brand-rename external executable names. |
| Generated worker commands/headings | every bundled skill and `harness/prompt.rs` currently emits `orchestratectl`, including mixed-case `# Orchestratectl run context`, exact `run merge` closure and source refs | A for newly generated text, P/B for already persisted old prompts; R5. |

No implementation may replace these with `Command::new("taskfleet")` through PATH. The dispatcher/current-executable route is the single engine.

## Skills, prompts and provenance

The catalog contains branded identities `orchestratectl-overview`, `octl-run-overview`, and `octl-spawn-spinoff`; generic `worktree-*`, `fan-out`, and `stint-*` names are stable workflow identities. Their bodies contain active command/product strings.

Provenance writers/readers are `src/skill.rs` and `doctor/checks/skill.rs`:

- Claude per-skill marker `.orchestratectl-managed`;
- Codex shared marker `.codex/prompts/_shared/.orchestratectl-managed`;
- pi's out-of-band schema-v3 record `<state-root>/state/pi-installed-skills.json` (no in-dir marker);
- embedded `path_in_repo`, catalog names, hashes, CLI versions and prune/orphan logic.

Classify new catalog names/commands as A (R5), but old markers, old catalog records and installed copies as P/B migration evidence. R5 must migrate records by hash, preserve diverged/user-authored files, and never infer ownership from a renamed directory. The fixture freezes one three-agent installation.

## State and historical protocol values

Schema v1 event kind strings, envelope/DTO fields, run/node ids, report `via`/typed origins, statuses, `applied_seq`, OIDs, branches and paths are P. `Kind::Unknown` is a P reader for removed/future values. Existing event lines may contain old generated commands, old skill names, source references or removed kinds; they are append-only history and must not be rewritten. Persisted argv, task/notify text, tmux/workmux identities and path strings are data, not replacement targets. `pending_merge` is especially immutable because recovery is pinned to its OIDs. R2/R3 tests must hash event files before/after adoption/movement.

`crates/octl-core/schemas/plan.v3.schema.json` carries permanent schema identifier `https://orchestratectl.dev/schemas/plan.v3.schema.json`; the removed plan subsystem makes the tracked file historical/schema evidence (P/G), not an active URL to rename in place. A future schema gets a new id. The LICENSE contributor identity is historical legal attribution until the maintainer deliberately updates it in R5; issue slugs and TODO links to old issues remain P/G even while active command prose around them becomes A.

The fixture under `fixtures/orchestratectl-0.5.1/` covers terminal, active, pending-merge, profile/config, installed-skill provenance and unknown-readable values. It is copied before tests and never mutated in place.

## Release, URLs, actions and distribution

| Surface | Current identity/writer | Class | Owner |
|---|---|---|---|
| Release contract | `OSS-RELEASE.md`: crates.io `octl-core` → `orchestratectl`, GitHub Release/Homebrew `orchestratectl` | A/B | R5/R6. |
| crates workflow | `.github/workflows/publish-crates.yml` hard-codes two packages, exact pin parsing and retry | A/F | R6 replaces with three ordered packages and registry receipt verification. |
| Shipshape wrapper | `scripts/shipshape-release.sh` hard-codes `jarimustonen/orchestratectl`, two package names; three protocol test scripts pin the same | A/F | R6 makes identity data-driven without weakening the held-tag exact-SHA gate. |
| Version hook | `scripts/shipshape-bump-hook.sh`, `check-version-snapshots.sh` pin package/help text | A/F | R5/R6 regenerate/review. |
| cargo-dist input/output | `dist-workspace.toml` pins cargo-dist 0.28.2, old app/tap; `.github/workflows/release.yml` is generated and currently writes `jarimustonen/homebrew-orchestratectl` | A/G | R7 edits input and regenerates output; never hand-maintain generated workflow identity. |
| GitHub exact URLs | Cargo metadata, README badges/installers, issue template/discussions, security/contributing docs, workflow comments/tests | A; old redirect references P only in migration docs | R5/R7 prepare; R9 (represented only in parent plan) activates after R8. |
| Assets/installers | `orchestratectl-<target>.tar.xz`, `orchestratectl-installer.sh`, binary in archives | A; old latest installer stub B | R7 prepares Taskfleet assets plus non-installing old stub; no old alias in new archives. |
| Homebrew | tap `jarimustonen/homebrew-orchestratectl`, formula `Orchestratectl`, install identity `jarimustonen/orchestratectl/orchestratectl` | old tap/formula E/B migration; canonical A | R7 prepares new tap and old atomic migration; activation remains post-R10/R11 in parent plan. |
| Third-party actions | `actions/*`, `dtolnay/*`, `taiki-e/*`, `EmbarkStudios/*`, `Swatinem/*` | Neutral external dependencies | No rename; only repository-scoped settings/secrets/runner context are R7/R9 concerns. |

## Authoritative external check (read-only, no reservation)

Observed 2026-09-02:

- crates.io: `orchestratectl` 0.5.1, 327 downloads; `octl-core` 0.5.1, 410 downloads. Both list only `jarimustonen` as owner. `orchestratectl` reports zero reverse dependencies; `octl-core` reports only exact `orchestratectl =0.5.1`.
- crates.io candidate endpoints `taskfleet` and `taskfleet-core`: HTTP 404.
- GitHub `jarimustonen/orchestratectl`: public, default branch `main`; `jarimustonen/taskfleet`: HTTP 404.
- GitHub `jarimustonen/homebrew-orchestratectl`: public and writable by the authenticated owner. Its live `Formula/orchestratectl.rb` installs only `orchestratectl` 0.5.1 from the three cargo-dist archives. `jarimustonen/homebrew-taskfleet`: HTTP 404.

A 404 is not ownership, availability assurance or reservation. Recheck immediately at every ADR irreversible gate. R0 performed no repository/tap creation, rename, token mutation, publish, tag, formula edit or install.

## External convergence inventory boundary

Tracked source establishes these known external ownership classes, but R0 did not mutate/search private fleets as if this repository owned them:

- installed binaries and skills on user machines;
- Homebase fleet units and Haapa/intake deployments;
- maintained repositories that invoke the command, package, old env/home/config, action URLs or tap;
- Cargo lockfiles and Homebrew receipts outside this repository.

R9/R10 remain unscheduled until R8 immutable evidence passes. Post-live E1 discovers the actual owner repository/unit for each external reference; E2 uses one owner worktree; unreachable machines remain unverified.

## R0 completeness gate

Every old-name occurrence is now assigned to A, B, P, F, G or E by the committed rule-based ledger. The highest-risk writers are the duplicated home/log resolver, **logging initialization before parsing/conflict refusal**, current-executable subprocesses, generated worker commands, skill provenance/pruning, hard-coded release wrapper, generated cargo-dist workflow and live old tap formula. R2 must select/refuse the authoritative root before file logging or any write; R3 must place migration logging outside both roots until its first-write decision. Their migration owners are R1–R8. No old-name writer may be introduced outside those owners without updating the classifier, ledger and ADR plan.
