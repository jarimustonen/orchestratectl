# orchestratectl

Rust CLI for orchestrating autonomous AI-agent workflows on a developer's
machine. It spawns agents into isolated git worktrees (kinds: `spinoff`,
`research`, `technical-decision`, `fan-out`), supervises them with a
per-run supervisor process, and merges their work back via `run merge`.
State is file-based under `~/.orchestratectl/runs/<run-id>/` (append-only
`events.jsonl` + flock-guarded projections), so any UI can present the
same canonical source of truth. The orchestration semantics ship as
bundled skills (`/worktree-spinoff`, `/worktree-research`,
`/worktree-technical-decision`, `/fan-out`, `/stint-start`,
`/stint-handoff`, …) installed with `orchestratectl skill install`.

The 0.2 series is the "thin supervisor" simplification (ADR
`docs/decisions/0001-thin-supervisor-vs-harden.md`): told-not-guessed —
`run merge` is the only success truth, terminal outcomes are typed tables,
and the old inference heuristics (activity clocks, git-reconcile probes,
kind-derived interactivity) are deleted. The former TUI/discussion/
orchestrate/code-pipeline surfaces were cut in that release; do not
resurrect them without a new decision.

## Tool family (the stack this repo belongs to)

orchestratectl is one component in a family of AI-first CLIs that share the
same conventions. What each one owns in THIS repo:

- **issuectl** — issue tracking. Owns `issues/`, `issues/AGENTS.md`,
  `.issuectl/AGENTS.md`, and the `/issue` skill. The execution DAG
  (`lane:`/`lane_seq:` frontmatter, `issuectl dag`) is the scheduling
  source of truth.
- **ossctl** — OSS release engine. Owns the approved `OSS-RELEASE.md`
  contract and the `/oss-*` skills. **Its multi-crate blocker
  (`release-rust-workspace-multicrate` in `~/Sources/ossctl`) is now
  `done`**, and `ossctl release plan --bump <level>` advertises exactly the
  steps this project still does by hand (version + intra-workspace pin
  rewrites + `Cargo.lock` refresh + CHANGELOG finalize), while
  `ossctl release verify <run-id>` reconciles against the registry. Whether
  it can cut THIS two-crate workspace end-to-end is **unverified** — until
  someone proves it, releases are still cut by hand per the release bullets
  below. Verifying this is tracked as `adopt-ossctl-release-cut`; prefer
  closing it over cutting more by hand.
- **project-canon** — ships the `/ai-first-cli-canon` skill (see below).
- **glasspad** — publishes rich HTML views to Jari's browser
  (`glasspad publish`); used for dashboards/reports, not part of the build.
- **intakectl** — routes bug/feature intake (e.g. Jari via Telegram) into
  `intake-{bug,feature}-orchestratectl-*` issues in this repo; they arrive
  with `needs-triage` and are folded via `/stint-handoff` /
  `/triage-unlaned-issues`.

## CLI Design Principles

Use the `/ai-first-cli-canon` skill shipped by `project-canon` as the maintained AI-first CLI canon. It is the binding reference for CLI surface work: strict input validation, `--json` output, JSONL logs, no interactive prompts, informative errors and composable commands. Do not keep or edit a repo-local `ai-first-cli-canon` copy; update the canon in `~/Sources/project-canon` and reinstall the skill from the released tool.

## Gitignored directories

- `history/` — agent scratchpad and ephemeral planning docs (not tracked)
- `target/` — Rust build artifacts

## Documentation Pattern

Every directory follows this structure:

- `CLAUDE.md` — symlink to `AGENTS.md`
- `AGENTS.md` — all AI-relevant info (consolidated)
- `AGENTS-<TOPIC>.md` — complex topics split out (optional)

## Issues & Planning

Issue tracking is managed by [`issuectl`](https://github.com/jarimustonen/issuectl). Use the `/issue` skill (installed by `issuectl init`) to create, search, update, and close issues.

- `issues/<slug>/item.md` — every issue and epic (flat layout — no numeric prefix, no `open/closed/` split)
- Status lives in the `status:` frontmatter field, not in the path
- `issues/AGENTS.md` — issue schema, types, workflow (owned by issuectl)
- `.issuectl/AGENTS.md` — repo-local policy for AI agents (owned by issuectl)

All planning documents (plans, analyses, validations, designs, breakdowns, todos) belong under their parent issue directory — not as standalone files. If work needs a planning document, it also needs an issue.

- `issues/<slug>/plan.md` — architecture, implementation plans
- `issues/<slug>/analysis.md` — research and analysis
- `issues/<slug>/validation.md` — design assumptions checked against current reality, noting what differs from first-pass analysis
- `issues/<slug>/design.md` — design documents
- `issues/<slug>/breakdown.md` — epic → child-issue breakdown with dependencies and critical path
- `issues/<slug>/todo.md` — task checklists

## Operating policy (for `/stint-start` and orchestrators)

Read by `/stint-start` Phase 0 (the round engine; `/stint` was split 2026-08-04 into
`/stint-start` + `/stint-handoff`, with bug intake decoupled to homebase `/triage-bugs`).
Every project-specific fact an orchestrator needs:

- **Release cadence (default posture, since 2026-08-04).** This project **ships real releases, often** — cut one whenever something production-ready lands, don't batch changes into a big release. Two channels, both driven from the approved `OSS-RELEASE.md` contract and **both triggered by pushing a `vX.Y.Z` tag**: **crates.io** via `publish-crates.yml` in CI (`octl-core` first, then `orchestratectl` — the CLI depends on `octl-core = "=<version>"`; do NOT run `cargo publish` locally, see the dedicated bullet below), and **prebuilt binaries + the per-tool Homebrew tap `jarimustonen/orchestratectl`** via cargo-dist (CI builds the mac target on the self-hosted `hauis` runner). End state: `brew install jarimustonen/orchestratectl/orchestratectl`. The `/oss-release` skill orchestrates the whole thing; mechanics live in `OSS-RELEASE.md` + `dist-workspace.toml`. Before a release, finalize `CHANGELOG.md` (`[Unreleased]` → dated version).
- **In-tool "deploy" (a stint's local reflection).** Distinct from a release: only the **orchestrator on the integrated source branch** may reflect a CLI-surface / SKILL change in the running binary. Run `cargo install --path crates/octl-cli --force --locked`, then assert `[ "$(orchestratectl version --output json | jq -r .data.commit)" = "$(git rev-parse HEAD)" ]`, then run `orchestratectl skill install --force && orchestratectl doctor` (expect 0 fail / 0 warn). The commit equality check is load-bearing: a plausible version string does not prove which commit produced the binary. **`--locked` is mandatory, not optional** (2026-08-16): without it `cargo install` re-resolves dependencies from scratch instead of using the workspace `Cargo.lock`, pulls `time 0.3.55` (requires rustc 1.88) against our 1.85 MSRV floor, and fails to compile. Chain the steps with `&&` **without** piping to `tail`/`head`, so any failure stops the deploy. Verify `ls -l ~/.cargo/bin/orchestratectl` too. `~/.cargo/bin` precedes `/opt/homebrew/bin` on `PATH`, so a missing Cargo-installed binary silently falls through to a stale tap build rather than failing loudly; this is why "does it run?" and version-only checks are insufficient. **Workers must never run `cargo install --path …`, `cargo install orchestratectl`, or `cargo uninstall`:** those mutate the user's global toolchain and are orchestrator actions only. A worker exercises its own build with `cargo build --release` and invokes `./target/release/orchestratectl …` explicitly from its worktree.
- **Release / deploy autonomy — fully autonomous, no ask (Jari, 2026-08-05).** local rebuild is always fine. **Pushing `main` is always allowed for this project (no ask)** — this deliberately overrides the global "never push without being asked" default for orchestratectl. **Cutting a release is ALSO fully autonomous now: the agent may push a `vX.Y.Z` release tag without asking, and may decide independently when a release is warranted** (per the release-often cadence above). The former `/oss-release` **approval boundary is removed** for this project — there is no remaining human-confirmation gate on the release action. *(Autonomy is unchanged; what changed 2026-08-17 is the mechanism — the tag push is now the single release action, because CI does the publishing. See the "DO NOT `cargo publish` locally" bullet below.)* The agent still executes it *correctly and deliberately* (right version, changelog finalized, snapshots regenerated, tree clean, **main CI green on the commit being tagged**) — autonomy means no permission prompt, not less care. crates.io publishes remain permanent (yank-only), so verify before publishing; that is a correctness duty, not an approval gate.
- **DO NOT `cargo publish` locally — pushing the tag IS the publish (2026-08-17).** `.github/workflows/publish-crates.yml` is tag-triggered (`v[0-9]+.[0-9]+.[0-9]+*`) and already publishes **both** crates to crates.io in dependency order from CI, using the repo's `CARGO_REGISTRY_TOKEN` secret — no local token, no hand-ordered two-crate sequence. A local `cargo publish` duplicates it; the CI job then only *looks* green because it tolerates "already exists on the crates.io index" as success. The correct sequence is: finalize CHANGELOG + bump the workspace version + the `octl-core` pin + regenerate the `version_*` insta snapshots → commit → push → **wait for main CI green on that commit** → push the `vX.Y.Z` tag → CI publishes the crates (`publish-crates.yml`) and builds the binaries/tap (`release.yml`) in parallel. **The tag push is the irreversible act** (crates.io is yank-only), so it is the step that must be gated:
  ```bash
  sha="$(git rev-parse HEAD)"
  for _ in $(seq 60); do
    id="$(gh run list --workflow ci.yml --branch main --commit "$sha" --event push --limit 1 --json databaseId -q '.[0].databaseId')"
    test -n "$id" && test "$id" != null && break
    sleep 5
  done
  test -n "${id:-}" && test "$id" != null || { echo "no main CI run for $sha" >&2; exit 1; }
  gh run watch "$id" --exit-status && git push origin "vX.Y.Z"
  ```
  The SHA and workflow filters prevent a concurrent push or an older run from producing a false green. `--exit-status` and `&&` are load-bearing: a red run cannot reach the tag push. (Same discipline as the never-`| tail`-an-exit-status rule above.) **Defense in depth for crates.io:** `publish-crates.yml` repeats main CI's full gate, verifies that the tag matches the workspace version and `octl-core` pin, and makes the publish job depend on every gate job. Therefore, even an incorrectly pushed tag cannot publish crates from a red commit. Cargo-dist's `release.yml` runs independently, so the pre-tag main-CI check remains load-bearing for the binary and tap channel. Context: v0.2.2 was published from a local `cargo publish` **before** CI had reported on the commit it contained; CI then went red (a test-only defect, so the release was unharmed, by luck rather than process).
- **Verifying a publish landed — use `ossctl release verify`, or send a `User-Agent` (2026-08-18).** The crates.io API **rejects requests with no `User-Agent`**, and the rejection reads as an empty result rather than an error: `curl -s https://crates.io/api/v1/crates/<crate> | jq -r .crate.max_version` prints `null`, which looks exactly like "the publish did not happen". This produced a false alarm on the v0.4.0 cut (both crates had in fact published). Prefer `ossctl release verify <run-id>` — registry reconciliation is ossctl's job and it sets a proper agent itself. When checking by hand as an interim, pass one: `curl -s -H "User-Agent: <something-identifying>" https://crates.io/api/v1/crates/<crate>/versions | jq -r '[.versions[].num][0]'`. Never conclude "not published" from a UA-less probe.
- **Staying in sync (no ask).** You may always run the `pull → rebase → push` sequence to keep local `main` reconciled with `origin/main` and published — `git pull --rebase` (or `git fetch && git rebase origin/main`), then `git push`. Parallel worktree sessions push under you, so bringing local up to date and pushing before/after a round, before a handoff, or before preparing a release is expected and safe (it rebases your local commits onto the remote first). Both the sync and the push need no asking, and (per the release-autonomy point above) neither do the release actions — nothing in this project's normal git/release flow requires a human-confirmation gate anymore.
- **Green gate (run before merging any worktree):** run the same commands as CI: `cargo fmt --all --check`; `cargo clippy --locked --workspace --all-targets -- -D warnings`; `cargo nextest run --locked --release --workspace`; `cargo test --locked --release --workspace --doc`; and `RUSTDOCFLAGS="-D warnings" cargo doc --locked --workspace --no-deps`. The orchestrator or machine setup installs nextest once with `cargo install cargo-nextest --locked` if the development machine lacks it; a worker reports the missing prerequisite rather than installing globally. This installs the test runner, not orchestratectl. Doctests are a separate step because nextest does not run them. CLI-surface / bundled-skill changes also need the insta snapshot loop, including review of every accepted change; see `crates/octl-cli/CLAUDE.md`. The rustdoc step is separate because tests and clippy do NOT catch dangling intra-doc links. **A symbol-removing cut MUST run the doc check:** deleting a module/fn leaves any `[`crate::…`]` doc-link to it dangling and makes CI's `docs` job red. **A developer machine is NOT a bare CI runner:** a test that depends on ambient `tmux`, an installed harness binary, or any other undeclared tool can pass locally and fail in CI. Exercise tool-sensitive tests with a stripped `PATH` containing only the explicitly required toolchain/stubs, or run them in an equivalent clean environment, rather than treating the fully equipped local host as evidence. (2026-08-15: `cut-run-kinds-discussion-machinery` removed `spinoff::approve` but left an intra-doc link to it in `proc.rs`; the round's test/clippy gate was green, `main` docs went red — `ci-red-main`. Fixed same session; the doc check is now part of every gate to prevent recurrence.)
- **Integrated gate (run after a multi-worktree round, before deploy):** re-run `cargo nextest run --locked --release --workspace` and `cargo test --locked --release --workspace --doc` on the *integrated* `main` once all the round's branches have landed. Per-worktree green does NOT imply integrated green: a test-isolation flake can stay latent until several workers' tests coexist in one run. (2026-07-25: five workers each passed their own gate, but `supervise::notify::tests::fires_hook_with_completion_env` — an order-dependent TOCTOU on an async hook file — failed only in the combined suite; caught by the integrated gate, fixed as `notify-test-toctou-flake`.) The gate also catches a distinct failure mode: a **lane misprediction** — a DAG lane assignment predicts an issue's *likely*-touched hot files, but a fix can legitimately land elsewhere, so two "disjoint-lane" spinoffs can silently collide. (2026-08-10: `supervisor-dies-before-worker-node` (Lane A, predicted `supervise/*`) actually landed in `run/*` — `run list`/`run show`/`RunSummary` — colliding with `run-wait-still` (Lane E); each was green alone but integrated `main` failed to **compile** (`E0425 stillborn`). Caught by the integrated gate, fixed via a follow-up spinoff.) Lesson: never skip the integrated gate for "independent" parallel units, and prefer sequencing any two units that might both touch the `run show` / `RunSummary` DTO surface.
- **Hot / correctness-sensitive files (sequence edits; never parallelize worktrees that touch the same one):** `crates/octl-core/src/{events,lock,reducer,schema}.rs` and `crates/octl-cli/src/supervise/*`. (The code-pipeline modules `crates/octl-cli/src/{floor,pipeline}/*` and the harness heavy layer were DELETED 2026-08-14 by `cut-pipeline-floor-harness-heavy`; only the light `harness/{mod,prompt,select,support}.rs` claude+pi launcher remains — no longer a hot cluster.) See "State integrity invariants".
- **MSRV / `time` pin (standing constraint).** RUSTSEC-2026-0009 (`time` stack-exhaustion DoS) is fixed only in `time ≥0.3.47`, but every such version requires rustc 1.88 > our 1.85 MSRV floor. `time` is transitive-only (via `tracing-appender`; we never parse untrusted time input, so the advisory is not exploitable here). Resolution: `time` pinned to `0.3.41` + a scoped `deny.toml` ignore of RUSTSEC-2026-0009. Re-evaluate (unpin + drop the ignore) if/when MSRV moves to ≥1.88. Corollary: bumping a dep to clear a `cargo-deny` advisory can silently blow the MSRV — always re-check the `msrv (1.85)` CI job, not just ubuntu.
- **Coding happens in worktrees, never in the orchestrator/stint session.** Spawn `/worktree-spinoff <issue-slug>` (headless for batches > 3; see the macOS PTY note). **Verify every landing from git** — `run` status can lag reality.
- **Verify against the running binary before spending a worker.** Bug reports (intake or old issues) routinely describe a defect the current release already fixed, or a read-surface mistake (`last_report` vs `report`, `data.runs[]` vs `data.<field>`). Reproduce against the installed binary first; three stints in a row this closed issues without code.
- **Filing bar for review residuals.** An automated review pass that files every "deferred residual" manufactures un-work. A residual becomes an issue only with (a) an observed occurrence or (b) a self-contained, readable description — never a bare pointer to a gitignored `history/` file.
- **Worker deaths are transient — retry with harvest.** Re-spawn and adopt the preserved branch (review → adopt → complete → merge), never hand-merge unreviewed work. Heavy-LLM units legitimately take 54–96 min; a long run is not a hang.
- **Test accounts / reset:** n/a (no external test accounts).

## Harness boundary: non-blocking waits are NOT an orchestratectl dependency

Decided 2026-08-16 by **homebase ADR 0011** (`~/Sources/homebase/docs/decisions/0011-pidev-background-process-runtime.md`, status Accepted), which supersedes this repo's closed `pi-background-jobs-extension` issue. Two separate lifecycles, and orchestratectl owns only one of them:

- **Interactive, session-scoped** background commands in a pi.dev TUI session are homebase's concern. Its runtime is the pinned third-party extension **`@aliou/pi-processes@0.10.9`** (conditionally adopted, gated on a smoke matrix; `pi-background-tasks` and a custom-built extension were both rejected). Processes it manages die with the pi session — that is a safety property, not a defect.
- **Durable, harness-neutral** background running is the separate `orx-background-runner` work (tracked in homebase, blocked on the smoke gate). Its contract is start / status / bounded logs / stop / bounded wait over runner-owned job metadata.

**The binding constraint for this repo:** `orchestratectl` MUST NOT import `@aliou/pi-processes`, reach into its manager object, assume its process ids or log paths, or send/receive its in-process EventBus events. A pi extension's internals are not a public interface. orchestratectl stays the run-state owner and keeps exposing `run wait`, the `landed` flag, and the JSON contracts; any future non-blocking adapter sits behind the neutral runner contract, never on a harness-specific substrate. Do not re-file a "build our own pi background-jobs extension" issue here — that option was evaluated and rejected.

## Spinoff workflow + lifecycle

Use `/worktree-spinoff <issue-slug>` for bug fixes / improvements; the bundled SKILL handles the whole loop end-to-end: spawn → work → merge (`orchestratectl run merge`) → self-cleanup (tmux window + worktree + branch all gone). Same for `/worktree-research`, `/worktree-technical-decision`, `/worktree-bug-analysis` (read-only analysis), and `/fan-out` units. For hands-on review, create the run with `--interactive` — the supervisor then never auto-terminalizes and waits for an explicit `/worktree-merge` (`run merge`) or `run cancel`. (The pre-0.2 `worktree-code` / `worktree-bugfix` / `worktree-make-skill` kinds and skills were cut; interactivity is the `--interactive` flag, not a kind.)

After any CLI surface or SKILL.template.md change lands on the integrated source branch, the **orchestrator** re-deploys so the running binary and on-disk skills reflect the edit. A worker never performs this step:

```bash
cargo install --path crates/octl-cli --force --locked   # orchestrator only; --locked is mandatory
expected="$(git rev-parse HEAD)"
actual="$(orchestratectl version --output json | jq -r .data.commit)"
test "$actual" = "$expected"
orchestratectl skill install --force
orchestratectl doctor   # confirms skill.sync.* ok for every entry
```

For parallel spawn batches, set up a `Monitor` watching `orchestratectl event tail <run-id> --follow` filtering `node\.report|run\.status|supervisor\.exited` so completions arrive as notifications instead of requiring polling.

### Never `pkill` a supervisor without verification

Twice in one session this rule was learned the hard way: `pgrep -lf "orchestratectl supervise"` finds processes from EVERY repo and every user-owned project, not just yours. Before killing anything:

1. Run `tmux list-windows -a` and look at the emoji prefix on each `wt-*` window — it identifies the source project (🏠 home, 🥨 dpad, 🎬 orchestratectl, etc.).
2. Run `git worktree list` in the **right repo** to see if the run's worktree is one yours.
3. Prefer `orchestratectl run cancel <run-id>` over `pkill` — graceful, triggers the supervisor's cleanup path, leaves no orphans.
4. If you must `pkill`, scope it: `pkill -f "orchestratectl__worktrees/.*supervise"` only kills supervisors built from inside a deleted worktree's debug target — never touches `~/.cargo/bin` production supervisors.

## macOS PTY constraint

macOS limits concurrent pseudo-terminals; ~5–6 simultaneous worktree spawns can hit `fork failed: Device not configured` from tmux. Symptom: `create.sh` fails with `workmux-add-failed` mid-batch.

Use `--headless` (or `--tmux-session <name>`) on `orchestratectl run create` to spawn into a detached tmux session that doesn't consume a foreground PTY. Mandatory for `/fan-out` of N≥5; recommended for any parallel `/worktree-spinoff` batch larger than 3. Attach later with `tmux attach -t headless` to inspect.

## State integrity invariants

These seven invariants govern correctness of the on-disk run state and the autonomous-spinoff loop. The first five were established by the 2026-06-29 pre-publication campaign; six (merge-transaction recovery) and seven (typed terminal outcomes) landed with the thin-supervisor 0.2 work (A2 / A6). They are easy to violate from inside a hot code path without realising it. Read them before touching the reducer, the lock layer, or the `run merge` / supervisor cleanup paths.

1. **`applied_seq` watermark**
   (`crates/octl-core/src/events.rs`)
   The reducer advances `manifest.applied_seq` only after every projection an event touches has been fsynced. On the next lock acquisition, events with `seq > applied_seq` are replayed before any new append. Any new event-appending path MUST go through the `LockedRun` witness and the `append_and_apply_*` API — never call `write_*` projection helpers directly.

2. **`LockedRun` witness**
   (`crates/octl-core/src/lock.rs`)
   Compile-time proof that the caller holds the run flock before calling `append_event_with_seq` / `append_and_apply_unlocked`. Don't add `#[allow(...)]` to bypass; thread the witness through.

3. **`LOCK_SH` on every multi-file read path**
   (`crates/octl-core/src/lock.rs::with_shared_lock`)
   Every reader that touches more than one of `manifest.json` / `nodes/*` in one decision wraps the scan in `RunLock::with_shared_lock`. The reducer holds the exclusive lock while it writes; without the shared lock a reader can observe a half-applied projection set. Don't add new readers that skip it.

4. **Progress polling branches on `manifest.status`, NOT `lifecycle`**
   (every `crates/octl-cli/skills/*/SKILL.template.md`, and any agent prose elsewhere)
   `Lifecycle` is `Autonomous | Interactive` — a *how-run category* set once at `run create` from the explicit `--interactive` flag (issue `interactive-flag`), never transitions and NOT derived from `kind` (the removed `code` kind used to carry it accidentally; `Kind::lifecycle` now only *seeds the default* for a non-`--interactive` create). `Status` is `Pending | Running | Done | Failed | Cancelled` — terminal states are `Done | Failed | Cancelled`. An agent that polls `lifecycle` for `completed | failed | cancelled` hangs forever; the field never matches. This was a real bug (`skill-progress-polling-wrong-field`); never re-introduce it. Do NOT resurrect kind-derived `Lifecycle::Interactive` inference — interactivity is an explicit told flag on any topology, and in interactive mode the supervisor never auto-terminalizes/tears-down from a dead pid or worker exit — it waits for explicit `run merge` / `run cancel` (design.md §6).

5. **Supervisor is the canonical worktree + tmux teardown actor**
   (`crates/octl-cli/src/supervise/cleanup.rs`)
   `merge.sh` no longer touches tmux or `git worktree remove` — the supervisor sees the terminal `node.report`, rolls the run up via `rollup_status`, and tears down. `find_window_by_path` is **session-scoped + exact-cwd-match**: it queries only the spawn-session via `tmux list-windows -t <session>` and requires `pane_current_path == worktree_path` (no sub-path prefix). Without these constraints the recovery would kill an unrelated pane that happened to `cd` into the worktree, including the user's master session.

   **Teardown is gated on the terminal outcome — unmerged work preserves the branch + worktree** (the typed outcome table `supervise::outcome::TerminalOutcome::teardown` + the source-relative check, issues `blocked-report-deletes-branch` / `typed-supervisor-outcomes`). Two layers:
   - **Primary gate (typed table, invariant 7):** `cleanup_node` classifies the node's terminal `node.report` via `TerminalOutcome::classify` and acts on the single `Teardown` policy it returns — never a re-derivation from raw signal fields. A blocked handoff OR any non-merge failure (`success: false`, no `via: "explicit-merge"`, not a `cancelled` run-cancel) is `Teardown::PreserveWork`: `cleanup_node` closes its tmux window (winding the run down is fine) but must NOT `git worktree remove` or delete its branch — it records a `cleanup.branch_preserved` audit event instead. Deleting them is silent data loss.
   - **Defense-in-depth (source-relative):** on ANY non-explicit-merge path (a plain success that skipped `run merge`, a `run cancel`, a genuine failure, a future ungated outcome), `cleanup_node` checks `git rev-list --count <manifest.source_branch>..<branch>` **before** touching anything. If the branch has commits not reachable from the run's OWN source branch, it preserves BOTH worktree and branch (`cleanup.branch_preserved`, reason `unmerged commits vs source`). The ancestry check is against the run's recorded source branch, NOT the main worktree's ambient `HEAD` (which may be on any branch when the supervisor ticks). This means a `run cancel` whose agent committed real work now preserves it too. **This check FAILS CLOSED** (issue `non-merge-teardown-dirty-worktree`): if `rev-list --count` cannot be computed (a git error / unparseable output), teardown preserves (`UnmergedCheck::Unverifiable`, reason `unmerged-commit check unavailable`) rather than proceed — the older code returned "nothing unmerged" on a git error and removed the worktree anyway.
   - **Dirty-worktree guard (uncommitted work):** the source-relative check only protects *committed* work. On the same non-explicit-merge paths, `cleanup_node` classifies the tree via `worktree_cleanliness` (`git status --porcelain --untracked-files=all` → typed `Clean`/`Dirty`/`Unverifiable`) **before** removing it; a `Dirty` tree preserves BOTH worktree and branch (`cleanup.branch_preserved`, reason `uncommitted changes in worktree`) and an `Unverifiable` one fails closed the same way but with the distinct reason `worktree cleanliness unavailable (git error)` — never mislabel a git failure as uncommitted work. `--untracked-files=all` is load-bearing: it defeats a repo/global `status.showUntrackedFiles=no` that would otherwise hide an agent's untracked files. So an agent's mid-edit uncommitted scratch is never silently discarded on a cancel/plain-success teardown (issue `non-merge-teardown-dirty-worktree`).
   - **HEAD-relative committed-work guard (detached / stale-branch metadata):** the source-relative check above measures the RECORDED `Node.branch`, so it is blind to a worktree on a DETACHED HEAD (or one whose `Node.branch` is `None`/stale) whose commits live only on the checked-out HEAD, protected by no branch ref — a clean such tree would pass every check above, remove non-force, have no branch to `-d`, and its commits would become unreachable. On the same non-explicit-merge paths `cleanup_node` inspects the ACTUAL HEAD via `head_teardown_safety` (`git rev-parse HEAD` + `git symbolic-ref HEAD`, typed `HeadTeardown::{DeferToBranch,Safe,Preserve}`) **after** the dirty guard. Only a HEAD on exactly the recorded `Node.branch` **defers** (the branch checks + `-d` backstop own it); a DETACHED HEAD **or a HEAD on a branch DIFFERENT from the recorded one** is removed only when its actual oid is reachable from source (`git rev-list --count <source>..<HEAD-oid> == 0`), else it preserves BOTH worktree and branch (`cleanup.branch_preserved`). A non-recorded branch is NOT treated as a durable protector: a merged sibling node can force-`-D` it after this worktree is removed (git only refuses to delete a branch checked out in a LIVE worktree), so its commits must be proven in source (issue `detached-head-teardown-commit-loss`, review finding B). The guard verifies the oid it READ (`head_oid`), never the branch tip from the separate `symbolic-ref` probe, so a HEAD moving between probes can't green-light removal of the observed commit. **FAILS CLOSED**: an unreadable HEAD, an unrecorded `source_branch`, or any `rev-list` git error preserves rather than removes; `Git::rev_list_count` rejects an empty endpoint (an empty `source_branch` would otherwise resolve `..<oid>` against ambient `HEAD` — finding A). **Residual (follow-up `detached-head-teardown-toctou`):** non-force removal re-checks cleanliness but NOT HEAD reachability, so a concurrent `git checkout --detach <new-commit>` between the probe and removal can still orphan the new commit — closing that needs a rescue ref / worktree lease.
   - **Force vs non-force removal (`Teardown::Full` is the only force):** `worktree remove --force` is used ONLY on a confirmed explicit `run merge`. Every non-explicit-merge teardown uses **non-force** `git worktree remove`, which is the atomic TOCTOU safety net: the tree reaching removal was verified clean, but if a race dirtied it (or it is locked / has an initialized submodule) git REFUSES rather than discard the work, and `cleanup_node` records `cleanup.branch_preserved` (reason `worktree not cleanly removable`) and **returns before branch delete** so the branch is not stranded and no misleading `branch_remove_failed` is emitted; a later tick retries. (Follow-up: no back-pressure/escalation on a *persistent* git error — fail-closed can leak a worktree indefinitely, visible via `git worktree list` — tracked separately.)
   - **Last-resort backstop:** only a confirmed `run merge` force-deletes (`git branch -D`); every other delete uses `git branch -d` (refuses an unmerged branch, ambient-HEAD-relative) for the residual case where `source_branch` was unrecorded and the source check could not run. Branch names are passed after `--`.

   **The `run create --notify` completion hook fires on the terminal transition, BEFORE teardown** (`crates/octl-cli/src/supervise/notify.rs`, issue `no-completion-notification-to-parent`). The order in the terminal tick is fixed: fire notify → cleanup → loop-exit, so a hook can observe the run before the worktree/window are gone. Delivery is **at-least-once** (owner's call: a missed completion signal is worse than a duplicate): under one exclusive lock the supervisor scans for a durable `run.notified` marker (idempotency key `supervisor-notify:<run-id>`, scoped by `(kind, key)`) and, if absent, spawns the hook FIRST and records the marker AFTER — so a crash between the two re-fires on restart. Do NOT reorder to record-before-spawn (that is at-most-once and silently drops the notification on a crash). `notify` state is tracked SEPARATELY from `cleaned` (a shared flag silently drops the notification on a transient append failure — a bug caught in review); don't re-merge them. The hook is spawned detached and reaped on a thread so a hung command can't wedge the single-threaded tick.

6. **`run merge` is a recorded, OID-recoverable transaction — never a raw git-then-append**
   (`crates/octl-cli/src/run/{merge,merge_recovery}.rs`, `crates/octl-cli/scripts/merge.sh`, `crates/octl-core/src/{schema,reducer}.rs`; issue `merge-transaction-recovery`, design §2.1b / A2)
   `run merge` spans two durability domains — git refs and the event log — and is not atomic across them. A crash after the git merge but before the terminal `explicit-merge` `node.report` would strand the work *merged in source* with *no merge event* (a false `failed`). So `run merge` records a `merge.started` transaction (`op_id`, `expected_source_oid`, `worker_oid`, source/worker branch, driver pid → `Node.pending_merge`) **before** mutating git; merge.sh guards the source-ref fast-forward with a **compare-and-swap** against `expected_source_oid` (exits `76` → `merge_source_moved` if the target moved) and re-checks driver liveness (`--driver-pid`) immediately before the mutation; and recovery (next `run merge` retry, or the supervisor tick via `merge_recovery::recover_run`) resolves that **one** transaction against **immutable OIDs** — the recorded `expected_source_oid` and `worker_oid`, plus one pinned `source_now`. It **completes** (appends the missing `explicit-merge` report) only when `source_now` moved off `expected_source_oid` AND the recorded `worker_oid`'s content is git-verified integrated into `source_now` (rebase-robust patch-id, via `run::landed`) AND was NOT already integrated into `expected_source_oid`; otherwise it **rejects** (`merge.aborted`, work preserved). Recovery runs only when the driver process is confirmed dead (with a staleness bound when no start-time identity was recorded), so a live merge is never raced; the entry path refuses (`merge_in_progress` / `merge_recovery_unverifiable`) rather than overwriting a live or unverifiable transaction; a failed/conflicted merge and any terminal node.status/report clear `pending_merge` so none dangles; and a durable-record failure fails the merge closed BEFORE any git mutation. This is scoped to ONE recorded transaction and pinned to immutable OIDs — distinct from the deleted git-reconcile probe (which scanned every branch every tick). It is NOT a fully atomic cross-domain commit: the CAS is check-then-FF under the merge lock (a non-cooperating writer between check and FF, or a force-push between classify and append, is a documented residual), and the orphan-child window is bounded, not closed — the durable operation lease that closes both is deferred to 0.2.1 (design §2.7). Don't reintroduce a broad "branch is an ancestor of source ⇒ done" inference, don't verify against the mutable worker *branch* (use the recorded `worker_oid`), and don't append `explicit-merge` without the CAS-guarded git mutation having demonstrably landed.

7. **Terminal outcomes are a typed table, never inferred from a signal cross-product**
   (`crates/octl-cli/src/supervise/outcome.rs`, consumed by `supervise::cleanup::cleanup_node` and `supervise::watchdog_tick`; issue `typed-supervisor-outcomes`, design §2.6 / A6)
   `run merge` is the only **success** truth, but not the only **terminal** truth. The supervisor no longer guesses done-ness from a cross-product of proxies (pid × pane × branch × report × activity clocks). Two small, pure, exhaustively-tested tables carry it: `TerminalOutcome::classify(&Node)` maps a terminal `node.report` to one typed outcome (`Merged` / `Blocked` / `Failed` / `Cancelled` / `PlainSuccess`) and `TerminalOutcome::teardown` maps that to the single `Teardown` policy it authorizes (`Full` = confirmed explicit `run merge`, force `-D`; `PreserveWork` = blocked handoff OR any non-merge failure, preserve branch+worktree; `SourceRelative` = cancel or a plain success that skipped `run merge`). `cleanup_node` (invariant 5) reads that table — it never re-sniffs `last_report` JSON. The **deleted** heuristics: the git-reconcile-implies-done probe + synthetic `merge-reconciled` success, the three activity clocks (commit-time / pane-mtime / CPU-rate) + the idle-unmerged synthesizer, and the tmux tri-state / streak-gating as a *primary* liveness signal. Don't reintroduce any of them, and don't add a teardown branch that bypasses `TerminalOutcome::teardown`.

   **PID liveness is now ONLY the residual crash backstop** (design §2.1a). The primary completion signal is the A1 told `worker.exited` fact (a recorded exit — clean or failing — short-circuits the pid path). Pid liveness fires `failed` only when the worker is confirmed gone (`Dead`/`Recycled` — a lost `worker.exited`: hard kill of the shim, host death) AND no merge, AND a **fixed, persisted post-death grace** has elapsed. The grace is anchored to the durable, monotonic `Node.first_death_at` (set by a `node.death_observed` event, first-write-wins, cleared on `node.retry`) so it survives a supervisor restart; the backstop re-reads under the exclusive lock and re-checks `worker_exit`/report/status before appending, so an exit/merge that landed in the grace window wins. `OCTL_DEATH_GRACE_SECS` overrides the ~5s default (tests set `0` to fire on the same tick). A clean exit (`code == 0`) with no merge stays **non-terminal** (attention-required) — the worker finished but skipped `run merge`; never auto-fail it.

### Related conventions

- **Concurrent spinoff reports** — bundled SKILLs use `/tmp/node-report-${run_id}.json`, never the shared `/tmp/node-report.json`. Drift re-introduces the clobber race.
