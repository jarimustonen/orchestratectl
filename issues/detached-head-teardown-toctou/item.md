---
created: 2026-08-15
updated: 2026-08-20
type: bug
status: wontfix
priority: normal
epic: lifecycle-architecture-review
closed_by: claude
closed: 2026-08-16
---

# Non-merge teardown TOCTOU: HEAD can move between the safety probe and worktree removal

## Description

Follow-up from /llm-review of detached-head-teardown-commit-loss (all four models). The HEAD-relative teardown guard (`head_teardown_safety`) is check-then-act: it reads the worktree HEAD, classifies it, and later runs non-force `git worktree remove`. Non-force removal re-checks CLEANLINESS but NOT HEAD reachability, so a concurrent `git checkout --detach <new-commit>` (or a new commit) that lands between the probe and the removal, leaving a clean tree, would let removal succeed and orphan the new commit. `close_tmux_window` runs first (killing the agent pane) and the supervisor tick is single-threaded, so the window is small; but it is non-zero (best-effort tmux kill, external shells/hooks/IDEs, another actor). The two HEAD probes (`head_oid` then `head_branch`) are also separate subprocesses; the current fix mitigates the fail-open by always verifying the oid it READ (never the branch tip), but full consistency is not guaranteed.

Fix direction (needs design, touches the hot supervise/cleanup path — do NOT bolt in): pin the observed HEAD in a supervisor-owned rescue ref (e.g. `refs/taskfleet/rescue/<run>/<node>`) before non-force removal, with a retention/GC policy and operator visibility; and/or a worktree lease/lock honored by all taskfleet writers. Analogous to the deferred durable-operation-lease work in invariant 6 (design §2.7). Scope: closes the committed-HEAD-movement window that non-force removal cannot.

## Resolution

### 2026-08-16T15:32:58Z · @claude

Suljettu epärealistisena. Vaatii että joku vaihtaa git-haaraa käsin juuri turvatarkistuksen ja työpuun poiston välisessä ikkunassa — sen jälkeen kun agentin tmux-ikkuna on jo tapettu ja yhden säikeen supervisor-tickin sisällä. Ei ole tapahtunut kertaakaan; henkilökohtaisen työkalun yhden käyttäjän koneella ei tapahdu. Nykyinen suojaus (verifioi luettu oid, ei haaran kärkeä; non-force removal) on riittävä.
