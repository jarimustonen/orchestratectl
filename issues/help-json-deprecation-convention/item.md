---
created: 2026-06-27
updated: 2026-06-28
type: task
reporter: jari
status: done
priority: normal
epic: orchestratectl-mvp
labels: [review-spinoff, help-json]
closed: 2026-06-28
commits:
- hash: ed61799
  summary: '[deprecated] help-text convention'
---

# Adopt a real deprecation-status source for --help --json (deprecated currently always false)

## Description

The structured-help payload carries deprecated:false on every flag. clap 4.6 exposes NO Arg deprecation getter (verified against clap_builder source), so there is no source — the field is accurate today (nothing is deprecated) but cannot become true. Adopt a convention so deprecation can be marked: a help-text [deprecated] prefix parsed out, a side registry keyed by command-path/arg-id, or a wrapper attribute. Extend deprecation to CommandNode and PositionalInfo, not just FlagInfo. Until then the field stays false. Note: one reviewer wrongly claimed Arg::is_deprecated() exists — it does not in clap 4.6. Surfaced in issues/help-json-structured/review.md.
