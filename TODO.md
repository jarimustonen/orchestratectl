# TODO

Currently open work — what to do, in what order, why.

For longer-running planning + design docs see `issues/<slug>/{plan,design,breakdown,validation,handoff,decisions}.md`. This file is the **session-level** plan and points at issuectl issues for the actual tracked work.

---

## HANDOFF — v0.1.0 is prepped; only the publish itself remains (2026-07-04)

**One-paragraph state.** v0.1.0 is fully prepared, gated green, and the release pipeline is proven end-to-end on the new self-hosted macOS runner. The ONLY thing left is the actual publish, and the two hard steps are Jari's (they need his GPG key + crates.io token — not doable from hauis). The next agent's job is to (1) get `main` pushed, (2) support Jari through the signed tag + `cargo publish`, and (3) run the post-publish smoke tests. Nothing is code-blocked.

### Exact repo state (verify first)

- **HEAD = `f3d9f46`** (a doc commit). **Release commit = `fead26e` "release: v0.1.0"** (2 commits down; tree is version `0.1.0` at both).
- **`main` is AHEAD of `origin/main` by 4 commits and NOT PUSHED.** Pushing is Jari's call (his CLAUDE.md) — ask before pushing. Pushing `main` does NOT fire a release (release.yml triggers only on a *tag* push).
- `orchestratectl version` from `~/.cargo/bin` currently reports `0.0.2-alpha` (the installed binary predates the version bump — that's fine; the bump lands via `cargo publish`, or reinstall locally to smoke it).
- **Alpha tag `v0.0.2-alpha` is on origin at `5e7453c`** with a green GitHub pre-release (run `28693213840`) — the pipeline verification. Do NOT delete it.
- **Open issues: 3, all normal-priority, all carried to v0.2** (non-blocking): `cancel-dead-supervisor-recovery`, `legacy-pid-identity-check`, `teardown-gate-trust-and-lifecycle`. Documented in CHANGELOG "Known gaps".
- **Self-hosted runner `hauis` is online** (`gh api repos/jarimustonen/orchestratectl/actions/runners` → `status: online`). launchd service, survives reboots.

### What landed this session (all merged to local main)

1. **`supervisor-dead-merge-no-teardown`** (`979b794`+`62948c8`) — `run merge` auto-reattaches a dead supervisor; `supervisor:{pid,alive}` on show/list; `supervisor:{state}` on merge. Never silent.
2. **`run-create-agent-startup-timeout`** (`e6df5d8`) — `run create --agent-startup-timeout` [1,600], default 90s, forwarded to create.sh. Fixed the spawn-under-load failures that plagued the session.
3. **`blocked-report-deletes-branch`** (`fe44a56`+`498cf5d`, HIGH silent-data-loss) — a `success:false` terminal report now PRESERVES the branch+worktree. Gate: `node_report_is_blocked` + source-relative `git rev-list --count <source>..<branch>` net; `-D` force reserved for confirmed `run merge`. Invariant #5 updated.
4. **Self-hosted macOS runner on hauis** + **alpha pipeline verified green** (see task 1 & 2 below). **x86_64-apple-darwin dropped** (hauis is Homebrew-Rust/no-rustup → can't cross-compile Intel).
5. **v0.1.0 prep** (`fead26e`): version bump, `publish=true` both crates, CHANGELOG dated 2026-07-04, snapshots regenerated. fmt/clippy/doc/test all green.
6. **Runner token-leak cleanup** — `actions/checkout` was leaving a short-lived GH token in `~/.gitconfig` (symlink into versioned dotfiles); fixed with `GIT_CONFIG_GLOBAL` in `~/actions-runner/.env` (see task 1's guard note). Also removed a scratchpad `source` line my smoke test left in `~/.profile`.

### Session gotchas the next agent MUST know

- **zsh word-splitting**: `for id in $IDS` does NOT split in zsh — use an array `IDS=(...)`/`"${IDS[@]}"`. Bit the stub-sweep this session.
- **Spawn `run create` with `run_in_background`**: it blocks until the agent launches (can exceed 2 min); a foreground 2-min Bash timeout SIGTERMs it mid-spawn and leaves a 0-node stub (then `run cancel` it).
- **cargo-dist installer**: always pass `--no-modify-path` for smoke tests, else it appends a PATH `source` line to `~/.profile`/shell rc.
- **Watch GitHub runs with a poll loop, not `gh run watch`** (loses auth / rate-limits): `until s=$(gh run view <id> --json status -q .status); [ "$s" = completed ]; do sleep 120; done`.

### Landed this session

- **D2 closure** — `runwriter-batched-append-api` closed as `wontfix` with a v0.2 carry-over line in `CHANGELOG.md`. Deferred because it overlaps with the just-landed `applied_seq` / `LockedRun` / `AppendOutcome` primitives.
- **`flaky-self-terminate-test`** — 23 tests in `supervise_gates.rs` + the `e2e_spinoff` round-trip serialize on a process-wide `#[file_serial(key, path => "/tmp/octl-test-supervise.lock")]` lock. Under `cargo test --workspace` the binaries used to race on filesystem bandwidth; six consecutive workspace runs are now green. Self-terminate deadline also bumped 10s → 30s.
- **cargo-dist E6/E7/E8 scaffold** — `dist-workspace.toml` (GitHub hosting; 4 POSIX target triplets; shell + Homebrew installers; tap = `jarimustonen/homebrew-tap`; `HOMEBREW_TAP_TOKEN` secret in place), `[profile.dist]`, `[package.metadata.dist] dist = true`, `homepage.workspace = true` on both crates, `.github/workflows/release.yml` generated.
- **Crate rename** — `octl-cli` → `orchestratectl` so crates.io, Homebrew formula, and shell installer all align with the binary name every SKILL.md already promises. The on-disk directory stays `crates/octl-cli`; only `[package].name` and the path-dep version field changed.
- **Windows dropped from the dist matrix** — orchestratectl is fundamentally POSIX (`fork`, `setsid`, `sigaction`, tmux). Windows was never a goal; documented in `dist-workspace.toml`.
- **Version 0.0.2-alpha** — workspace bumped from `0.0.1` (was mandatory for the tag to match the version cargo-dist announces).
- **`run wait <run-id>...` subcommand** — new completion-blocking primitive. Flags `--all` (default) / `--any` / `--timeout <dur>` / `--fail-on-error` / `--progress` / `--poll-interval <dur>` / `--output`. Exit codes `0` (wait met) / `1` (usage/unknown) / `2` (timeout) / `3` (`--fail-on-error` + a run failed). Every `worktree-*` SKILL.template.md's "Following progress" section now uses `orchestratectl run wait` instead of the hand-rolled zsh-unsafe `for id in $ids` loop. Closes `run-wait-subcommand` + `skill-multi-run-poll-zsh-unsafe`.
- **`headless-tmux-session-not-torn-down`** — supervisor's teardown path (`crates/octl-cli/src/supervise/cleanup.rs`) now records `managed_tmux_session` on the manifest and, after `cleanup_terminal_nodes`, kills the managed session iff (1) it is managed, (2) not attached by a human, and (3) only synthetic default shell windows remain. Sibling live runs keep the session alive; last run to finish kills it. New behaviour test `tests/headless_session_teardown.rs` drives the full loop on a private tmux socket.
- **`orchestrated-children-hang-pending`** — `--kind orchestrated` children used to sit at `status: pending` / `nodes: []` forever after their commits landed on the integration branch. Root cause: the `orchestrate` **driver** did not spawn its own per-run supervisor, so parent-pointed children's `node.report` events were never consumed. `run merge` succeeded at the git layer but the run never terminalized, no teardown. Fixed by spawning a driver-side supervisor. Also: `run cancel` now tears down worktree + tmux for orchestrated children; historical failures cleaned up separately.
- **Repo flipped to PUBLIC.** No secrets in tree; only public references (`SECURITY.md` mentions `jari@itsellesi.fi` for responsible disclosure).
- **Bug reports filed by orchestrated-hang recovery** — 3 new external issues filed during the session (all in `issues/`): `orchestrated-children-hang-pending` (fixed), plus context in `BUG-REPORT-supervisor-dead-merge-no-teardown.md` (still uncommitted at repo root — needs triage; see below).
- **Homebrew `HOMEBREW_TAP_TOKEN` secret configured** in orchestratectl repo Actions secrets. Classic PAT with `repo` scope, expires 2027-06-29.

### What works for real-world use today

Everything from the previous snapshot, PLUS:

- `orchestratectl run wait <id>...` is the canonical way for any driver to block on a spinoff/orchestrated child. Used inside this session for every spinoff and worked perfectly (exit code 0 on success terminates the caller's background command).
- `/orchestrate` now correctly terminalizes parent-pointed children.
- Empty `headless` tmux sessions no longer accumulate after batch teardown.

`orchestratectl doctor` is `126 ok / 0 fail / 0 warn`. `cargo test --workspace` + `clippy -D warnings` + `fmt --check` + `RUSTDOCFLAGS=-D warnings cargo doc --workspace --no-deps` all green.

---

## What's left before v0.1.0

### 0. Fix `supervisor-dead-merge-no-teardown` — ✅ DONE (2026-07-03)

Fixed and merged (`979b794` + `62948c8`, closed `938f10f`/`60147b6`). `run merge` now auto-reattaches a fresh supervisor when the recorded one is dead — never silent (warning + `supervisor:{state}` outcome), `run show`/`run list` gained `supervisor:{pid,alive}`, serialized e2e test added, full `/llm-review` pass (report in `history/review-supervisor-dead-merge-no-teardown.md`). Two follow-ups filed as issues (`cancel-dead-supervisor-recovery`, `legacy-pid-identity-check`).

### 0.5. Fix `run-create-agent-startup-timeout` — ✅ DONE (2026-07-03)

Fixed and merged (`e6df5d8`, closed `cfda215`). `run create` now takes `--agent-startup-timeout <s>` (clap-validated [1,600]), threaded through `create::Args → SpawnRequest → run_create_sh`, and **always forwards it to create.sh with octl's own default of 90s** (higher than create.sh's 30s because octl batch-spawns self-load the host). Deployed: `cargo install --path crates/octl-cli --force` + `skill install --force` done; `doctor` = 234 ok / 0 fail / 0 warn. **The OCTL_CREATE_SH workaround below is now retired** — spawns use the real 90s default. Keep the workaround note only as historical reference for pre-`e6df5d8` binaries.

### 1. Self-hosted macOS runner on `hauis` — ✅ DONE (2026-07-03)

`actions/runner` v2.335.1 installed on hauis (`~/actions-runner`), configured (name `hauis`, labels `self-hosted, macOS, ARM64`), running as a launchd service (`svc.sh install/start` — survives reboots), **online**. Registered against `jarimustonen/orchestratectl`.

**Key deviation from the original plan — `x86_64-apple-darwin` DROPPED** (Jari's call, 2026-07-03): hauis has **Homebrew Rust, not rustup**, so it cannot cross-compile the Intel target (no x86_64 std). `dist-workspace.toml` now builds 3 targets (`aarch64-apple-darwin`, both Linux) with:
```toml
[dist.github-custom-runners]
aarch64-apple-darwin = "self-hosted"
```
The 2 Linux targets stay on GitHub-hosted ubuntu. Apple Silicon fully covered; Intel-Mac users install from source. To add x86_64-darwin back later: install rustup on hauis surgically (`--no-modify-path`, point only the runner's `.path` at it), `rustup target add x86_64-apple-darwin`, re-add the target + custom-runner line, `dist generate`.

**Public-repo runner safety (verified):** no `pull_request`-triggered workflow reaches the self-hosted runner — ci.yml's matrix is `[ubuntu-latest, macos-latest]` (GitHub-hosted), and release.yml's build job is gated off on PRs by cargo-dist's default `pr-run-mode: plan` (`publishing == false` on PRs). Only a **tag push to upstream** (which forks can't do) reaches hauis. **Do NOT set `pr-run-mode = "upload"` or add `self-hosted` to ci.yml.**

**Runner `GIT_CONFIG_GLOBAL` guard (MUST keep):** on hauis, `~/.gitconfig` is a symlink into the versioned homebase dotfiles repo. `actions/checkout` runs `git config --global http.<url>.extraheader <token>` and, if a job crashes before its cleanup post-step, LEAVES a short-lived GitHub App token in `~/.gitconfig` → i.e. inside a versioned repo working tree (latent secret leak, hit once 2026-07-03). Fix in place: `~/actions-runner/.env` sets `GIT_CONFIG_GLOBAL=/Users/jari/actions-runner/.gitconfig-ci`, so all job `git --global` writes land in a runner-local file, never `~/.gitconfig`. Verified with a probe write. If the runner is ever re-installed, re-add this line to `.env` and restart the service. (Homebase-side alternative Jari may also do: `includeIf`/`[include]` so `~/.gitconfig` isn't a direct symlink to the tracked file.)

### 2. Alpha pipeline end-to-end — ✅ VERIFIED (2026-07-03, run `28693213840`, green)

Retagged `v0.0.2-alpha` at HEAD (`5e7453c`) → pushed → pipeline succeeded end-to-end:
- All 3 build-local jobs green: `aarch64-apple-darwin` **on hauis** (~15 min), both Linux on ubuntu (~2 min). Confirmed hauis went `busy` and picked up the job.
- `announce` created the GitHub **pre-release** with: 3 platform tarballs + `.sha256` each, `orchestratectl-installer.sh`, `orchestratectl.rb`, `sha256.sum`, source archive.
- `publish-homebrew-formula` **correctly SKIPPED** — gated `if: !announcement_is_prerelease || publish_prereleases` (release.yml:287); v0.0.2-alpha is a prerelease so the tap is left clean. **This job WILL run for the non-prerelease v0.1.0** (the only path not exercised by the alpha — HOMEBREW_TAP_TOKEN + checkout of jarimustonen/homebrew-tap are configured; trust-but-verify at 0.1.0).
- Shell installer smoke: installed a working binary reporting `0.0.2-alpha` / commit `5e7453c`, carrying the `--agent-startup-timeout` flag (all session fixes shipped).

**Watch runs with a poll loop, not `gh run watch`** (loses gh auth / rate-limits): `until s=$(gh run view <id> --json status -q .status) && [ "$s" = completed ]; do sleep 120; done` — or the Monitor per-job poll pattern used this session.

If any of these fail, iterate on the alpha before moving to Phase F.

### 3. Phase F — the actual v0.1.0 publish

Same sequence as the previous handoff snapshot. **Don't reverse the order.** Each step depends on the previous.

**Steps 1–5 (prep) — ✅ DONE (2026-07-04), landing in the `release: v0.1.0` commit:**

1. ✅ Open issues are 3 normal-priority follow-ups (`cancel-dead-supervisor-recovery`, `legacy-pid-identity-check`, `teardown-gate-trust-and-lifecycle`) — all consciously carried to v0.2 (non-blocking; noted in CHANGELOG "Known gaps"). No `BUG-REPORT-*` file remains.
2. ✅ `cargo fmt --check` + `clippy --workspace --all-targets -D warnings` + `RUSTDOCFLAGS=-D warnings cargo doc` + `cargo test --workspace` (0 failures across ~600 tests). Version snapshots regenerated (insta, version-string only).
3. ✅ Workspace version `0.0.2-alpha → 0.1.0`; `octl-core` path-dep `=0.1.0` in `crates/octl-cli/Cargo.toml`.
4. ✅ `CHANGELOG.md` dated `[0.1.0] — 2026-07-04` with this session's Added/Fixed folded in; fresh `[Unreleased]`.
5. ✅ `publish = true` on **both** crates.

**Steps 6–8 — JARI's steps (need GPG key + crates.io token; not doable from hauis):**

6. On **gertrud**: `git pull` (get the `release: v0.1.0` commit at HEAD), then `git tag -s v0.1.0 -m "orchestratectl v0.1.0"` (GPG-signed — hauis has no signing key) and `git push origin v0.1.0`.
7. GitHub Actions release workflow runs on the tag — mac build on **hauis** (self-hosted, already verified), Linux on ubuntu, and **`publish-homebrew-formula` runs this time** (not a prerelease → writes `orchestratectl.rb` to `jarimustonen/homebrew-tap`). Watch with a poll loop, not `gh run watch`.
8. `cargo publish -p octl-core`; wait ~30s; `cargo publish -p orchestratectl` (needs your crates.io token).

**Step 9 (smoke) — agent can run once 6–8 land:**

9. Smoke on a clean shell:
   - `cargo install orchestratectl` — from crates.io.
   - `brew install jarimustonen/orchestratectl/orchestratectl` — from tap.
   - `curl -LsSf https://github.com/jarimustonen/orchestratectl/releases/latest/download/orchestratectl-installer.sh | sh` — shell installer.
   - `orchestratectl version` → `0.1.0`; `orchestratectl skill install --force`; `orchestratectl doctor` clean.
10. Announce / archive this TODO into `issues/v0.1.0-release-campaign/handoff.md` and seed a v0.2.0 TODO.

---

## How to start where the previous agent left off (2026-07-04)

1. **Sanity-check first:**
   ```bash
   git log --oneline -3            # HEAD = f3d9f46; release commit fead26e "release: v0.1.0"
   git status -sb                  # main ahead of origin by 4, NOT pushed
   grep -m1 '^version' Cargo.toml  # expect 0.1.0
   issuectl ls --status open       # expect 3 (all normal-priority, carried to v0.2)
   gh api repos/jarimustonen/orchestratectl/actions/runners --jq '.runners[]|{name,status}'  # hauis: online
   cargo test --workspace          # expect green (0 failures)
   ```

2. **First action — ask Jari to push `main`** (or confirm he'll do it). The `release: v0.1.0` commit must reach origin before he can pull+sign+tag on gertrud. Pushing `main` is safe (no release fires — only a *tag* push does) but pushing is Jari's call per his CLAUDE.md, so confirm.

3. **Then walk Phase F (task 3 below).** Steps 6–8 are Jari's (signed tag on gertrud + `cargo publish` — his GPG key + crates.io token; hauis has neither). Step 9 (smoke) is yours once the release + crates + tap are live. Watch the release run with a poll loop, NOT `gh run watch`.

4. **Watching GitHub Actions runs:**
   ```bash
   until s=$(gh run view <id> --json status -q .status) && [ "$s" = "completed" ]; do sleep 120; done
   ```
   For orchestratectl child runs use `orchestratectl run wait <run-id> --timeout 90m --output json`.

5. **If a fix surfaces a new bug:** file it via `issuectl new --title "..." --type bug --priority high --slug <2-3-word-kebab>`, then `/worktree-bugfix` it. Track through issuectl, not this TODO.

6. **After any CLI-surface or SKILL change, redeploy** (else `~/.cargo/bin` + on-disk skills go stale — a silent failure mode):
   ```bash
   cargo install --path crates/octl-cli --force
   orchestratectl skill install --force   # NOTE: overwrites homebase-managed ~/.claude/skills/{worktree-*,fan-out,orchestrate} — by design (orchestratectl replaces that family), but it drifts homebase's copies
   orchestratectl doctor                  # expect 0 fail / 0 warn
   ```

---

## Rough remaining estimate

- Task 0 (`supervisor-dead-merge-no-teardown` bugfix worktree): 30–60 min autonomous.
- Task 1 (self-hosted runner setup on hauis): 20–30 min once the registration token is in hand from Jari.
- Task 2 (alpha pipeline verification): 30–60 min including macOS build time.
- Phase F end-to-end: 1–2 h assuming clean smoke.

**Total to v0.1.0 published: ~3–4 h of focused work.** Nothing path-blocked except the token exchange.

---

## Notable invariants the codebase now relies on

These were established over the two-day handoff campaign and are easy to violate accidentally.

- **`applied_seq` watermark** (`crates/octl-core/src/events.rs`). The reducer advances `manifest.applied_seq` only after every projection an event touches is fsynced; on the next lock acquisition, events with `seq > applied_seq` are replayed. Any new event-appending path MUST go through the `LockedRun` witness and `append_and_apply_*` API — never call `write_*` projection helpers directly.
- **`LockedRun` witness** (`crates/octl-core/src/lock.rs`). Compile-time proof that the caller holds the run flock before calling `append_event_with_seq` / `append_and_apply_unlocked`. Don't add `#[allow(...)]` to bypass; thread the witness through.
- **Read paths under `LOCK_SH`** (shared flock). Every multi-file read (manifest + nodes + discussions + spinoffs) wraps in `RunLock::with_shared_lock`. Don't add new readers that skip it.
- **SKILL.template.md progress-polling.** Agents call `orchestratectl run wait <id> [<id>...]` — NEVER hand-roll a `while ... run show ... case` loop. The `run wait` primitive owns the correct cadence + terminal-state semantics + zsh-safe multi-run handling. If you must inspect one-shot, use `run show`.
- **Concurrent spinoff reports.** Bundled SKILLs use `/tmp/node-report-${run_id}.json`, not the shared `/tmp/node-report.json`. Drift would re-introduce the clobber race.
- **Supervisor is the canonical tmux/worktree teardown actor.** `merge.sh` no longer touches tmux; `find_window_by_path` is session-scoped + exact-cwd-match so it never kills an unrelated pane. `--kind orchestrate` drivers ALSO spawn a supervisor now (fix `b12e13c`), so orchestrated children terminalize cleanly like spinoff children always have.
- **Test isolation via `#[file_serial(...)]`.** Any new integration test that spawns a real `orchestratectl supervise` process must serialize on `/tmp/octl-test-supervise.lock` via `serial_test::file_serial`. Otherwise it will race the other supervisor-spawning tests across binaries under `cargo test --workspace` and re-introduce the `self_terminate_*` flake.
- **`cargo install --path crates/octl-cli --force` after every CLI-surface change**, then `orchestratectl skill install --force`, then `orchestratectl doctor`. Skipping this leaves stale skills + a stale binary on `~/.cargo/bin/` while your worktrees claim the fix is deployed. Bit this session twice.
