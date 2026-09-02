# TODO

Session-level plan + handoff. Longer-running planning/design docs live under
`issues/<slug>/{design,plan,breakdown,…}.md`; this file points at issuectl issues
for the actual tracked work. Standing rules and canonical learnings live in the
root `AGENTS.md` (operating policy + state-integrity invariants) — this file
holds only the **active handoff** and a **compact stint archive**.

---

## 🔄 Continue here (ALOITA TÄSTÄ), 2026-08-22 (**stint 8: v0.5.0 shipped; worker control-plane design sequence prepared**)

**Ownership is settled.** The release-recovery workers `01m0fggg0zg9a5ezcdgpeq5r4g`, `01m0hj9bnnfamteydmt7qvyh64`, and `01m0ja657ejjjyc7j7230jf42n` are terminal `done`, report `landed: true`, and have no worktree or recoverable work. The only globally listed non-terminal rows are old `stillborn: true` records with no source repository, worktree, supervisor, or ownership; none belongs to live orchestratectl work.

**What landed and shipped.** `distinguish-untriaged-work` now keeps unaccepted intake distinct from explicit human deferral. The release wrapper recognizes the proven ossctl 0.9 held-tag checkpoint, and its pre-tag protocol was separately revalidated for exactly ossctl 0.10.0 while preserving the exact-SHA CI gate and abandoned-run rejection. The full integrated gate passed after both fixes. v0.5.0 was then published through the tag-triggered pipeline to crates.io, GitHub Release, cargo-dist binaries, and Homebrew; release run `01M0JF9T187YJVNZAT2STRCZGH` is completed with all four targets verified and no in-flight release. The two earlier failed journals remain abandoned and must never be resumed.

**Repository-local truth.** `main` was clean and synchronized before this handoff edit. Normal repository work has no stint deploy step: source HEAD is validated with repository-local builds and explicit `./target/release/orchestratectl …`, without replacing the user's installed release or bundled instructions. Installation and upgrades belong to the published distribution channels, outside repository build/test work.

**Agreed product direction — design the whole worker control plane before implementation.** First, `worker-telemetry-protocol` designs a harness-neutral told-fact lease/status contract and a separate pi.dev adapter; silence may mean stale telemetry but never inferred success, failure, or teardown, and Claude remains explicit-interactive unless it gains a real adapter. Second, `add-configurable-agent` revises its historical profile design against that protocol, including telemetry capability, autonomy eligibility, residency, fallback, and effective-policy provenance. Third, `worker-control-plane-review` presents both designs together for Jari's explicit approval. Only after that checkpoint are production implementation slices filed and scheduled. `end-end-stint` remains the wider durable start → work → handoff → user-checkpoint loop that must consume, not duplicate, these decisions.

**Worker-to-spawner reporting decision still to settle.** Existing communication is the durable terminal `node.report`: completed work carries it through `run merge --report-file`, while blocked work submits it directly and preserves recoverable work. `consult-failure-hard-fail` has been generalized to require actionable disclosure of failed or incomplete tools/sub-workflows through that channel. The current §7.3 JSON contract has `success`, `summary`, `discussion_items`, `spinoff_proposals`, and `wrap_up_recommendations`, but no official `tool_failures[]`; decide during its work whether concise use of existing fields is enough or whether a small advisory schema extension is justified.

**Awaiting human lane-or-close triage (context only, not executable agenda).** Unscheduled untriaged intake remains for the normal sweep, including run repository identity/filtering reports and the watch-only Cargo provenance observation. Two accepted-looking open items, `end-end-stint` and `run-wait-json`, are also currently outside the DAG and will re-surface in the wrap-up verifier for an explicit lane-or-close decision; do not infer scheduling from their mechanical unscheduled output.

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
mechanics, repository-local validation, green gate + integrated gate, hot files,
standing learnings) live in the root `AGENTS.md`. Read them before touching the reducer, the lock
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
  `cli-canon-config` (already shipped in 0.2.0). This stint also exposed now-retired source-tree local-deploy rules;
  normal repository work no longer installs the tool. ADR 0011 (homebase) boundary recorded
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

- [ ] 🐛 Piialiisan bugiraportti: run create omits source_repo from fresh run manifest — jari via Telegram ([`intake-bug-orchestratectl-19a653fff4c9`](issues/intake-bug-orchestratectl-19a653fff4c9/item.md))
- [ ] 🐛 Piialiisan bugiraportti: run show cannot identify a run repository once its worktree is gone — jari via Telegram ([`intake-feature-orchestratectl-f706c536df01`](issues/intake-feature-orchestratectl-f706c536df01/item.md))
- [ ] 🐛 Piialiisan bugiraportti: Expose source_repo in run show JSON — jari via Telegram ([`intake-feature-orchestratectl-635e9e31cdf2`](issues/intake-feature-orchestratectl-635e9e31cdf2/item.md))
- [ ] 🐛 Piialiisan bugiraportti: stint-handoff blocks on unrelated concurrent agents — jari via Telegram ([`intake-bug-orchestratectl-53fa835cfa74`](issues/intake-bug-orchestratectl-53fa835cfa74/item.md))
- [ ] 🐛 Piialiisan bugiraportti: Add teardown for terminal failed runs with preserved worktrees — jari via Telegram ([`intake-feature-orchestratectl-41343c4dd3e4`](issues/intake-feature-orchestratectl-41343c4dd3e4/item.md))
- [ ] 🐛 Piialiisan bugiraportti: stint-handoff blocks on unrelated global runs — jari via Telegram ([`intake-bug-orchestratectl-6edf517c691a`](issues/intake-bug-orchestratectl-6edf517c691a/item.md))
- [ ] 🐛 Piialiisan bugiraportti: stint-start should safely rebase a clean diverged main — jari via Telegram ([`intake-feature-orchestratectl-5565259bd11f`](issues/intake-feature-orchestratectl-5565259bd11f/item.md))
