---
created: 2026-07-27
updated: 2026-07-27
type: improvement
status: done
priority: high
related: ['@code-pipeline']
commits:
- hash: 645fa06
  summary: implement DAG maintenance in stint skill
- hash: 41dc10d
  summary: apply skill-review findings to DAG convention
closed: 2026-07-27
---

# stint maintains an issue-derived execution DAG in TODO.md

## Description

The /stint orchestrator should treat the execution DAG as a first-class, continuously-maintained artifact in TODO.md — not an ad-hoc thing built once. Design this properly, then implement it in the bundled stint skill (crates/octl-cli/skills/stint/SKILL.template.md).

## Problem
Today the stint skill plans a round from the TODO.md handoff + open issues, but there is no durable, always-current dependency graph telling a *fresh* agent what to do next. When a session hands off, the next agent re-derives ordering from prose. A one-off DAG was hand-built in TODO.md on 2026-07-27 (lanes A/B/C, file-collision edges); this issue makes maintaining such a DAG a standing part of the workflow.

## What we want
- The stint skill has an explicit instruction to **maintain an execution DAG in TODO.md** derived from the open issuectl issues, so the agent always knows the next actionable task (head-of-line per lane) and the ordering constraints (file-collision edges + logical deps).
- **New issues must be inserted into the DAG** when filed/triaged (or when a round is planned), so the graph never goes stale.
- A fresh agent resuming from 'jatketaan @TODO.md' can read the DAG and immediately know what is ready vs blocked.

## Design must decide (this is the 'suunnitellaan kunnolla' part)
- **Representation**: the exact DAG format in TODO.md (the current lane-based ASCII block vs a node/edge list vs a table). Must be human-readable AND cheaply machine-updatable by an agent. Weigh whether a fenced code block, a checklist, or a small structured (YAML-in-fence) form is best.
- **Edge semantics**: what edges mean (file-collision 'must sequence' vs logical dependency vs cross-lane). The current DAG uses hot-file lanes as the primary partition — validate or revise that model.
- **Where edges come from**: how the agent derives collision edges (from the repo's hot-file list in root CLAUDE.md/AGENTS.md) and dependency edges (issue 'related'/'blocked_by' frontmatter). Consider leaning on issuectl's blocked_by field as the source of truth rather than free prose.
- **Maintenance triggers**: which stint phases update the DAG (Phase 2 planning, Phase 1 triage inserting new bugs, Phase 7 handoff), and the exact edit each performs.
- **Head-of-line selection**: how the agent picks the next task and marks in-progress vs done in the DAG without racing the worktree's own issue-lifecycle updates.
- **Sync with issuectl**: avoid duplicating state — decide what lives in TODO.md (ordering/graph) vs what stays authoritative in issuectl (status). The DAG should be a *view/plan*, not a second source of truth for status.
- **Staleness/repair**: how the agent detects a DAG entry whose issue was closed/renamed, and reconciles.

## Deliverables
1. A design doc at issues/stint-maintains-execution-dag/design.md that resolves the above (consider an /llm-panel or /llm-workshop pass — this is workflow design worth diverse input).
2. Implementation: update crates/octl-cli/skills/stint/SKILL.template.md (and any helper script under that skill dir) to encode the DAG-maintenance convention across the relevant phases. Keep the skill generic (project facts stay in the repo's AGENTS.md/TODO.md).
3. If a DAG format/section convention should live in TODO.md structurally, document it (e.g. a template stub the skill writes into).
4. Redeploy: cargo install --path crates/octl-cli --force && orchestratectl skill install --force && orchestratectl doctor (0 fail/0 warn).

## Notes
- The current hand-built DAG in TODO.md (## Execution DAG (2026-07-27)) is the working example to generalize from — read it.
- Bundled-skill change ⇒ insta snapshot loop applies (see crates/octl-cli/CLAUDE.md).
