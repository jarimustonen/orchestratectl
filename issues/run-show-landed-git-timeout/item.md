---
created: 2026-08-15
updated: 2026-08-20
type: improvement
status: wontfix
priority: normal
epic: lifecycle-architecture-review
closed_by: claude
closed: 2026-08-16
---

# run show landed check has no git subprocess timeout

## Description

The `landed` signal (`crates/taskfleet-cli/src/run/landed.rs::git_verify_landed`) shells out to up to three git subprocesses per `run show` — `git cherry`, `git merge-base --is-ancestor`, `git rev-list --count` — each via `Command::output()` with **no timeout**. A hung git (NFS-backed repo, a `.git` lock, a pathological history) blocks `run show` indefinitely, with no error surfaced.

This is pre-existing `landed.rs` behavior shared by every landing consumer (`run show`, `run wait`, `run list`), not a regression of the `false_failed` change — but the new `false_failed` read-time consumer (issue `raw-git-selfmerge-false-failed`) runs the same path on every `run show`, amplifying the cost on the status-navigation hot path.

Surfaced by multi-model llm-review (anthropic #4, deepseek #5) during the `raw-git-selfmerge-false-failed` review.

**Scope (needs design):**
- A timeout wrapper around the git subprocess calls (e.g. a bounded wait + SIGKILL), returning `None` (→ marker fallback / `unverified`) on elapse rather than hanging.
- OR make landing verification opt-in behind a `--verify-landed` flag on `run show`.
- OR cache the verdict on the `Node` projection with a TTL so repeated `run show` calls don't re-shell-out.
- Applies uniformly to all `landed` consumers; must not regress the rebase-robust semantics.

**Acceptance:** `run show` cannot hang unbounded on a slow/stuck git; the landing verdict semantics are unchanged for a healthy repo.

## Resolution

### 2026-08-16T15:32:58Z · @claude

Suljettu epärealistisena. Issue perustelee riskin verkkolevyllä (NFS) ja patologisella historialla; kumpaakaan ei ole. Paikallisella levyllä git ei jumitu ikuisesti — vanhentunut index.lock palauttaa virheen, ei riipu. Ei havaittua esiintymää.
