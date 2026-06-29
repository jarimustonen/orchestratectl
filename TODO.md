# TODO

Currently open work — what to do, in what order, why.

For longer-running planning + design docs see `issues/<slug>/{plan,design,breakdown,validation,handoff,decisions}.md`. This file is the **session-level** plan and points at issuectl issues for the actual tracked work.

---

## Status snapshot (2026-06-29 afternoon)

- ✅ **MVP epic** [`orchestratectl-mvp`](issues/orchestratectl-mvp/item.md) — done.
- ✅ **Follow-up campaign** (21 review-spinoff issues + 9 packs) — all merged.
- ✅ **Skill-bundling campaign** — closed as `done` this session (commit `f96b36b`). 13 skills bundled, `doctor` 63/0, end-to-end loops proven.
- ✅ **Phase E (partial)** — README rewritten, CHANGELOG seeded for v0.1.0, ISSUE_TEMPLATEs + CONTRIBUTING + SECURITY landed (commits `fc9e81e`, `9a85dc1`). E4 (Cargo.toml metadata) deferred until B fixes merge to avoid conflict.
- 🟡 **Phase B (in flight)** — first batch of 3 spinoffs running in parallel (without `--headless` because B2.1 blocks it):
  - `01kw8ttnx3` — `fix-headless-parent` (B2.1, unlocks `--headless` for rest)
  - `01kw8ttvth` — `fix-atomicity-watermark` (B1.1)
  - `01kw8tv19t` — `fix-wmt-orphan-window` (B3)
  - Once **B2.1 merges**, the remaining 5 spinoffs (B1.2, B1.3, B1.4, B2.2, B2.3, B2.4) can spawn `--headless` in parallel. Prompt files already prepared in `/tmp/spinoff-prompts/`.
- 🟡 **Pre-publication campaign (this TODO's active scope)** — close every open issue, then ship to GitHub. Target: **zero open issues** before `v0.1.0` tag.

### What works for real-world use today

- `/worktree-spinoff`, `/worktree-research`, `/worktree-bugfix`, `/worktree-technical-decision`, `/worktree-make-skill` — autonomous spawn → work → merge → self-cleanup.
- `/worktree-code` + `/worktree-merge` — interactive review, then explicit merge cleans up.
- `/fan-out` — N identical units with manifest + resume + auto-cleanup per child.
- `/orchestrate` — toy-tested DAG runtime; usable for small campaigns; **do NOT rely on for large real-world campaigns until the 4 polish bugs below are fixed**.

---

## Active goal: zero open issues, GitHub-publishable

Currently **29 open issues** (snapshot 2026-06-29). The plan below brings that to zero, then publishes. Sequenced so correctness-affecting bugs land before polish, and held items get a decision before the publication tag.

---

## Phase A — Close the skill-bundling-campaign epic

The epic itself ([`skill-bundling-campaign`](issues/skill-bundling-campaign/item.md), `status: open`) still has the original "open" marker even though every child phase has merged and the bonus `bundle-worktree-merge` capstone has shipped. Update its body with the final state (10 + 1 bundled SKILLs, deployment notes, references to the 4 polish bugs as the natural follow-on) and close it.

- `issuectl close skill-bundling-campaign --status done`

Single commit, ~10 min.

---

## Phase B — Fix bugs (correctness gate for publication)

Order: data-integrity → UX-affecting → polish. Some can run in parallel as `/worktree-spinoff`s (no overlap in files); some must serialize because they touch the same paths.

### B1. Data-integrity bugs (must fix before publish)

These can corrupt run state on disk. Block publication.

| # | Issue | Why blocking |
|---|---|---|
| 1 | [`apply-event-atomicity-watermark`](issues/apply-event-atomicity-watermark/item.md) | Append-then-apply is not atomic across reducer failure → state can desync from event log |
| 2 | [`torn-write-truncate-tail`](issues/torn-write-truncate-tail/item.md) | `events.jsonl` torn-write recovery doesn't truncate cleanly → corrupted line on next read |
| 3 | [`recover-last-seq-empty-lines`](issues/recover-last-seq-empty-lines/item.md) | `recover_last_seq` doesn't loop over multiple trailing empty lines |
| 4 | [`manifest-counter-desync`](issues/manifest-counter-desync/item.md) | Reducer manifest counters can permanently desync after partial failure |

### B2. /orchestrate polish bugs (surfaced by yesterday's smoke)

`/orchestrate` works for toy cases but these four need fixing before it's safe at scale. All are `high` (or effectively so).

| # | Issue | Pri |
|---|---|---|
| 5 | [`headless-parent-session-rejected`](issues/headless-parent-session-rejected/item.md) | high |
| 6 | [`orchestrated-source-branch-ignored`](issues/orchestrated-source-branch-ignored/item.md) | high |
| 7 | [`failed-spawn-leaves-phantom-child`](issues/failed-spawn-leaves-phantom-child/item.md) | (effectively high) |
| 8 | [`supervisor-worktree-remove-no-force`](issues/supervisor-worktree-remove-no-force/item.md) | (effectively high) |

### B3. Other open bugs

| # | Issue | Notes |
|---|---|---|
| 9 | [`worktree-merge-orphans-tmux-window`](issues/worktree-merge-orphans-tmux-window/item.md) | `worktree-merge`: tmux window orphaned when a rebase fails partway |

**Recommended approach.** Spawn B1 sequentially (touch reducer / event-log internals, conflict risk). B2 spawn in parallel (mostly disjoint files). B3 can ride with B2 (worktree-merge cleanup is independent).

---

## Phase C — Land improvements

Order: correctness/safety improvements first, then ergonomics, then nice-to-haves.

### C1. Safety / correctness improvements

| # | Issue |
|---|---|
| 1 | [`read-side-shared-lock`](issues/read-side-shared-lock/item.md) — read paths need shared flock |
| 2 | [`reducer-path-traversal-defense`](issues/reducer-path-traversal-defense/item.md) — path-traversal defense for IDs |
| 3 | [`locked-run-witness-type`](issues/locked-run-witness-type/item.md) — type-system enforcement for lock-held writes |
| 4 | [`spinoff-issuectl-subprocess-bounds`](issues/spinoff-issuectl-subprocess-bounds/item.md) — bound issuectl subprocess |
| 5 | [`spinoff-issuectl-materialization-arch`](issues/spinoff-issuectl-materialization-arch/item.md) — redesign spin-off issuectl materialization |

### C2. Output / API cleanups

| # | Issue |
|---|---|
| 6 | [`always-emit-warnings-array`](issues/always-emit-warnings-array/item.md) — feature |
| 7 | [`cli-json-dto-layer`](issues/cli-json-dto-layer/item.md) — DTO layer for `--json` payloads |
| 8 | [`cli-text-output-escape`](issues/cli-text-output-escape/item.md) — escape control chars in text output |
| 9 | [`core-idempotency-api`](issues/core-idempotency-api/item.md) — centralize `--idempotency-key` |
| 10 | [`envelope-schema-constant-relocation`](issues/envelope-schema-constant-relocation/item.md) — relocate envelope SCHEMA_VERSION |
| 11 | [`hoist-text-warning-formatting`](issues/hoist-text-warning-formatting/item.md) — central text-mode warning emission |
| 12 | [`passably-shaggy-parent`](issues/passably-shaggy-parent/item.md) — surface dropped-log count on error envelopes |
| 13 | [`projected-paths-into-reducer`](issues/projected-paths-into-reducer/item.md) — move projection enumeration into reducer |
| 14 | [`supervisor-state-not-event-sourced`](issues/supervisor-state-not-event-sourced/item.md) — make supervisor state event-sourced |

### C3. Tests / CI

| # | Issue |
|---|---|
| 15 | [`spinoff-e2e-harness`](issues/spinoff-e2e-harness/item.md) — end-to-end test harness (already started, bounced by PTY; retry with `--headless` once B2 lands) |
| 16 | [`idempotency-hash-golden-test`](issues/idempotency-hash-golden-test/item.md) — golden test for idempotency-key hash |
| 17 | [`macos-ci-matrix`](issues/macos-ci-matrix/item.md) — macOS CI matrix |

**Recommended approach.** C1 sequentially (locking + safety code is conflict-prone). C2 in batches of 3–4 in parallel. C3 last (validates the rest).

---

## Phase D — Held items: Jari decisions needed

These two have been sitting in "needs product decision" for a while. Before publication, each gets a clear yes/no — implement, defer-with-issue-closed-as-deferred, or close-as-wontfix.

| # | Issue | Decision needed |
|---|---|---|
| D1 | [`help-json-depth-control`](issues/help-json-depth-control/item.md) | Schema bump for `--help --json` top-level depth — is it needed pre-publication, or close as wontfix? |
| D2 | [`runwriter-batched-append-api`](issues/runwriter-batched-append-api/item.md) | V4 latency 639ms p99 vs 10ms budget — accepted post-MVP per B1 decision; still accepted, or fix before publication? |

### Agent's recommendation (2026-06-29)

**D1 — IMPLEMENT before v0.1.0.** A 2100-line firehose on the very first `orchestratectl --help --json` an agent runs is bad first-impression UX, and the SKILL family teaches agents to use `--output json` for discovery. Proposed shape: default depth 1 (each node lists immediate subcommands as `{name, about, aliases}`), opt-in `--tree` for full recursion. Schema bumps to `2`; old shape stays available via `--depth full`. ~half-day of work. Spawn as `/worktree-spinoff` once B is clear.

**D2 — DEFER to v0.2.0, document in CHANGELOG known-gaps.** The 639ms p99 is felt only when a single supervisor appends many events in a tight loop — current workloads (one append per agent action) are below that frequency. Architectural change (long-lived RunWriter guard) is substantial and overlaps with the data-integrity work in B1.1–B1.4; doing it now risks merge conflicts and forces a second design pass once the watermark settles. Close the issue with `status: deferred-v0.2` and move on.

Bring these up early in the post-resume session — D2 just needs your nod to close, D1 needs ~3 hours of focused work.

---

## Phase E — Pre-publication polish

Mechanical but required for a presentable GitHub release.

| # | Task | Notes |
|---|---|---|
| E1 | `README.md` at repo root | Project pitch, install (homebrew / cargo / shell), `orchestratectl skill print orchestratectl-overview` as the agent's onboarding, examples. The SKILLs already encode the operating manual — README links to them. |
| E2 | `LICENSE` | Pick (MIT? Apache-2? dual?). If MIT, drop the standard file. |
| E3 | `CHANGELOG.md` | `v0.1.0` entry covering everything that landed; reference closed issues. |
| E4 | `Cargo.toml` metadata | `description`, `homepage`, `repository`, `keywords`, `categories`, `license`. Required for crates.io publication. |
| E5 | GitHub Actions CI | `cargo test --workspace`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, plus the SKILL example CI gate (already exists) and `macos-ci-matrix` (issue C3 #17) once landed. |
| E6 | Release pipeline | `cargo-release` or `release-plz` — automates version bump, changelog, tag, crates.io publish, GitHub release. |
| E7 | Homebrew tap | `jarimustonen/orchestratectl` per the SKILL.md install instructions. Currently a placeholder — make it real. |
| E8 | Shell installer | `curl -LsSf .../orchestratectl-installer.sh | sh` per SKILL.md. Currently a placeholder — wire up via `cargo-dist` or similar. |
| E9 | Repo hygiene | `.github/ISSUE_TEMPLATE/`, `CONTRIBUTING.md` (optional v0.1.0; required if accepting external PRs), `SECURITY.md` (optional). |
| E10 | Doc build | Verify `cargo doc` is clean; consider docs.rs metadata in `Cargo.toml`. |

---

## Phase F — Publish

Order is meaningful — don't reverse:

1. Confirm zero open issues (`issuectl ls --status open` returns empty).
2. Final `cargo test --workspace`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` all clean.
3. Bump version to `0.1.0` in workspace `Cargo.toml`.
4. Update `CHANGELOG.md` with the `v0.1.0` entry.
5. Tag `v0.1.0` and push.
6. GitHub Actions cuts a release, uploads binaries (via cargo-dist or equivalent).
7. Publish to crates.io (`cargo publish` from each workspace member, in dependency order — `octl-core` first, then `octl-cli`).
8. Update Homebrew tap with the v0.1.0 formula.
9. Smoke: `brew install jarimustonen/orchestratectl/orchestratectl` on a clean machine works; `cargo install orchestratectl` works; shell installer works.
10. Announce / hand over to early users.

---

## How to start a phase (for the next agent)

1. **First**: read `git log --oneline -30` and `issuectl ls --status open` to confirm current state matches this TODO.
2. Pick the lowest-numbered unfinished item in the current phase.
3. **Default workflow for code fixes**: spawn `/worktree-spinoff <issue-slug>` (autonomous). The SKILL now handles spawn → work → merge → `orchestratectl run merge` → self-cleanup end-to-end.
4. **For substantial / cross-cutting changes**: spawn `/worktree-code <issue-slug>` (interactive) so a human reviews before merge.
5. **For parallelizable batches** (e.g. several disjoint improvements): spawn 3–5 spinoffs in succession, set a `Monitor` watching `orchestratectl event tail` for `node.report|run.status|supervisor.exited` events, and continue with the next batch when they land.
6. **For multi-feature campaigns** (e.g. release pipeline = E5 + E6 + E7 + E8 together): `/orchestrate` is now battle-tested at toy scale but needs the B2 fixes before scaling — until then, use a sequence of `/worktree-spinoff`s with the orchestrator agent reading the report after each.
7. After each merge: confirm via `git log --oneline -5` that it landed, `issuectl --json show <slug>` reports `status: closed`, and `pgrep -lf "orchestratectl.*supervise"` doesn't include any of your spawns (= auto-cleanup worked).
8. If a fix surfaces a NEW bug, file it as a new issue and add it to this TODO under the appropriate phase.

---

## Estimate (rough)

- Phase A: 10 min (one commit closing the epic).
- Phase B: ~6–10 h total. B1 ~1h each (4 items, sequential). B2 ~30 min each (4 items, parallel-able). B3 ~30 min.
- Phase C: ~10–15 h total. C1 ~1h each (5 items, mostly sequential). C2 ~30 min each (9 items, parallel-able in batches). C3 ~1h each (3 items).
- Phase D: ~30 min total (Jari decision + execution per item).
- Phase E: ~5–8 h total (E1–E4 ~1h each, E5–E8 ~1h each, E9–E10 ~30 min each).
- Phase F: ~2 h end-to-end.

**Grand total: ~25–35 h across multiple sessions.** Spreadable across days; nothing is path-blocked except B → E (don't publish before bugs fixed), D → F (decisions before tag).

---

## When the campaign finishes

- `issuectl ls --status open` returns empty.
- `cargo test --workspace`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` all green.
- `v0.1.0` tag pushed, GitHub release live, crates.io package published, Homebrew tap updated.
- README, CHANGELOG, LICENSE, CI all in place.
- This TODO.md gets archived (move to `issues/<v0.1.0-release-campaign>/handoff.md` and replace with a fresh skeleton for v0.2.0 planning).
