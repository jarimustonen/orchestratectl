---
created: 2026-06-28
updated: 2026-06-28
type: feature
status: fixed
priority: normal
commits:
- hash: b35db3c
  summary: run merge verb + interactive cleanup-on-explicit-merge
closed: 2026-06-28
---

# Bundle /worktree-merge into orchestratectl + interactive-kind cleanup-on-merge

## Description

Bundle `/worktree-merge` into the orchestratectl binary so the complete interactive-worktree lifecycle (spawn → review → merge → cleanup) is owned end-to-end by orchestratectl, not partially by orchestratectl + partially by the homebase merge.sh script with no integration between them.

Today's gap (observed 2026-06-28):

User ran `/worktree-merge` in an interactive `--kind code` worktree (`bots-image-vision-gemini`). The git merge into main succeeded, but the tmux window + worktree directory + branch were left in place. The user expected them to disappear. Root cause: homebase `/worktree-merge` is a bash-driven skill that calls `workmux merge` directly — it has no knowledge of the orchestratectl run, does not submit a terminal `node.report`, does not signal the per-run supervisor to clean up. The supervisor's auto-cleanup (commit ed99cc7) only fires on terminal `node.report` AND only for autonomous kinds (interactive kinds were intentionally excluded so /worktree-code spawns don't auto-close the user's review window).

The current policy "interactive kinds = user owns the window, no auto-cleanup" was correct for the SPAWN phase but is wrong for the MERGE phase. When the user explicitly runs `/worktree-merge`, that IS the signal that the window can close.

Scope of this change:

1. **New CLI verb** `orchestratectl run merge <run-id> [--source <branch>]` (or equivalent — design call). Owns the full merge lifecycle:
   - Detects the worktree's branch and source branch (from manifest, or `--source` override)
   - Rebases the branch onto source if needed
   - Performs the merge into source
   - Submits a terminal `node.report` on the run (success/failure structured per §7.3)
   - Returns the structured outcome via the standard JSON envelope
   - Does NOT itself tear down the worktree / tmux / branch — the supervisor does that via the existing cleanup path once it sees the terminal report

2. **Supervisor policy change**: when an interactive-kind run reaches terminal via `node.report` (NOT via supervisor's spawn-time decisions), allow the same cleanup path as autonomous kinds (close tmux window, remove worktree, delete branch). The trigger condition must be "user-initiated merge" — i.e. the supervisor sees the report came from an explicit merge action, not from a watchdog or other source. One way: the new `run merge` verb sets a flag in the report (e.g. `via: "explicit-merge"`) and the supervisor's cleanup checks for it before firing on interactive kinds.

3. **Bundled SKILL** `crates/octl-cli/skills/worktree-merge/SKILL.template.md`:
   - Same template structure as the other 10 bundled skills (frontmatter with cli_version, Workflow, Errors, Install or upgrade, etc.)
   - Body instructs the agent to call `orchestratectl run merge <run-id> [--source <branch>]` from inside the worktree
   - Discovers the run-id from the branch prefix (same snippet as worktree-orchestrated)
   - On merge failure (conflicts): SKILL says how to recover (e.g. `/complex-rebase` may still be the fallback, or document a new path)
   - On merge success: the SKILL's job is done; the supervisor handles cleanup; the agent's session ends naturally as the tmux window closes

4. **CLI implementation of the merge mechanics**: the actual rebase + merge can wrap existing merge.sh logic (shell out to it) for v1 to avoid re-implementing the conflict-detection / complex-rebase fallback. Goal of v1 is the orchestratectl integration, not a from-scratch git wrapper. v2 can move logic into Rust (or wait for the workmux library extract).

5. **Sunset homebase `/worktree-merge`**: once bundled skill is deployed, remove `~/.claude/skills/worktree-merge/`. Skill catalog audit (`orchestratectl doctor`) should report the new bundled version in sync.

6. **`worktree-spinoff` and siblings' SKILL update**: the autonomous-merge contract currently says "merge back via /worktree-merge". Update to point at the new bundled verb. Plus: spinoff agents that previously called `/worktree-merge` followed by `orchestratectl node report` now have a SINGLE step (`orchestratectl run merge` does both). Simplify their workflow sections.

Why one campaign, not many issues:

The 4 sub-pieces are tightly coupled:
- New CLI verb must exist before the SKILL can reference it
- SKILL must exist before homebase can be sunset
- Supervisor policy change is the missing link that makes the SKILL actually clean up the window
- Autonomous-merge SKILL update folds in the simplification

Done as a single campaign with sequenced sub-tasks, this is ~1 day of work. Done as separate issues with separate spawns, it has merge-conflict risk on the SKILL files and inter-issue coordination overhead.

Acceptance:

1. `/worktree-merge` SKILL is bundled. `orchestratectl skill list` shows it.
2. User runs `/worktree-merge` from inside an interactive worktree → branch merges into source → tmux window closes within seconds → worktree directory + branch are gone — no manual cleanup needed.
3. Same flow works for autonomous kinds (spinoff, research, etc.) — they ALREADY work, but the simplification means the SKILL only has one closing step (`orchestratectl run merge`) instead of two (`/worktree-merge` + `node report`).
4. Homebase `~/.claude/skills/worktree-merge/` removed (or stub redirecting to bundled).
5. `orchestratectl doctor` reports 47 ok / 0 fail (one more bundled SKILL).
6. `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` all green.
7. Re-deploy: `cargo install --path crates/octl-cli --force && orchestratectl skill install --force`.

Severity: HIGH for UX completeness. Today the interactive-worktree user experience has a manual cleanup step that surprises everyone the first time. This is the last big rough edge in the spawn → work → cleanup loop.
