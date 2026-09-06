---
created: 2026-08-04
updated: 2026-08-04
type: task
status: done
priority: normal
related: ['@stint-maintains-execution-dag', '@triage-bugs-stint-inprogress-ownership-conflict']
closed: 2026-08-04
---

# Split /stint into stint-start + stint-handoff; decouple bug intake

## Description

Decompose the monolithic bundled `/stint` skill into two orx-maintained bundled skills
plus a shared reference file, and fully decouple bug intake. Settled with Jari in a live
`/stint` session (2026-08-04).

## Design (settled)

1. **`stint-start`** — the round engine, run EVERY round: orient (Phase 0: pull, read
   operating policy, ground-truth-from-git, Execution-DAG merge) → plan → orchestrate
   (spawn worktrees) → deploy (local rebuild when the project permits) → report
   (`/worktree-status`). Phase-6 feedback mini-rounds are just a re-run of `stint-start`,
   NOT duplicated spawn/deploy logic. Carries the current trigger phrases ("aloitetaan
   rupeama", "jatketaan @TODO.md", "start a work session", bare invocation).
2. **`stint-handoff`** — terminal wrap ONLY: update the `TODO.md` `## 🔄 Continue here`
   block + final Execution-DAG merge (commit on its own), then `/wrap-up`; test-account
   reset reminder if the project declares one. Run on the user's go at session end.
3. **Bug intake fully decoupled** — REMOVE Phase 1 (the `/triage-bugs --no-pull`
   invocation and the mandatory fix-now/defer/not-a-bug pause) from stint entirely.
   `/triage-bugs` stays in homebase (`dotfiles/src/.claude/skills/triage-bugs/`),
   user-invoked and independent. stint-start must NOT reference or call it. (Note: the
   Phase-0 pull that used to feed triage stays in stint-start as a plain pull.)
4. **Shared reference file under stint-start** — extract the shared prose (Execution-DAG
   convention + notation/rules, operating-policy reading, project-prerequisites) into a
   reference file UNDER the `stint-start` skill dir (e.g.
   `crates/taskfleet-cli/skills/stint-start/AGENTS-EXECUTION-DAG.md` or similar); `stint-handoff`
   LINKS to it. No separate `stint-shared` skill.

## Mechanics / done criteria
- Both new skills are BUNDLED: `crates/taskfleet-cli/skills/stint-start/SKILL.template.md` and
  `.../stint-handoff/SKILL.template.md`, wired into the skill install registry so
  `taskfleet skill install` installs them and `taskfleet doctor` reports
  `skill.sync.stint-start` / `skill.sync.stint-handoff` ok.
- Old bundled `stint` skill removed from the registry. JUDGMENT CALL to record in the
  terminal report: whether to keep a thin `stint`→`stint-start` alias for muscle-memory
  or remove `/stint` outright. Default: remove; make `stint-start` carry the triggers.
  If unsure, record as a `discussion_items[]` entry.
- insta snapshots regenerated (bundled-skill catalog is snapshotted); `cargo test
  --workspace` green; `cargo fmt --all`; `cargo clippy --workspace --all-targets` no NEW
  warnings.
- No behavioural regressions to the round logic — the split is organizational; the DAG
  merge rules, spawn discipline, and deploy gating are preserved verbatim (moved, not
  rewritten).

## Related
- `stint-maintains-execution-dag` (the DAG convention this shared file will host).
- `triage-bugs-stint-inprogress-ownership-conflict` (already fixed in homebase; confirms
  triage-bugs is homebase-owned).
