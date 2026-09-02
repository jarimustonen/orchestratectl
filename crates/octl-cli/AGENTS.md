# crates/octl-cli

The orchestratectl CLI binary. Verb-noun structure (`run create`, `node list`, `event tail`, `skill install`, etc.) per `ai-first-cli-canon`. Bundled SKILLs live under `skills/<name>/SKILL.template.md` and are embedded via `build.rs` + `include_str!`.

## Shared CLI dispatcher

`src/lib.rs` owns the sole linkable parser/execution engine. Binary entry points are thin calls to `dispatch(InvocationIdentity)`; identity is explicit and is used only for help/version branding (future compatibility warnings attach at the same seam), never inferred from `argv[0]` or `PATH`. Hidden self-execution is centralized in `src/self_exec.rs` and always starts `current_exe()`. Detached supervise, reattach, merge-recovery reattach, and doctor fixes must use that helper rather than a product-name lookup or a second parser.

## Taskfleet dual-name resolver (ADR 0002 R2)

`src/home.rs` is the sole reader/resolver for `TASKFLEET_HOME`, `TASKFLEET_PROFILE`, `TASKFLEET_HARNESS`, `TASKFLEET_LOG`, their bounded `ORCHESTRATECTL_*` aliases, and `.taskfleet.toml` / `.orchestratectl.toml`. Dispatch parses first, then resolves these inputs before logging or command writes; structured and text help return before resolution and remain filesystem-pure. Resolved process inputs and repository-config bytes are frozen in a `OnceLock`, so this process cannot switch truth midway through a command. This cannot fence an unmodified 0.5.1 process; concurrent first establishment remains outside the documented operator-exclusion limit. Compatibility warnings are aggregated into one stderr line and hidden `self_exec` children inherit `OCTL_INTERNAL_SELF_EXEC=1` to suppress repeats. Do not read these branded variables directly elsewhere.

With no explicit home, a readable directory containing any entry is populated/managed (unknown entries fail safe as managed). Canonical-only selects `~/.taskfleet`; legacy-only adopts `~/.orchestratectl` in place; neither selects fresh canonical; dual-populated distinct roots fail before logging. Existing paths compare by canonical physical identity (symlink and filesystem case behavior included); missing paths compare as absolute lexically normalized paths. A sole explicit home intentionally overrides default-root discovery, while differing dual explicit homes fail. R2 never moves data or creates aliases.

## Taskfleet state migration (ADR 0002 R3)

`src/state.rs` owns `state migrate` / `state rollback`. The pair-keyed receipt and global migration lock live under `$HOME/.taskfleet-migrations`, outside both roots. Ordinary commands hold the shared external fence for their lifetime; migration takes a nonblocking exclusive fence and then bounded per-run exclusive probes. This fences current writers only: operators must exclude old 0.5.1 processes and open descriptors. Migration validates through typed core projection/event readers, requires terminal runs/nodes and no pending merge/live identity-bearing process, hashes all bytes, and performs one same-filesystem whole-root rename. Receipt ordering is `prepared → renamed → verified`, then either `canonical_write_started` or durable `rollback_prepared → rolled_back`; only `verified` may begin rename-back. Dispatcher marks `canonical_write_started` before canonical logging (the ordinary command's first attempted write). Never move logging into either root on migration paths, weaken marker-before-write ordering, rewrite events/projections, add a symlink, or infer that an open descriptor was fenced. See `docs/STATE-MIGRATION.md`.

## `doctor` binary build provenance

`doctor` always emits the stable `binary.commit` check first. Its optional `details` object exposes `binary_commit`, `repository_head`, and `comparison` (`match`, `mismatch`, `unavailable`, or `not_applicable`) so machine callers never scrape hashes from prose. When cwd is inside an orchestratectl checkout, a recorded build commit that differs from `HEAD` is a WARN, never a FAIL; branch and released-binary mismatches are legitimate. Outside this project's checkout, or when either reference cannot be established, the check remains informational. It never offers an autonomous fix or manages the installed binary.

## `skill install` dual-homes into pi.dev (`~/.pi/agent/skills/`)

A default `skill install` (plain, `--force`, or `--agent all`, without `--dest`) writes each skill's `SKILL.md` to **two** locations: the Claude Code path `~/.claude/skills/<name>/SKILL.md` AND a pi.dev mirror at `~/.pi/agent/skills/<name>/SKILL.md`. pi discovers skills from a per-skill dir just like Claude and invokes them as `/skill:<name>`; bare `/name` cross-references resolve via pi's injected available-skills list, so **only the install target differs: no body/link rewrite**. The pi mirror is byte-identical to the Claude `SKILL.md`. Because pi uses a per-skill directory like Claude, any bundled companion resources also mirror as byte-identical siblings of `SKILL.md`; their lifecycle is tracked in the out-of-band provenance record. This prevents a skill that requires a bundled sibling from aborting under pi. The current catalog bundles no companions. The pi mirror carries no in-dir provenance marker. Mirroring is skipped for a custom `--dest` and for `--agent codex` alone. See `src/skill.rs` `cmd_install`.

### pi mirror lifecycle — out-of-band provenance (`pidev-pi-skill-lifecycle`)

The pi dir may hold no `.orchestratectl-managed` marker, so its lifecycle is keyed on a single **out-of-band** JSON record at `<ORCHESTRATECTL_HOME or ~/.orchestratectl>/state/pi-installed-skills.json`. As of **schema v3** the record is a **flat per-file model** — `{ schema_version, skills: { <name>: { cli_version, files: { <relpath>: { sha256, kind: "skill"|"companion" } } } } }` — where every mirrored file (the `SKILL.md` body AND each companion sibling) is one independent `files` entry keyed by relpath. The body is no longer an ownership root nesting companions under it; that pre-v3 nesting (`{ sha256, cli_version, companions: { <file>: sha } }`) forced several lifecycle point-fixes (a companion written while the body write was skipped had no record to attach to; prune coupled companion cleanup to body divergence), which the flat model removes (issue `pi-provenance-flat-file-model`). **Read/upgrade path:** loading a legacy v1 (bare `sha256`/`cli_version`) or v2 (`+ companions` map) record reconstructs the `files` map from the legacy fields in place (`RawPiSkillRecord` via `#[serde(from)]`), so old records keep working; the strict future-schema guard still fails an install closed on a record newer than v3. On every pi write, the record is union-merged: a SKILL.md write inserts/refreshes the `SKILL.md` file entry (`kind: skill`) and the skill's `cli_version`, a companion write files itself directly (`kind: companion`, creating the skill entry if the body write was skipped) — so a targeted single-skill install never forgets the rest of the managed set. It is the SOLE authority for two decisions, both of which **never touch a pi dir we did not write** (a user's hand-authored pi skill is never recorded):

- **Prune** (gated on the same full-catalog `--force` as the Claude dir prune): each tracked file of a de-registered skill is handled **independently** (flat per-file — no privileged body). A file is deleted only if its on-disk bytes still hash to the recorded value (proof it is our unmodified copy); a **diverged** copy (user-edited since we wrote it), a symlink, or a squatting dir is left in place and dropped from tracking (`pi_mirror_diverged` / `pi_companion_diverged` / relinquished), and a failed delete keeps the file tracked for a retry. **Companions are pruned BEFORE the `SKILL.md`** so the per-skill dir can empty out; unlike the pre-v3 model, a diverged body no longer shields the companions — an unmodified companion is still pruned even when the body diverged, and the body is deleted even when a companion delete failed (the Kept companion simply stays tracked, never stranded). Each file that is deleted/relinquished/absent is removed from the record's `files` map; the skill entry is dropped once `files` is empty. Finally the now-possibly-empty per-skill dir is best-effort removed (`remove_dir`, non-recursive — a user sibling, or a surviving diverged/Kept file, is preserved).
- **`doctor` drift** — `skill.sync.<name>.pi` (older/newer/unparseable/edited via the recorded hash) + `skill.orphan.<name>.pi` (de-registered but still on disk), plus the **companion** arms `skill.sync.<name>.pi.<file>` (forward drift of each bundled companion vs the embedded body — content-identity in-sync signal, same as the codex companion check) and `skill.orphan.<name>.pi.<file>` (a companion the record tracks that the binary no longer bundles). All gated on the record being non-empty so a host that never dual-homed into pi stays 0-warn. ALL pi arms are **advisory — no autonomous `FixAction`** (unlike the Claude older-drift arm): the fix applier runs `skill install <name> --force`, which dual-homes and would force-overwrite the Claude copy too, so autofixing pi drift could silently downgrade a deliberately newer/edited Claude copy. Symmetric with the codex checks. Mirrors the Claude/codex checks in `src/doctor/checks/skill.rs`.

Integrity: the record is **loaded and validated before any file is written** (`load_pi_provenance_for_write`) — a corrupt or future-schema record **fails the install closed** (never silently laundered to empty and overwritten, which would erase tracking for every mirror). Record-sourced skill names are validated as single path components before any `join`→`remove_file` (`is_simple_skill_name`), and the empty-dir cleanup is bound to `<pi-root>/<name>/`. The record read-modify-write is **unlocked** (parity with the Claude/codex markers): concurrent `skill install` runs can lose one another's additions, so mutation commands are not meant to run concurrently.

## Insta snapshot test loop

Many integration tests in `tests/` use `insta` for envelope / help / skill-catalog snapshots. After any CLI surface change (added flag, renamed verb, new bundled skill, edited error message), running `cargo test -p orchestratectl` produces `.snap.new` files for every diff. Accept them with:

```bash
find crates/octl-cli/tests/snapshots -name "*.new" -exec sh -c 'mv "$1" "${1%.new}"' _ {} \;
cargo test -p orchestratectl
```

Often takes **2–3 rounds** because the first accept-pass reveals further drifts only visible once earlier snapshots settle. Re-run the loop until `cargo test -p octl-cli` is green.

**A workspace version bump is a snapshot change too.** The `version_*` snapshots (`envelope_snapshots__version_{text,json,jsonl}.snap`) bake in the literal crate version, so bumping `[workspace.package] version` in `Cargo.toml` stales them exactly like a CLI-surface edit — run the accept loop above (or `cargo insta test --accept -p orchestratectl`) after the bump, or `cargo test` goes red. `scripts/check-version-snapshots.sh` (also a CI job) fails loudly on a version/snapshot mismatch as a fast pre-publish guard; the release-mechanics obligation is in `OSS-RELEASE.md` alongside the CHANGELOG-finalize step.

## Skill catalog: edit pin-test explicitly

The bundled-skill list is hardcoded in `tests/skill.rs::skill_list_json_pins_catalog_shape`. When adding or removing a bundled skill, **also edit that test's `vec![...]` literal** — the snapshot loop above will NOT catch it (different test). Forgetting this gives a single failing test on the next run.

## Test-spawn hygiene

Integration tests that exercise `run create`'s production path spawn real `orchestratectl supervise` subprocesses. The shared `tests/common/mod.rs` `TestHome` fixture reaps them on Drop — use it (`let home = TestHome::new()`) for any new test that goes through `bin(&home)`. Tests that skip the fixture and use raw `TempDir` leak supervisor processes; the `supervise-test-teardown-leak` issue covers the recovery + reaper logic in detail.

After `cargo test -p octl-cli` finishes, `pgrep -lf "orchestratectl.*supervise"` from the workspace `target/debug/` path should return nothing. Any survivor is a missing-fixture bug in the new test.

## End-to-end spinoff harness

`tests/e2e_spinoff.rs` drives ONE full autonomous-spinoff round-trip on every run — `run create --kind spinoff --headless` (real detached supervisor) → live stub agent → `run merge` → supervisor rolls the run up to `done`, tears down, and exits — and asserts the canonical event sequence (`run.created`, `node.created`, `supervisor.started`, `node.report`, `run.status`, `supervisor.exited`) + terminal manifest. It is the CI gate for the merge / cleanup / supervisor-lifecycle paths.

It stubs the two shell-out boundaries through the production override hooks — `OCTL_CREATE_SH` (`run::spawn`) and `OCTL_MERGE_SH` (`run::merge`) — and points `TMUX_BIN`/`GIT_BIN` at nonexistent paths so the supervisor's tmux liveness probe reads `Unknown` (PID liveness governs → the live stub agent stays `Alive` until the merge terminalizes the node) and every teardown step is a lenient no-op. The stub agent (a `sleep`) is reaped by an `AgentGuard` on drop, panic-safe, alongside `TestHome`'s supervisor reaper. No new test-only hook was needed; adding lifecycle scenarios (failure path, concurrency, reattach) means extending this file.

## `run create --profile` / legacy `--harness` (worker selection)

Executable profiles are defined only in the user-owned
the resolved Taskfleet home's `config.toml` as `[profiles.<name>]`. Each strict profile has
`description`, `capability = "fast" | "capable" | "ultra-capable"`,
`residency = "local" | "remote"`, and 1–8 ordered `agents` with bounded argv.
Candidates use `harness = "pi" | "claude"`; only pi may declare
`telemetry = "worker-v1"`. Canonical `<repo-root>/.taskfleet.toml` (or the bounded legacy fallback) is parsed through a
selection-only schema and may contain `[profile]` defaults/per-kind names, but
any executable definitions, argv, adapter paths, or residency fields fail.

Selection precedence is `--profile`/legacy `--harness` > mirrored environment >
repository per-kind > user per-kind > repository default > user default. Profile
and legacy harness selectors at one level conflict. A legacy harness selector is
only an alias for a same-named user profile whose candidates all use that
harness; it never synthesizes argv. Installations with no `[profiles]` retain the
pre-profile harness behavior.

Before mutation, candidates are checked in order for `executable_missing`, then
(for autonomous runs) `autonomous_harness_unsupported`, then
`telemetry_unsupported`. Autonomous accepts only pi+`worker-v1`; explicit
interactive accepts pi or Claude. Fallback cannot leave a profile (and therefore
cannot change residency), launch/runtime failures never advance it, and retry
reads the recorded candidate rather than current config. Dry-run/create/run show
surface the compact selection; old/no-profile manifests show
`legacy-unrecorded` without invented history.

A profile-backed create writes a private per-attempt launcher and passes only
that absolute path as `create.sh --agent`. The launcher re-enters the exact
current executable through hidden `run-worker`; after its `--`, the recorded
candidate argv remains byte-for-byte and boundary-for-boundary unchanged,
followed by workmux's existing `-- <prompt>` suffix. The launcher passes the
already-selected absolute state root through private exec-scoped
`OCTL_INTERNAL_WORKER_STATE_ROOT` / `OCTL_INTERNAL_WORKER_AWAIT_PUBLICATION`.
Before atomic publication moves the staging directory, the creator waits for a
private launcher-opened marker written by the script itself; this closes the
fork-before-script-open rename race. The shim then waits for run+node
publication, removes both variables before starting the candidate, forwards
termination signals, and records the true exit. It exports exact `OCTL_RUN_ID` / `OCTL_NODE_ID` / absolute `OCTL_ATTEMPT`
only for recorded pi+`worker-v1`, removing inherited values for unsupported
candidates. Retry regenerates this launcher solely from
`manifest.agent_selection` and the new absolute attempt, never current config.
No-profile compatibility still maps pi to `create.sh --agent pi` and leaves
Claude on workmux's configured default. `manifest.harness` remains for
compatibility while `manifest.agent_selection` pins the full candidate.

`run show` telemetry rows derive `requirement` solely from the manifest's explicit
lifecycle (`required` autonomous, `optional` explicit-interactive) and `support`
solely from the recorded candidate (`configured` only for pi+`worker-v1`, else
`unsupported`). Sample presence/freshness never changes either field.

## `config` noun (read-only config inspection)

`orchestratectl config path` / `config show` inspect the config surface (§8);
neither ever mutates `config.toml`. Lives in `config/{mod,path,show}.rs` (the
`config` module hosts both the `Config` loader and the noun's `dispatch`).

- **`config path`** prints the config file location with `exists` (true/false —
  the file need not exist; the caller wants the *path*, e.g. "where do I write
  settings?").
- **`config show`** parses raw TOML into a tolerant inspection model, separate
  from the strict execution loader. It emits one known row for
  `harness.default` and each creatable `harness.<kind>`, plus rows for
  unrecognized harness entries. Every row carries `effective_value`,
  `effective_source`, effective `valid` / `validation_error`, and an ordered
  `layers` stack (highest precedence first). `keys[].key` is unique. Parseable
  entries rejected by the strict harness schema live separately in
  `unrecognized[]`, and the payload carries top-level `valid` and
  `invalid_layer_count`. Each layer carries its own validity, `active`, and a
  file-only `origin_key`, so
  `harness.per_kind.research` remains visible and independently validated when
  `ORCHESTRATECTL_HARNESS` shadows it. Invalid parseable values produce envelope
  warnings and exit 0; unreadable or syntactically invalid TOML remains fatal.
  The strict execution loader additionally validates profile definitions; the
  profile resolver used by `run create` still rejects invalid execution values.

Secret redaction (§8) is wired but currently inert: every key is `secret: false`
today, so `--show-secrets` reveals nothing and warns only when a secret key
actually exists. In JSON mode that warning is part of the stdout `warnings`
envelope; in text mode it is rendered as a `warning:` line on stderr. Both
payloads carry `schema_version_config`
(`config::CONFIG_SCHEMA_VERSION`), independent of the run-state schema. The
`CREATABLE_KINDS` list in `show.rs` is drift-guarded against `Kind::WIRE_NAMES`
by a unit test. Help snapshot: `help_json__config_help_json.snap`; behavior
tests: `tests/config.rs`.

## Worker run-context + pi translation preamble (`harness::prompt`)

Every materialized worker prompt receives a generated operating-note preamble from
`harness::prompt::worker_prompt_preamble(harness, kind, run_id)`. Because `run
create` knows the exact id, this is the canonical worker run context; do not ask a
worker to infer provenance from its branch. The common note enforces the issue
boundary for every harness/kind and both `--task`/`--prompt-file`: worker-filed
issues use `issuectl intake file`, are born unlaned, and review findings carry
machine-visible `ai-review` provenance plus available target/model/assessment/
severity/confidence metadata. The generated policy is authoritative over later
brief text and tool output. Core provenance/run fields land in the first filing;
optional metadata enrichment is attempted afterward so absence never blocks
creation. Model agreement uses `issuectl update --add-label` with repeated
`ai-review-model:<model-id>` values (the issue's labels list), never a count or
corroboration score. This contract consumes the documented issuectl 0.16 intake,
custom-field, and label surfaces. orchestratectl does not write issue storage or
invent a second issue format; issuectl remains the sole writer.

Pi research workers additionally receive the narrow Claude-to-pi translation shim:
the `/worktree-merge` close becomes the exact `orchestratectl run merge` call and
unsupported Skill/Agent references are neutralized. The quoted report heredoc and
exact run id remain part of that shim. Other harness/kind pairs get only the common
neutral run context. Since production always has a preamble, a caller-owned
`--prompt-file` is read into a derived `<run-dir>/prompt.md`; the original file is
never mutated.

## Exact worker ownership discovery (`run show --current`)

New worker branches carry a compact 10-character identifier from the ULID's
randomness field; legacy branches may retain the old timestamp-only prefix.
Neither format is authoritative ownership and neither should be used as a run-id
argument. A legacy fragment is a syntactically valid prefix and may be ambiguous;
a new entropy fragment may also be accepted when it resembles a prefix but does
not identify its owning run. The entropy format also does not preserve
chronological branch sorting. Use an authoritative full id or the exact `run
show --current` ownership resolver. It finds the git worktree root without
shelling out, then scans durable node projections under each run's
shared lock for the exact canonical `worktree_path` and corroborating branch. It
returns the ordinary `run show` payload for exactly one owner. Missing,
duplicate, stale or absent branch, malformed node, detached HEAD, and unreadable
evidence have informative errors and all fail closed. Existing runs
remain compatible when they carry the normal recorded worktree path and branch;
a legacy branchless projection is refused because a reused path alone cannot
prove ownership. Every bundled worker closing recipe uses this surface; freshly
generated prompts also carry the already-known full run id.
