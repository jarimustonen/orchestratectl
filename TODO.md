# TODO

Currently open work — what to do, in what order, why.

For longer-running planning + design docs see `issues/<slug>/{plan,design,breakdown,validation,handoff,decisions}.md`. This file is the **session-level** plan and points at issuectl issues for the actual tracked work.

---

## Status snapshot (2026-07-03 → continued)

**`supervisor-dead-merge-no-teardown` is FIXED and merged** (task 0 done — commits `979b794` fix + `62948c8` llm-review hardening, closed at `938f10f`/`60147b6`). The bugfix auto-reattaches a fresh supervisor on merge when the recorded one is dead (never silent — emits a warning + machine-readable `supervisor:{state}`), adds `supervisor:{pid,alive}` to `run show`/`run list`, and carries a serialized e2e test. A full `/llm-review` caught an extra orphan case (pid-file-absent) that was also fixed.

**Three data-integrity supervisor/merge bugs fixed & deployed this session:**
- `supervisor-dead-merge-no-teardown` (task 0) — merge auto-reattaches a dead supervisor; `supervisor:{pid,alive}` on show/list. Commits `979b794`+`62948c8`.
- `run-create-agent-startup-timeout` (task 0.5) — `run create --agent-startup-timeout` [1,600], default 90s, forwarded to create.sh. Kills spawn-under-load failures. Commit `e6df5d8`. **Workaround retired.**
- `blocked-report-deletes-branch` (task X, HIGH silent-data-loss) — blocked terminal report (`success:false`) no longer force-deletes the branch/worktree. Two-layer gate: `node_report_is_blocked` + source-relative `git rev-list --count source..branch` net; `-D` force reserved for confirmed `run merge`. 7 unit + 2 e2e tests. Commits `fe44a56`+`498cf5d`. Invariant #5 updated.

**Three open issues now** (all normal-priority, all non-blocking for v0.1.0):
- `cancel-dead-supervisor-recovery` — apply the dead-supervisor reattach to `run cancel` (the blocked-branch source-relative net already prevents cancel-with-committed-work data loss).
- `legacy-pid-identity-check` — recycled legacy bare-integer supervisor.pid reads as alive (rare residual).
- `teardown-gate-trust-and-lifecycle` — from blocked-branch `/llm-review`: reserve `via`/`cancelled` markers in the node-report validator against spoofing; preserved-worktree lifecycle (status surface + GC/reclaim verb).

The v0.1.0 publish is now **one hard thing away**: install the self-hosted macOS runner on `hauis` so the alpha pipeline can complete (Jari's Free-tier GitHub account has no usable macOS-runner budget — public repo did NOT unblock this).

### Spawn-under-load: now fixed (historical note)

`run-create-agent-startup-timeout` is fixed & deployed (default 90s window), so spawns no longer die at the old 30s ceiling under load. Two operational notes still apply:
- `run create` blocks until the agent launches (can exceed 2 min on a very loaded host) — spawn it with `run_in_background` so a shell timeout doesn't SIGTERM it mid-spawn and leave a 0-node stub. If it does, `orchestratectl run cancel <id>` the stub.
- Prefer `--headless` on a busy host (also the macOS-PTY guidance for batches).
- Pre-`e6df5d8` binaries only: the `OCTL_CREATE_SH` wrapper (`exec …/create.sh --agent-startup-timeout=180 "$@"`) was the reversible workaround. No longer needed.

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

1. `issuectl ls --status open` empty (currently 0 — decide inline what to do with `BUG-REPORT-supervisor-dead-merge-no-teardown.md`).
2. `cargo test --workspace` + clippy + fmt + doc-warnings clean.
3. Bump workspace version `0.0.2-alpha → 0.1.0` in `Cargo.toml`; update the `octl-core` path-dep version to `=0.1.0` in `crates/octl-cli/Cargo.toml`.
4. `CHANGELOG.md`: replace `[Unreleased]` heading with `[0.1.0] — 2026-MM-DD`, add fresh `[Unreleased]` above.
5. Flip `publish = false → true` on **both** `crates/octl-core/Cargo.toml` and `crates/octl-cli/Cargo.toml`.
6. Commit `release: v0.1.0`, tag `v0.1.0` **signed with Jari's GPG on `gertrud`** (hauis has no signing key — Jari has to run the tag command from gertrud) then push tag.
7. GitHub Actions release workflow runs on the tag. Wait for Homebrew formula update.
8. `cargo publish -p octl-core`; wait ~30s; `cargo publish -p orchestratectl`.
9. Smoke on a clean shell:
   - `cargo install orchestratectl` — from crates.io.
   - `brew install jarimustonen/orchestratectl/orchestratectl` — from tap.
   - `curl -LsSf https://github.com/jarimustonen/orchestratectl/releases/latest/download/orchestratectl-installer.sh | sh` — shell installer.
   - `orchestratectl version` → `0.1.0`; `orchestratectl skill install --force`; `orchestratectl doctor` clean.
10. Announce / archive this TODO into `issues/v0.1.0-release-campaign/handoff.md` and seed a v0.2.0 TODO.

---

## How to start where the previous agent left off (2026-06-30)

1. **Sanity-check first:**
   ```bash
   git log --oneline -5            # confirm HEAD is at least the TODO commit
   issuectl ls --status open       # expect 1: supervisor-dead-merge-no-teardown
   orchestratectl version          # expect 0.0.2-alpha
   cargo test --workspace          # expect green
   ```

2. **First actual task: task 0 above** — `/worktree-bugfix supervisor-dead-merge-no-teardown`. Do this before touching the runner setup. The `run merge` silent-success fix will likely also help you smoke-test the runner setup safely (a stale supervisor from a leftover worktree won't mislead you into thinking `run merge` cleaned up).

3. **Then task 1** (self-hosted runner on hauis). Decision already made — Jari picked hauis over gertrud ("hauis on aina livenä"). You just need the registration token from Jari and to run the config steps below.

4. **`run wait` is available** — use it for every spinoff completion signal:
   ```bash
   orchestratectl run wait <run-id> --timeout 90m --output json
   ```
   Do NOT use `gh run watch` for GitHub Actions runs — it periodically loses auth and rate-limits. Use a plain poll loop:
   ```bash
   until s=$(gh run view <id> --json status -q .status) && [ "$s" = "completed" ]; do sleep 120; done
   ```

5. **If a fix surfaces a new bug:** file it via `issuectl new --title "..." --type bug --priority high --slug <descriptive-2-3-word-kebab>`, then `/worktree-bugfix` or `/worktree-spinoff` it. Don't add to this TODO; track it through issuectl.

6. **When you rebuild the binary and want to smoke-test locally:**
   ```bash
   cargo install --path crates/octl-cli --force
   orchestratectl skill install --force
   orchestratectl doctor
   ```
   The binary in `~/.cargo/bin/` is what every SKILL, spinoff, and driver invokes — a stale binary is a very silent failure mode. This session hit it once (the `9bdadff` tmux-session fix wasn't active until we re-installed).

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
