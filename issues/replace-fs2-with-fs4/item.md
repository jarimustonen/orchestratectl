---
created: 2026-06-12
updated: 2026-06-27
type: chore
status: done
priority: normal
epic: orchestratectl-mvp
labels: [review-spinoff, cargo-scaffolding-review]
commits:
- hash: '1101604'
  summary: swap fs2 → fs4, V4 stress test passes
- hash: 105247b
  summary: pin fs4 sync feature + clarify lock wording (review fixes)
closed: 2026-06-27
---

# Replace unmaintained fs2 with fs4 (or rustix)

## Description

fs2 is unmaintained (~6 years) with known soundness issues on some platforms. design.md §6 currently picks fs2 — replacing it requires touching the design table. Pick fs4 (drop-in API) or rustix (lower-level). Surfaced by cargo-scaffolding review.
