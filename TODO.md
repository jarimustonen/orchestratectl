# TODO

Session-level plan + handoff. Longer-running planning/design docs live under
`issues/<slug>/{design,plan,breakdown,…}.md`; this file points at issuectl issues
for the actual tracked work. Standing rules and canonical learnings live in the
root `AGENTS.md` (operating policy + state-integrity invariants) — this file
holds only the **active handoff** and a **compact stint archive**.

---

## 🔄 Continue here (ALOITA TÄSTÄ), 2026-08-20 (**stint 7: nine units landed; v0.5.0 release integration exposed three fail-closed defects; final checkpoint fix still live**)

**Current ownership — resolve this first.** Headless spinoff `01m0fggg0zg9a5ezcdgpeq5r4g`
(`release-wrapper-held-tag`, issue `release-wrapper-rejects`) is still **pending with a live supervisor** and owns its preserved worktree at
`/Users/jari/Sources/orchestratectl__worktrees/wt-cdgpeq5r4g-release-wrapper-held-tag`. It is fixing the release wrapper's held-tag checkpoint recognition. No terminal report exists and `landed` is unverified. Start by running `orchestratectl run wait 01m0fggg0zg9a5ezcdgpeq5r4g`, then read the full `run show` report and require `landed: true`; do not spawn duplicate release work while it owns the task.

**What landed and is pushed.** Nine implementation/fix units completed, all reviewed and green in their own workers:
- public-artifact audit removed personal/private defaults and metadata, added release build-path remapping, and closed `audit-no-user-specifics`;
- `doctor-report-binary-commit` discloses the running build commit and warns (never fails) on applicable repo-HEAD mismatch;
- the release contract now truthfully declares both CI-published crates plus cargo-dist GitHub Releases and Homebrew;
- `worktree-issue-provenance` forces worker-filed findings to be unlaned with machine-visible run/review provenance and optional assessment/model metadata;
- `run-prefix-collision` resolves the owning run by exact worktree identity, and `run-branch-name-ulid-entropy` gives new worker branches 50 bits of ULID randomness;
- `adopt-ossctl-release-cut` added the resumable `scripts/ossctl-release.sh` flow and deterministic snapshot bump hook;
- two real-cut defects found by the new path were fixed: unsupported `gh repo view -R` targeting (`release-wrapper-uses`) and the snapshot hook's silent exit 1 when snapshots genuinely changed (`bump-hook-fails`).

**Release state — nothing published.** v0.5.0 is **not shipped**. Two ossctl journals are deliberately abandoned and must never be resumed:
- `01M0FD8FSTMGYG8YTV92WMWC87`: bump hook failed before a bump commit/tag;
- `01M0FG88NAKBJ7Y3QNFZEHRM4K`: bump/dry-run/build/delegation succeeded and the safety hook correctly held the local tag, but the wrapper expected `current_phase=tag` while ossctl 0.9 records `current_phase=null` + failed tag phase. The remote tag was absent, zero targets were published, the journal was abandoned, and the unpublished local `v0.5.0` tag was deleted.
There are currently zero in-flight ossctl releases. After the live fix lands: run the full exact green gate again, sync/push `main`, perform the orchestrator-only commit-verified local deploy, seal a **fresh** `scripts/ossctl-release.sh plan minor`, and cut only that new plan. Never reuse the three earlier plan IDs or either abandoned journal.

**Local/deployed truth.** `main` is clean and pushed at `95fcdff` (the active issue filing commit). The installed binary remains 0.4.1 from commit `46f1aa1`; before the active issue-only commit it was verified with all 39 skill mirrors and doctor **1106 ok / 0 warn / 0 fail**. Treat deploy equality as currently stale/unverified because `HEAD` moved after that check; rebuild after the active fix lands. The full integrated CI-equivalent gate was green repeatedly through the landed bump-hook fix.

**Unscheduled intake context.** The round produced `achingly-keen-camp` (a run cannot identify its repository after worktree teardown), and external intake added `intake-bug-orchestratectl-19a653fff4c9` plus a run-list repo-filter request. They remain for the normal lane-or-close intake sweep; do not silently pull them into release recovery. `add-configurable-agent` remains deliberately deferred under the product decision recorded on its issue.

**Watch-only deploy observation.** Twice, the first `orchestratectl version` immediately after a successful `cargo install --force` briefly reported the replaced binary's old commit; the same command reported the new commit moments later without another install. The mandatory equality gate failed closed, and no orchestratectl-side cause or unsafe acceptance was observed. Take no implementation action now. If it recurs, preserve the exact command/timing/path evidence and re-evaluate `considerably-utter-deer`; do not weaken or silently retry the provenance gate.

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

- [ ] 🐛 Piialiisan bugiraportti: run create omits source_repo from fresh run manifest — jari via Telegram ([`intake-bug-orchestratectl-19a653fff4c9`](issues/intake-bug-orchestratectl-19a653fff4c9/item.md))
- [ ] 🐛 Piialiisan bugiraportti: run show cannot identify a run repository once its worktree is gone — jari via Telegram ([`intake-feature-orchestratectl-f706c536df01`](issues/intake-feature-orchestratectl-f706c536df01/item.md))
