# TODO

Currently open work — what to do, in what order, why.

For longer-running planning + design docs see `issues/<slug>/{plan,design,breakdown,validation,handoff,decisions}.md`. This file is the **session-level** plan and points at issuectl issues for the actual tracked work.

---

## Status snapshot (2026-06-29 late evening)

**Zero open issues.** `runwriter-batched-append-api` closed as `wontfix` with a v0.2 carry-over note in CHANGELOG. `flaky-self-terminate-test` fixed by serializing supervisor-spawning tests on a process-wide file lock (`serial_test::file_serial`).

Remaining work to v0.1.0: Phase E6/E7/E8 (release pipeline via `cargo-dist`) + Phase F (publish).

### Closed this session

- **Phase A** — `skill-bundling-campaign` epic closed.
- **Phase B (9/9)** — all data-integrity + `/orchestrate` polish bugs:
  `apply-event-atomicity-watermark`, `torn-write-truncate-tail`, `recover-last-seq-empty-lines`, `manifest-counter-desync`, `headless-parent-session-rejected`, `orchestrated-source-branch-ignored`, `failed-spawn-leaves-phantom-child`, `supervisor-worktree-remove-no-force`, `worktree-merge-orphans-tmux-window`.
- **Phase C1 (5/5)** — safety improvements:
  `read-side-shared-lock`, `reducer-path-traversal-defense`, `locked-run-witness-type`, `spinoff-issuectl-subprocess-bounds`, `spinoff-issuectl-materialization-arch`.
- **Phase C2 (9/9)** — output / API cleanups:
  `always-emit-warnings-array`, `envelope-schema-constant-relocation` (obsolete), `hoist-text-warning-formatting`, `passably-shaggy-parent`, `cli-text-output-escape`, `core-idempotency-api`, `supervisor-state-not-event-sourced`, `cli-json-dto-layer`, `projected-paths-into-reducer`.
- **Phase C3 (3/3)** — `macos-ci-matrix`, `idempotency-hash-golden-test`, `spinoff-e2e-harness`.
- **Phase D1** — `help-json-depth-control` shipped. Schema bumped to v3; default depth=1 cuts top-level `--help --json` from ~4300 lines to 153. `--depth N` and `--depth tree` drill-down.
- **Phase E (most)** — `README.md`, `CHANGELOG.md`, `.github/ISSUE_TEMPLATE/`, `CONTRIBUTING.md`, `SECURITY.md`, `Cargo.toml` workspace metadata, macOS CI matrix. CI doc job now green (`doc-links-octl-core-broken` closed).
- **Session-found bugs (high priority)** —
  `find-window-by-path-cross-session-kill` (would have killed user's master pane on merge — session-scoped + exact-cwd match now),
  `skill-progress-polling-wrong-field` (SKILLs steered agents at the wrong field for completion polling — branch on `manifest.status` now),
  `merge-sh-tmux-pane-recovery` (deferred to supervisor),
  `concurrent-spinoff-report-path-race` (SKILL templates now use `/tmp/node-report-${run_id}.json`),
  `headless-cancel-leaves-tmux-window`.

### What works for real-world use today

- `/worktree-spinoff`, `/worktree-research`, `/worktree-bugfix`, `/worktree-technical-decision`, `/worktree-make-skill` — autonomous spawn → work → merge → self-cleanup. Master-pane-kill risk fixed.
- `/worktree-code` + `/worktree-merge` — interactive review, then explicit merge cleans up. Supervisor is the canonical teardown actor.
- `/fan-out` — N identical units with manifest + resume + auto-cleanup.
- `/orchestrate` — DAG runtime; all 4 smoke-found polish bugs landed.

`orchestratectl doctor` is currently `126 ok / 0 fail / 0 warn`.

---

## What's left

### Phase E — Remaining pre-publication polish

These are not blocked by anything; they can land in any order before the `v0.1.0` tag.

| # | Task | Status | Notes |
|---|---|---|---|
| E5 | GitHub Actions CI | ✅ partial | `cargo test --workspace` matrix (ubuntu+macos), `clippy`, `fmt`, `cargo-deny`, `cargo doc` all gated. Re-verify after publish-config changes. |
| E6 | Release pipeline | ⬜ todo | Pick `cargo-dist` (recommended — bundles E7+E8 automation) or `release-plz`. Generates `.github/workflows/release.yml`. Needs `publish = true` on both crates (currently `false`). |
| E7 | Homebrew tap | ⬜ todo | `cargo-dist` writes the formula to a configured tap repo (`jarimustonen/homebrew-tap`?) on every release. Otherwise hand-craft per the `worktree-spinoff` SKILL.md install snippet. |
| E8 | Shell installer | ⬜ todo | `cargo-dist` also generates `orchestratectl-installer.sh`. The SKILL placeholder is currently `curl -LsSf https://...`-shaped; point it at the GitHub release URL once cargo-dist publishes one. |
| E10 | Doc build / docs.rs metadata | ✅ effectively | `cargo doc --workspace --no-deps` runs clean under `RUSTDOCFLAGS=-D warnings` (CI gate). Optional: add `[package.metadata.docs.rs]` for fine-grained control. |

**Recommended order for E6–E8:** install `cargo-dist`, run `cargo dist init` interactively, commit the generated workflow + `dist-workspace.toml`, set both crates to `publish = true`, push a `v0.0.2-alpha` test tag to verify the release pipeline produces binaries + a tap formula. Then proceed to Phase F with confidence.

---

## Phase F — Publish (the actual release)

**Don't reverse the order.** Each step depends on the previous.

1. `issuectl ls --status open` must return empty (currently 1 — see Phase D2).
2. `cargo test --workspace`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps` — all clean.
3. Bump workspace version `0.0.1 → 0.1.0` in `Cargo.toml`.
4. Update `CHANGELOG.md` — replace the `[Unreleased]` heading with `[0.1.0] — 2026-MM-DD` and add a fresh `[Unreleased]` above it.
5. Flip `publish = false` → `publish = true` on `crates/octl-core/Cargo.toml` and `crates/octl-cli/Cargo.toml`.
6. Commit + tag: `git commit -m "release: v0.1.0"` then `git tag -s v0.1.0 -m "v0.1.0"` (signed tag — Jari's GPG).
7. `git push && git push --tags`. GitHub Actions cuts the release.
8. Publish to crates.io in dependency order: `cargo publish -p octl-core` first, wait ~30s for index update, then `cargo publish -p octl-cli`.
9. If using `cargo-dist`, the tap formula auto-updates. If hand-crafted: bump `jarimustonen/homebrew-tap` to point at the new release artifacts.
10. **Smoke** on a clean shell:
    - `cargo install orchestratectl` — installs from crates.io.
    - `brew install jarimustonen/orchestratectl/orchestratectl` — installs from tap.
    - `curl -LsSf <installer-url> | sh` — shell installer.
    - `orchestratectl version` should report `0.1.0`, `orchestratectl skill install --force` deploys, `orchestratectl doctor` is clean.
11. Announce / hand to early users. Archive this TODO.md (move to `issues/v0.1.0-release-campaign/handoff.md` and seed a new TODO for v0.2.0).

---

## How to start where the previous agent left off

You are very likely starting from a **clean main** (~50 commits ahead of origin, all merged through `orchestratectl run merge` from this session's spinoffs).

1. **Sanity-check first:**
   ```bash
   git log --oneline -5            # confirm HEAD matches what's described here
   issuectl ls --status open       # should show 1 issue (D2 runwriter-batched-append-api), or 0 if Jari closed it
   orchestratectl doctor           # should report ok=126 fail=0 warn=0
   cargo test --workspace          # should be green (one occasionally-flaky test: self_terminate_when_whole_run_dir_removed — passes in isolation)
   ```

2. **Decision item:** confirm Phase D2 closure with Jari if it's still open. The `issuectl close ... --status deferred` call is the only blocker between current state and zero-open-issues.

3. **Mainline work:** Phase E6/E7/E8 (release pipeline). Start with `cargo dist init`; that one tool covers all three. It WILL want both crates to flip to `publish = true` — defer that flip until you've verified everything else, since it's the smallest reversible change.

4. **When you spawn worktrees:** all 13 bundled SKILLs are deployed (`~/.claude/skills/`). The corrected progress-polling guidance (`branch on data.manifest.status`, not `lifecycle`) is in place — use it directly. Reports go to `/tmp/node-report-${run_id}.json` to avoid the concurrent-write race.

5. **If a fix surfaces a new bug:** file it via `issuectl new --title "..." --type bug --priority high --slug <descriptive-2-3-word-kebab>`. Don't add to this TODO; track it through issuectl.

6. **After each merge:**
   ```bash
   git log --oneline -5             # confirm landed
   issuectl --json show <slug>      # status: fixed/closed
   pgrep -lf 'orchestratectl.*supervise'   # should show no supervisors from this session
   ```

---

## Rough remaining estimate

- D2 closure: 2 min (one issuectl call + CHANGELOG line).
- E6 + E7 + E8 via `cargo-dist`: 1–2 h (init, verify on a throwaway tag, iterate).
- Phase F end-to-end: 1–2 h (assuming clean smoke).

**Total to v0.1.0 published: ~3–4 h of focused work.** Nothing path-blocked, nothing waiting on review.

---

## Notable invariants the codebase now relies on

These were established this session and are easy to violate accidentally — knowing them prevents regressions.

- **`applied_seq` watermark** (`crates/octl-core/src/events.rs`). The reducer advances `manifest.applied_seq` only after every projection an event touches is fsynced; on the next lock acquisition, events with `seq > applied_seq` are replayed. Any new event-appending path MUST go through the `LockedRun` witness and `append_and_apply_*` API — never call `write_*` projection helpers directly.
- **`LockedRun` witness** (`crates/octl-core/src/lock.rs`). Compile-time proof that the caller holds the run flock before calling `append_event_with_seq` / `append_and_apply_unlocked`. Don't add `#[allow(...)]` to bypass; thread the witness through.
- **Read paths under `LOCK_SH`** (shared flock). Every multi-file read (manifest + nodes + discussions + spinoffs) wraps in `RunLock::with_shared_lock`. Don't add new readers that skip it.
- **SKILL.template.md progress-polling.** Agents branch on `data.manifest.status` (terminal: `done | failed | cancelled`), NEVER `lifecycle` (a category — `autonomous | interactive`). The `lifecycle: pending|completed|...` strings are wrong and will never match.
- **Concurrent spinoff reports.** Bundled SKILLs use `/tmp/node-report-${run_id}.json`, not the shared `/tmp/node-report.json`. Drift would re-introduce the clobber race.
- **Supervisor is the canonical tmux/worktree teardown actor.** `merge.sh` no longer touches tmux; `find_window_by_path` is session-scoped + exact-cwd-match so it never kills an unrelated pane.
