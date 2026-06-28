# TODO

Currently open work — what to do, in what order, why.

For longer-running planning + design docs see `issues/<slug>/{plan,design,breakdown,validation,handoff,decisions}.md`. This file is the **session-level** plan and points at issuectl issues for the actual tracked work.

---

## Status snapshot (2026-06-28)

- ✅ **MVP epic** [`orchestratectl-mvp`](issues/orchestratectl-mvp/item.md) — `done`. Binary spawns runs end-to-end through all 8 worktree kinds, supervisor watchdog, dedup, signal handling, etc.
- ✅ **Follow-up campaign** (21 review-spinoff issues + 9 packs) — all merged.
- 🟡 **Skill-bundling campaign** [`skill-bundling-campaign`](issues/skill-bundling-campaign/item.md) — **active**, this is what TODO.md tracks below.
- ⏸ **Held items** (not in current scope):
  - [`help-json-depth-control`](issues/help-json-depth-control/item.md) — schema bump for `--help --json` top-level depth, needs Jari's product decision before authoring.
  - [`runwriter-batched-append-api`](issues/runwriter-batched-append-api/item.md) — V4 latency (639ms p99 vs 10ms budget). Per B1 decision: accepted as-is post-MVP unless `/orchestrate`/`/fan-out` actually scale to where it matters.

---

## Active campaign: replace homebase skills with binary-bundled SKILL.md

**Why.** Per `AGENTS-AI-FIRST-CLI.md` §17, the binary is the source of truth and the skill follows it. Today the actual `~/.claude/skills/worktree-*` / `/orchestrate` / `/fan-out` skills still live as standalone files in homebase that call `~/.claude/skills/worktree/scripts/create.sh` directly. They DO work (the create.sh patch we landed gives them structured stdout + the 30 s timeout + slash-handling), but none of them go through `orchestratectl run create`, so nothing about a real run lands in `~/.orchestratectl/runs/`. The supervisor, watchdog, deterministic dedup, exactly-once report consumption — all of it — sits unused.

**Approach.** Author the replacement SKILL.md files as **bundled skills inside the binary** (`crates/octl-cli/skills/<name>/SKILL.md`). Each skill calls `orchestratectl run create --kind <X>` (and friends) instead of bare `create.sh`. Deploy via `orchestratectl skill install --all --force` over `~/.claude/skills/`. §17's `cli_version` frontmatter + drift-detection then make version sync a one-call audit.

**Sequence.** One skill at a time. Phase 1 sets the contract (frontmatter shape, error-envelope expectations, Issue Management section, Workflow section). Every subsequent phase reads phase 1's output as the canonical template and notes deviations explicitly. If phases 2/3 surface a contract bug in phase 1, fix it there (regress-commit) before continuing — keep the family internally consistent.

**Each phase delivers:**
- SKILL.md (with `build.rs`-substituted `cli_version:` frontmatter) under `crates/octl-cli/skills/<name>/`
- Skill registry update so `orchestratectl skill list` + `skill print` + `skill install` see it
- Build / test / clippy / fmt clean
- Smoke: `skill print <name>`, `skill install <name> --dest /tmp/test/SKILL.md`, then for phase 1+ a real spawn through the new skill into a throwaway worktree, verify `orchestratectl run show <id>` sees it
- `/llm-review` over the SKILL.md text (it's prose, not code — review focuses on agent-actionability + contract fidelity)
- Merge

---

## Phases

| # | Issue | What | Why this order |
|---|---|---|---|
| 1 | [`skill-bundle-worktree-spinoff`](issues/skill-bundle-worktree-spinoff/item.md) | Author `crates/octl-cli/skills/worktree-spinoff/SKILL.md`. Calls `orchestratectl run create --kind spinoff`. | Simplest + most-used → sets the contract for all others. |
| 2 | [`skill-bundle-worktree-code`](issues/skill-bundle-worktree-code/item.md) | `worktree-code` (interactive variant). | Demonstrates `lifecycle: interactive` path (waits for human, different merge flow). |
| 3 | [`skill-bundle-worktree-orchestrated`](issues/skill-bundle-worktree-orchestrated/item.md) | `worktree-orchestrated` with `--parent-run-id` + `--parent-node-id`. | Establishes child-spawn pattern — Phase 5 needs this for `/orchestrate`. |
| 4a | [`skill-bundle-worktree-research`](issues/skill-bundle-worktree-research/item.md) | Autonomous research worktree. | Mechanical after Phase 1+2 contract. |
| 4b | [`skill-bundle-worktree-make-skill`](issues/skill-bundle-worktree-make-skill/item.md) | Autonomous skill-authoring worktree. | Mechanical. Note: this skill is what would author future bundled skills — meta. |
| 4c | [`skill-bundle-worktree-bugfix`](issues/skill-bundle-worktree-bugfix/item.md) | Autonomous bugfix worktree. | Mechanical. |
| 4d | [`skill-bundle-worktree-technical-decision`](issues/skill-bundle-worktree-technical-decision/item.md) | Autonomous ADR worktree. | Mechanical. |
| 5 | [`skill-bundle-orchestrate`](issues/skill-bundle-orchestrate/item.md) | `/orchestrate` DAG runtime SKILL.md. | Substantial. Spawns lapsiagentit per `--kind orchestrated`, reads `event tail --follow` for reports, synthesizes worker output, handles failure modes. **Needs a design conversation with Jari before authoring** — the DAG runtime in prose is a real design problem. |
| 6 | [`skill-bundle-fan-out`](issues/skill-bundle-fan-out/item.md) | `/fan-out` SKILL.md. | Analogous to `/orchestrate` but simpler — N identical units in parallel, `--kind fan-out` per child. |
| 7 | [`skill-rollout-and-sunset`](issues/skill-rollout-and-sunset/item.md) | `skill install --all --force` over `~/.claude/skills/`; smoke per kind; add `orchestratectl run adopt` (or `import-existing-tmux`) for pre-existing `wm-*`/`🎬 🚀`-prefixed windows; sunset homebase `~/.claude/skills/worktree-*` (or keep one release as fallback). | Ships the campaign. |

**Estimate (rough):**
- Phase 1: 1–2 h authoring + Jari review (~30 min) — quality determines the next 9 phases
- Phases 2–3: ~1 h each + ~10 min review
- Phase 4 (a–d): ~30 min each, mechanical — could batch 2+2 in parallel once contract is solid
- Phase 5 (`/orchestrate`): 2–3 h, with a design conversation up front
- Phase 6 (`/fan-out`): 1–2 h
- Phase 7: ~1 h rollout + smoke

**Total**: ~10–14 h across multiple sessions.

---

## Skills NOT in this campaign

These already exist in homebase and don't need a binary-bundled replacement (for now):

- `/worktree-merge` — thin wrapper around `workmux merge`; no orchestratectl interaction needed.
- `/worktree` — router (delegates to `/worktree-spinoff` etc.); needs no behavior change.

If a future iteration decides the router should also be bundled, file a new issue.

---

## How to start a phase

1. Pick the lowest-numbered open phase issue.
2. Spawn an **interactive** worktree-code (not spinoff) — Jari reviews each skill before it sets the precedent for the next.
3. The interactive worktree prompt should:
   - Read this TODO.md
   - Read the previous phase's merged SKILL.md (`crates/octl-cli/skills/<previous>/SKILL.md`) for the contract template
   - Read the homebase original (`~/.claude/skills/<name>/SKILL.md`) for the working semantics it needs to preserve
   - Read `orchestratectl --help --output json` for the available commands
   - Author `crates/octl-cli/skills/<name>/SKILL.md.template` with `{{CLI_VERSION}}` placeholder
   - Register in the skill list
   - Build + smoke + `/llm-review` + commit + `/worktree-merge` (the user does the merge to keep the contract reviewed)
4. After merging: bump the phase issue `status: done` via `issuectl close <slug>`.
5. If the phase surfaced a contract bug in Phase 1's SKILL.md, fix that first as a regress-commit and update Phase 1's issue body with a note.

---

## When the campaign finishes

- All 10 phases closed.
- `orchestratectl version --output jsonl | jq .data.skills` lists 10+ bundled skills, all `cli_version` aligned with the running binary.
- `~/.claude/skills/worktree-*` etc. either replaced (via `skill install --force`) or removed (sunset).
- Smoke run: spawn one of each of the 8 kinds through the new skill chain, verify each produces a manifest in `~/.orchestratectl/runs/<id>/` and gets a supervisor PID.
- This TODO.md gets archived (move to `issues/skill-bundling-campaign/handoff.md` or delete).
