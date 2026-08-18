# TODO

Session-level plan + handoff. Longer-running planning/design docs live under
`issues/<slug>/{design,plan,breakdown,…}.md`; this file points at issuectl issues
for the actual tracked work. Standing rules and canonical learnings live in the
root `AGENTS.md` (operating policy + state-integrity invariants) — this file
holds only the **active handoff** and a **compact stint archive**.

---

## 🔄 Continue here (ALOITA TÄSTÄ), 2026-08-18 (**v0.4.0 + v0.4.1 SHIPPED — stint 6 was the round that fixed the gate that kept letting CI go red**)

**✅ (2026-08-18, stint 6 — this handoff).** Biggest round so far: **8 units landed, every one first-spawn, zero
worker deaths**, and **two releases** cut through the gated pipeline. crates.io has `octl-core 0.4.1` +
`orchestratectl 0.4.1`; both tags' publish-crates and release CI green; main CI green on each tagged commit. Local
binary **0.4.1**, commit-verified equal to `HEAD`, `orchestratectl doctor` **1047 ok / 0 warn / 0 fail** (all three
skill mirrors). `main` clean and pushed.

**What landed:**
- **`stint-skills-drop-intake-specifics`** — downstream bug-intake vocabulary (specific labels, tool, slug schemas)
  removed from the bundled stint skills; the generic autonomy tightening kept.
- **`stint-skills-issuectl-dag`** — the big one. `/stint-start` + `/stint-handoff` now read lane order, dependencies,
  collision tokens and spawnability from `issuectl dag --json` (`--reservations` supported), and the retired
  `AGENTS-EXECUTION-DAG.md` companion is deleted from all three mirrors. Its stale markdown-DAG guidance had been
  contradicting actual practice on *every* stint-start.
- **`uncommonly-fuzzy-swing`** — an autonomous worker that hits a genuine decision fork now writes a durable
  `node.awaiting_input` event (report-shaped `topic`/`options`/`recommended_default`), visible instantly on
  `run show`/`run list`; after a restart-safe 180 s grace (`OCTL_AWAITING_INPUT_GRACE_SECS`) `run wait` settles and
  `--notify` fires with `OCTL_STATUS=awaiting-input`. `node.input_resolved` is generation-fenced by `event_seq`. The
  worker must not block on stdin: it proceeds on its stated default or files a blocked report (work preserved).
- **`run-show-null-worktree-path`** — pending materialized runs expose `worktree_path` + `source_branch`.
- **`align-green-gate`** (see the incident section) and **`skills-stale-tbd-channels`** (the obsolete
  "publishing channels are TBD" hedge removed from five templates; the live channels verified).
- Two test-only CI-red fixes.

**⚠ THE ROUND'S REAL FINDING — the documented local gate was systematically weaker than CI, and that is now fixed.**
Three consecutive stints had a CI-red that a green local gate missed, each in a *different* gap: stint 5 release-mode
injection hooks; stint 6 `clippy::pedantic` `format_push_string` (CI runs `-D warnings`); stint 6 a test asserting
`doctor`'s **global exit status**, which passes on a dev box with tmux + a harness installed and fails on a bare
runner. That is a gate defect, not bad luck. **The green gate in `AGENTS.md` is now CI's exact commands**
(`cargo fmt --all --check`; `cargo clippy --locked --workspace --all-targets -- -D warnings`;
`cargo nextest run --locked --release --workspace`; `cargo test --locked --release --workspace --doc`;
`RUSTDOCFLAGS="-D warnings" cargo doc --locked --workspace --no-deps`), the integrated gate matches, and both carry an
explicit warning that **a developer machine is not a bare CI runner** — tool-sensitive tests must be exercised with a
stripped `PATH`. The round's own integrated gate ran on those commands: 1000/1000 green.

**⚠ AND A WORKER DESTROYED THE USER'S INSTALLED BINARY — TWICE, hours apart.** A worker ran
`cargo install --path crates/octl-cli` from inside its worktree, overwriting the global
`~/.cargo/bin/orchestratectl` and recording the worktree as install source. The binary later vanished entirely, `PATH`
silently fell through to a **stale Homebrew tap build from an older release**, and that stale binary reinstalled
**pre-migration** bundled skills over the corrected ones — partially undoing the round's work. `doctor` reported
`0 warn` throughout, because a stale binary validates its own stale skills. It recurred *after* a brief explicitly
forbade it, so prose in one brief is not enough. Now standing policy in `AGENTS.md` + three skill templates: workers
use `cargo build --release` and invoke `./target/release/orchestratectl` by explicit path; global
`cargo install`/`cargo uninstall` is an orchestrator-only action. **The deploy step now asserts the installed binary's
commit equals `git rev-parse HEAD`** — a plausible version string proved nothing, and that check is the only reason
this was caught.

**◆ JARI'S DECISIONS THIS ROUND.**
1. Green gate → CI's commands. Done (above).
2. Workers may and should exercise their builds, but **inside their own worktree**, never into the global toolchain.
   Done (above) — this was Jari's own proposed shape.
3. **x86_64 macOS is deliberately NOT supported.** An earlier claim in this session that the release had a missing-Mac
   gap was **wrong**: `dist-workspace.toml` + `OSS-RELEASE.md` declare exactly `aarch64-apple-darwin`,
   `aarch64-unknown-linux-gnu`, `x86_64-unknown-linux-gnu`, and the releases match. No action.
4. **`add-configurable-agent` stays deferred**, knowingly. Jari's steer is now recorded **on the issue itself** (not
   duplicated here): the local/`secure` profile is *genuinely weak*, so it should be given tasks so small and
   unambiguous that it never occurs to it to do anything else — and worth considering that it simply **cannot / does
   not know how to spawn further worktrees**, since removing the capability beats trusting a weak model to decline it.
   Residency remains a machine-checkable profile attribute, never a fallback that can escalate to remote. Deferring is
   an accepted, stated risk. It is cross-cutting (config + `harness::select` + run-create) and wants a round of its own;
   design.md v2 is ready to implement.

**⏭ NEXT.** Consult `issuectl dag` for what to pick — this file no longer states it. Context a fresh agent will want:
`run-prefix-collision` is a **real, observed** bug, not speculation (two concurrently created runs shared their first
10 ID characters; a worker nearly reported against the wrong run, and three colliding prefixes exist on disk right
now); it pairs naturally with `run-branch-name-ulid-entropy`. `audit-no-user-specifics` has now waited five stints.
`intake-bug-orchestratectl-169460ea27e7` (stale pendings cluttering `run list`) is confirmed still live — preflight
saw 6 stale pending runs, **most of them from other repos**, which is exactly the reported symptom.

**Release-mechanics notes (mechanics only — rules live in `AGENTS.md`):** the gated tag push worked as designed both
times. Reading crates.io's API **requires a `User-Agent` header** — without one it returns null and looks like a failed
publish (this cost a false alarm this round). After a version bump run `skill install --force` for `--agent codex` too,
or doctor shows codex sync warnings.

---

## Scheduling

Canonical scheduling lives in `issuectl` frontmatter (`lane:`, `lane_seq:`, `blocked_by:`, `collision:`). Do not maintain a markdown DAG or adjacent backlog in this file.

Use these views instead:

```bash
issuectl dag
issuectl dag --json
issuectl ls --status open
issuectl ls --status in-progress
```

`TODO.md` is only the session handoff and project notes; issue bodies and `issuectl dag` are the source of truth.

---

## Invariants + operating policy

The **7 state-integrity invariants** and the stint operating policy (release
mechanics, deploy, green gate + integrated gate, hot files, standing learnings)
live in the root `AGENTS.md`. Read them before touching the reducer, the lock
layer, or `supervise/`, and before any release action.

---

## Stint archive (compact — durable facts only)

Full narratives live in git history of this file; canonical rules extracted from
these stints are in `AGENTS.md`.

- **Stint 5 (2026-08-17, v0.3.0).** Full round + release, 5 headless spinoffs (3 planned + 2 CI-red fixes), all
  first-spawn. Landed: `release-gate-on-ci` (publish-crates.yml now repeats the full main-CI gate + a tag/manifest
  version-match check before any publish step — proven live by that release), `create-idempotency-lease-recovery`
  (durable creator lease on pre-publication reservations; follow-up `recover-unkeyed-child-publication` closed
  `wontfix` on Jari's call), `config-show-layered-view` (config schema v2, layered tolerant inspection). Two CI-reds:
  `ci-red-release-mode-injection` (debug-only `OCTL_TEST_*` hooks vs CI's `--release`) and
  `etxtbsy-cross-module-stub-race` (the third ETXTBSY; killed structurally by moving CI to cargo-nextest
  process-per-test rather than re-mutexing). Both are inputs to the stint-6 green-gate fix.
- **Review session (2026-08-17, after stint 4).** Fable-driven repo review + doc cleanup, parallel to the
  `add-configurable-agent` design session (design.md v2). `AGENTS.md` rewritten for consistency; `README.md` rewritten
  against 0.2.2 reality; stints 1–3 compressed into this archive. Code: `cut-plan-module` (dead 2013-line
  `octl-core/src/plan.rs` removed — the breaking entry that made the next release 0.3.0) + `harness-pi-default`
  (built-in default flipped to `pi` per ADR 0001 D4). Epics `code-pipeline` + `lifecycle-architecture-review` closed.
- **Stint 4 (2026-08-17, v0.2.2).** Full round + release + a caught mistake: 4 parallel spinoffs (all first-spawn),
  v0.2.2 cut, then CI caught one fix incomplete and a 5th spinoff finished it. Landed: `pi-spinoff-batch` (staged
  atomic run-create publication — the load-bearing fix), `cli-canon-help-json` (§14, clap-derived help envelope),
  `tmux-stub-etxtbsy-flake` (took two spinoffs; its tmux-family mutex later proved too narrow — the class was killed
  structurally in stint 5 by nextest process-per-test in CI), `spinoff-skill-stale-preview-banner` +
  `skill-install-force-symlink`. Lane fix: phantom `supervise` lane merged into `lifecycle` (verified from git;
  `lifecycle` deliberately NOT split despite depth). Origin of the "DO NOT `cargo publish` locally — the tag push IS
  the publish" rule (promoted to `AGENTS.md` after v0.2.2 was luckily-unharmed published before CI reported). Triage
  sweep: 2 closed cannot-reproduce (incl. `run-wait-false-stillborn-slow-start` — did occur when filed, did not recur
  after staged-create; re-file readily), stale-pendings intake laned.
- **Stint 3 (2026-08-17, v0.2.1).** Full round + release, 3 parallel spinoffs, all landed first-spawn.
  Landed: `spinoff-report-fields-null` (report read-back surface + docs in five skills — the four "null report" bugs
  were all read-surface errors), `run-create-long-title-stillborn` (branch names bounded to workmux's 50-byte
  window-name input), `cli-canon-version-schemas` (§10: `supported_schemas` in `version`). Closed without code:
  `cli-canon-config` (already shipped in 0.2.0). Origin of the `--locked`-is-mandatory + never-`| tail` deploy rules
  (now in `AGENTS.md`). ADR 0011 (homebase) boundary recorded
  in `AGENTS.md`: no pi-processes dependency.
- **Stint 2 (2026-08-16, triage-only).** 39 unscheduled issues → 24 closed, 15 laned; whole queue verified against
  current code. Two-thirds of the queue was not real work: 4 "bugs" were the same report-read-surface mistake, 11 were
  LLM review-residue with template bodies pointing at gitignored `history/` files, 5 duplicates. Origin of the
  filing-bar and verify-against-running-binary rules (now in `AGENTS.md`). Queue hygiene: `issuectl doctor --fix`,
  recovered real close-dates from git. `audit-no-user-specifics` arrived and was laned (skills lane): grep of the
  shipped artifact hits 19 files, five of them bundled SKILL templates; zero hits under `crates/*/src/`.
  *Left for Jari, outside this repo:* `~/.claude/skills/triage-bugs/` had dangling symlinks after the homebase rename
  to `triage-unlaned-issues`; fix on the homebase side if not already done.
- **Stint 1 (2026-08-16, v0.2.0).** The 0.2 simplification shipped end-to-end: thin supervisor (A1 exit-status shim,
  A2 OID-based merge-transaction recovery, A6 typed outcome table, A5 `attention_required`, A3 fenced `run salvage`,
  explicit `--interactive`), the teardown work-preservation guards, `config path`/`config show`, and the kind/heuristic
  cuts. Everything is documented as invariants 5–7 + the ADR (`docs/decisions/0001-thin-supervisor-vs-harden.md`).
- **Pre-0.2 (2026-07 → 2026-08-15).** The pivot: bug-cluster analysis showed ~57% of open issues concentrated in the
  supervisor/lifecycle subsystem with one root cause (state INFERRED from `pid × pane × branch × report`), so patching
  was stopped and the `lifecycle-architecture-review` epic ran instead — three research worktrees
  (`analysis.md`/`feature-audit.md` with 717-run usage evidence/`alternatives.md`), DECISION-1 (cut/keep/reframe, with
  Jari), a facilitated design session → `design.md`, DECISION-2 (thin model) → the ADR. v0.1.5–v0.1.8 shipped along the
  way. Durable residue: (a) "a subsystem whose bugs are combinatorial needs an architecture review, not more patches";
  (b) supervisor deaths under spawn saturation — the *surfacing* half shipped in 0.1.5 and the staged-create fix (0.2.2)
  removed the main trigger; the remaining resilience work is tracked as `create-idempotency-lease-recovery` + the
  stale-pendings issue; (c) the disjoint-lanes/integrated-gate and worker-deaths-transient learnings, both promoted to
  `AGENTS.md`.

---

## Piialiisan bugiraportit

- [ ] 🐛 Piialiisan bugiraportti: Worker that exits without run merge is indistinguishable from a healthy… — jari via Telegram ([`intake-bug-orchestratectl-9efd3de5753c`](issues/intake-bug-orchestratectl-9efd3de5753c/item.md))
