---
created: 2026-07-22
updated: 2026-07-22
type: bug
reporter: jari
status: open
priority: normal
related: ['@agent-died-merge-no-teardown-interactive']
---

# orchestrate: integration branch created without a worktree → child run merge fails with merge_failed

_Version: orchestratectl 0.1.0. Source: `/orchestrate` skill (integration-branch setup) + `run merge` backend (merge.sh)._

## Description

**Reporter:** 3dbear-monorepo main session running `/orchestrate` for a 3-feature campaign (`opiskelijaryhmat-mvp`).
**Reported:** 2026-07-22

The `/orchestrate` skill creates the shared integration branch with a plain
`git branch <integration> <source>` (SKILL.md § "Create the integration branch",
~line 189). This makes a ref but **no worktree**. When a child
(`--kind orchestrated`) later runs `orchestratectl run merge <child-run-id>`, the
merge targets that integration branch as its `source_branch`, and the merge
backend (`merge.sh`) fails:

```
merge_failed: merge.sh exited 1 merging wt/<short>-<slug>:
  Merge worktree: wt/<short>-<slug>
```

`run merge --dry-run` **passes** (branch + source + report file all validate) — the
failure is only at execution time, which made it look like a report/branch problem
at first. Root cause: `merge.sh` needs the *target* (integration) branch checked
out in a worktree to merge into it; a bare branch ref has no working tree, so the
merge cannot land.

## Reproduce

1. `orchestratectl run create --kind orchestrate …` (driver run).
2. `git branch orchestrate/<slug>-<date> main` (as the skill instructs).
3. Spawn a child: `orchestratectl run create --kind orchestrated --source-branch orchestrate/<slug>-<date> --parent-run-id <drv> …`.
4. Child commits its work, then `orchestratectl run merge <child-run-id>`.
5. → `merge_failed` (see message above), even though the child branch is a clean
   fast-forward onto the integration branch and `--dry-run` succeeds.

## Workaround (used this session)

Check the integration branch out into its own worktree *before* the first child
merge:

```bash
git worktree add ~/Sources/<repo>__worktrees/wt-integration-<slug> orchestrate/<slug>-<date>
```

After that, all child `run merge` calls succeed (fast-forward + terminal report +
teardown all work normally). The extra worktree then has to be removed by hand at
campaign end (`git worktree remove … --force` + `git branch -D`).

## Suggested fix

Two candidate directions:

1. **Skill-side (cheap):** change `/orchestrate` SKILL.md § "Create the integration
   branch" to `git worktree add` the integration branch into a managed path instead
   of a bare `git branch`, and have § 7 (final synthesis) tear that worktree down.
   This keeps the fix in the skill without touching the merge backend.
2. **Backend-side (robust):** `merge.sh` / `run merge` could detect that the target
   branch has no worktree and create a temporary detached one for the merge, then
   remove it — so a bare integration branch "just works". This makes `run merge`
   robust regardless of how the caller set up the target.

Preference: (1) is a one-line skill change and unblocks the common `/orchestrate`
path immediately; (2) is the durable fix. Consider doing both — (1) now, (2) as the
real guard.

## Notes

- `run merge --dry-run` giving a green result while the real merge fails is itself a
  small papercut — the dry-run doesn't check that the target branch is materialized
  in a worktree. Worth having the dry-run surface this so the failure is caught
  before the child has committed.

