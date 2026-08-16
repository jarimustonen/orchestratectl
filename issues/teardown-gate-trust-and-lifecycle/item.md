---
created: 2026-07-03
updated: 2026-08-16
type: improvement
reporter: jari
status: obsolete
priority: normal
labels: [supervisor, followup, rescope-0.2]
related: ['@blocked-report-deletes-branch']
closed: 2026-08-16
closed_by: claude
---

# Harden teardown-gate trust boundary + preserved-worktree lifecycle

_Source: crates/octl-cli/src/supervise/cleanup.rs_

## Description

Follow-ups surfaced by the multi-model `/llm-review` pass on
`@blocked-report-deletes-branch` (fixed in commits `fe44a56` + `498cf5d`).
None is a standalone data-loss blocker — the source-relative unmerged-work
check now preserves committed work on essentially any non-merge outcome — but
each is a legitimate hardening the reviewers flagged and worth doing before
v0.1.0 hardens the supervisor contract.

## Items

### 1. `via` / `cancelled` are trusted from agent-controlled report JSON

`node_merged_explicitly` reads `last_report.via == "explicit-merge"` and
`node_report_is_blocked` reads `cancelled` straight out of the free-form
`node.report` payload. A plain `node report` that stamps
`{"success": false, "via": "explicit-merge"}` would flip `merged = true` and
force-delete (`git branch -D`) the node's branch, skipping the source-relative
safety net. This trust boundary predates the fix (`any_node_merged_explicitly`
already trusted `via`), and the realistic exploit is an agent destroying its
OWN work (which it could do with `git branch -D` anyway), so it was deferred.

Fix direction: reserve `via` / `cancelled` in the `node report` validator
(`crates/octl-core/src/report.rs`) so only `run merge` / `run cancel` can stamp
them, OR derive the outcome from event provenance (distinct event kinds) rather
than payload fields. `success` is already validation-required, so the
missing-`success` loophole is closed; this is the remaining spoofable field.
Add tests: a plain `node report` cannot spoof `via: explicit-merge` /
`cancelled: true`.

### 2. Preserved-worktree lifecycle (accumulation + discoverability)

The blocked / unmerged path intentionally leaves the worktree registered in
`git worktree list` and the branch in place. There is no follow-up lifecycle:

- No `orchestratectl run status` / list surface showing "work preserved here:
  branch X, worktree Y" (today it is only a `cleanup.branch_preserved` event +
  supervisor stderr). Consider a manifest/projection field so a CLI view can
  render it.
- No GC / prune path once the human has merged — stale `.git/worktrees/*`
  entries and `wt/*` branches accumulate over many blocked runs. Consider a
  `run reclaim` / `cleanup preserved` verb, and confirm the next `run create`
  cannot collide on a preserved worktree path.
- `cleanup.branch_preserved` records a `worktree_path`/`branch` without
  verifying they still exist — an operator who manually removed one mid-flight
  gets an event that over-claims. Consider recording `branch_exists` /
  `worktree_exists` booleans.

### 3. Minor / optional

- `delete_branch`'s `merged: bool` argument is a mild code smell — could split
  into `force_delete_branch` / `safe_delete_branch`, or move the
  `node_merged_explicitly(n)` call inside since it already has `n`.
- Consider a closed `TerminalOutcome` → `CleanupPolicy` enum mapping so new
  terminal outcomes must explicitly opt into a worktree/branch policy rather
  than defaulting into the teardown arm (gpt-5.5's suggestion).
- A distinct `blocked` manifest status (vs `failed`) would make the
  preserved-branch scenario discoverable at the run level.

## Notes

Raw review corpus: `history/review-blocked-report-deletes-branch-raw.txt`
(gitignored). Consensus top finding (source-relative check) was already
implemented in `498cf5d`.

## Decisions

### 2026-08-13T11:10:43Z · @adr-decision-2

RE-SCOPE: The teardown gate survives (invariant 5), but the report-shape trust decision is re-framed by the A6 typed-outcome table + A1 exit shim. Re-target the hardening at the typed-outcome gate rather than report-payload inference. Recorded by ADR 0001 (docs/decisions/0001-thin-supervisor-vs-harden.md).

## Resolution

### 2026-08-16T15:33:24Z · @claude

Suljettu ohitettuna. Vuoden 2026-07 katselmusjäännös purkuvaiheen suojauksista; sen jälkeen purkuvaihe on rakennettu uusiksi (tyypitetty TerminalOutcome-taulu, lähdesuhteinen unmerged-tarkistus, likaisen työpuun vahti, HEAD-suhteinen vahti, non-force removal). Listan kohdista suurin osa on toimitettu ja loput ovat teoreettisia — ne kirjattiin erikseen ja suljettiin samassa triagessa.
