---
created: 2026-07-24
updated: 2026-08-05
type: feature
status: done
priority: normal
commits:
- hash: 6322a9f
  summary: expose rebase-robust landed signal on run wait/show; fix stint-start + worktree-spinoff skill docs
- hash: 098d6e9
  summary: harden landed git-verification per llm-review (ancestry net, git-authoritative precedence, success-gate, arg guard, doc clarifications)
closed: 2026-08-05
---

# run wait/show should expose a reliable git-verified 'landed' signal; stint skill's is-ancestor check gives false negatives after rebase

## Description

Two-part improvement (CLI + bundled skill doc), same root cause, same repo.

### The trap (hit twice in one real /stint session, 2026-07-22)

The `worktree-spinoff` / `stint` skills instruct the caller to confirm a landing
**from git**, not from run status ("settled ≠ landed"), using:

```
git merge-base --is-ancestor <worker-branch> <target>
```

This check is **unreliable in exactly the environment /stint targets**: a
heavy-parallel repo where other sessions push to `origin/main` continuously. The
conductor must `git rebase origin/main` its local `main` repeatedly during a
round. After such a rebase:

- The worker's merge commit is **replayed under a new hash** on the rebased
  local `main`.
- The worker **branch ref** still points at the **pre-rebase** hash.
- `git merge-base --is-ancestor <worker-branch> main` then returns **false**
  ("not landed") even though the worker's *content* is fully merged.

Result: the conductor concludes "the worker died / didn't land," and nearly
takes a destructive recovery action (re-spawning a redundant finisher spinoff, or
hand-salvaging committed-and-merged work). In the observed session this false
negative fired **twice** — for the importer-content-match spinoff and again for
the audit-trail spinoff — both of which had in fact merged cleanly. Ground truth
was only recoverable by checking file/symbol presence on `main`, `git log
origin/main`, and the `run wait` `merged: true` flag.

### Part 1 — CLI: expose a trustworthy `landed` signal

`run wait` already returns `merged: true` in its envelope, and that flag was
correct in the session where the git ancestry check lied. Make this the
first-class, documented landing signal:

- `run wait` / `run show` should surface a **`landed`** (or keep `merged`)
  boolean that is computed robustly — e.g. by checking whether the worker's
  **tree/patch-id** or its recorded merge commit is reachable from the current
  target, not by branch-ref ancestry that a caller-side rebase invalidates.
- Ideally the CLI verifies against the **actual target branch tip** (post any
  rebases) so the caller never has to run `merge-base --is-ancestor` by hand.

### Part 2 — bundled skill docs: fix the misleading instruction

`stint` and `worktree-spinoff` (installed via `orchestratectl skill install`)
tell the caller to git-verify via `git merge-base --is-ancestor <branch>
<target>`. Update that guidance to:

- **Warn** that a caller-side `git rebase` (which /stint does every round on a
  busy repo) invalidates branch-ref ancestry as a landing check.
- **Prefer** the CLI's `merged`/`landed` flag, or verify by **content on the
  rebased target** (`git log origin/main --oneline | grep`, file/symbol
  presence), not by the worker branch ref.

## Impact

The current guidance actively misleads the conductor precisely in the
high-parallel scenario /stint exists for. It risks (a) redundant re-spawns and
(b) hand-salvage of already-merged work — both dangerous. A reliable CLI signal
+ corrected doc removes the trap.

## Environment

- orchestratectl 0.1.0 (commit a54f0ff6), bundled `stint` + `worktree-spinoff` skills.
- Observed in a 3-round /stint on 3dbear-monorepo where `origin/main` moved ~19
  commits under the session via parallel course-work sessions.
