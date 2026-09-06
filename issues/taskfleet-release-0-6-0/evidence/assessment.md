# Assessment: R10 Phase A release-safety review

| # | Finding | Conf | Like | Read | Arch | Confidence | Recommendation |
|---|---|---|---|---|---|---|---|
| F1 | Bash `if`/errexit authorization bypass | CONFIRMED | REGULAR | IMPROVES | MINOR | HIGH | FIX — fixed |
| F2 | Reusable-gate failure could skip builds into permissive host | CONFIRMED | OCCASIONAL | IMPROVES | MODERATE | HIGH | FIX_WITH_CARE — replaced |
| F3 | PR release trigger exposed inherited repository secrets | CONFIRMED | OCCASIONAL | IMPROVES | MODERATE | HIGH | FIX_WITH_CARE — removed |
| F4 | Exact-main alone did not establish wrapper authorization | CONFIRMED | OCCASIONAL | IMPROVES | MODERATE | HIGH | FIX_WITH_CARE — protected ref added |
| F5 | Prepared/active validation conflicted | CONFIRMED | REGULAR | IMPROVES | MINOR | HIGH | FIX — fixed |
| F6 | `gh api` 404 body prevented create-ref path | CONFIRMED | REGULAR | IMPROVES | MINOR | HIGH | FIX — fixed |
| F7 | Tag-time live-main lookup can burn an authorized version | CONFIRMED | OCCASIONAL | IMPROVES | MINOR | HIGH | FIX — fixed |
| F8 | Unsafe topology fixture was dead code | CONFIRMED | REGULAR | IMPROVES | NONE | HIGH | FIX — fixed |
| F9 | Release runner prerequisites were implicit | CONFIRMED | OCCASIONAL | IMPROVES | MINOR | HIGH | FIX — fixed |
| F10 | Checked archive could select an unrelated `dist` | CONFIRMED | RARE | IMPROVES | NONE | HIGH | FIX — fixed |
| F11 | Ruleset IDs did not prove live policy | CONFIRMED | OCCASIONAL | IMPROVES | MINOR | HIGH | FIX — fixed |
| F12 | Repository-scoped Homebrew proof absent | CONFIRMED | REGULAR | IMPROVES | MODERATE | HIGH | FIX_WITH_CARE — Homebase completed |
| F13 | `host --steps=create` mutates during planning | INCORRECT | — | — | — | HIGH | DROP (Rule 1a) |
| F14 | Workflow-wide write permission requires generated hand-edit | CONFIRMED | RARE | WORSENS | MODERATE | MED | DROP (Rule 1d; bounded upstream limitation) |
| F15 | Authorization ref must be atomic with Shipshape tag push | CONFIRMED | RARE | WORSENS | MAJOR | MED | DROP (Rule 1d; same-journal saga is clearer) |

FIX: 7   FIX_WITH_CARE: 5   SPIN_OFF: 0   DISCUSS: 0   DROP: 3

Every confirmed release-safety or evidence gap within the pinned topology was fixed. No finding meets the bar for a new issue: the two retained limitations are explicit properties of exact cargo-dist 0.28.2 and the already-decided resumable cross-domain release saga, not unscheduled speculative work.
