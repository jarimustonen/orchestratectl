# Assessment: R8 evidence review

Source: `evidence/review.md`

| # | Finding | Conf | Like | Read | Arch | Confidence | Recommendation |
|---|---|---|---|---|---|---|---|
| F1 | Machine authority trusted aggregate status | CONFIRMED | REGULAR | IMPROVES | MINOR | HIGH | FIX |
| F2 | Harness bytes absent from immutable index | CONFIRMED | REGULAR | IMPROVES | MINOR | HIGH | FIX |
| F3 | Raw logs unavailable | CONFIRMED | REGULAR | IMPROVES | NONE | HIGH | FIX |
| F4 | Homebrew assertions and isolation incomplete | CONFIRMED | REGULAR | IMPROVES | MINOR | HIGH | FIX |
| F5 | Install channels omitted legacy wrapper and isolation | CONFIRMED | REGULAR | IMPROVES | MINOR | HIGH | FIX |
| F6 | CLI parity inventory insufficient | CONFIRMED | REGULAR | IMPROVES | MINOR | HIGH | FIX |
| F7 | Acceptance scenarios lacked traceability | CONFIRMED | REGULAR | IMPROVES | NONE | HIGH | FIX |
| F8 | Stress test claimed as separately run | CONFIRMED | REGULAR | IMPROVES | NONE | HIGH | FIX |
| F9 | Superseded diagnostics conflated with evidence | CONFIRMED | REGULAR | IMPROVES | NONE | HIGH | FIX |
| F10 | Private-artifact scan missing | CONFIRMED | REGULAR | IMPROVES | MINOR | HIGH | FIX |
| F11 | Formula String/Symbol mismatch | INCORRECT | — | — | — | HIGH | DROP (Rule 1a: incorrect) |
| F12 | Local pre-live formula relabel violates production generation | INCORRECT | — | — | — | HIGH | DROP (Rule 1a: incorrect) |
| F13 | Ignored flock stress test blocks R8 | INCORRECT | — | — | — | HIGH | DROP (Rule 1a: incorrect) |
| F14 | Native-only local install invalidates Linux coverage | INCORRECT | — | — | — | MED | DROP (Rule 1a: incorrect) |
| F15 | Moving nextest LEAK marker proves product leak | INCORRECT | — | — | — | MED | DROP (Rule 1a: incorrect) |
| F16 | xcrun warning invalidates test execution | INCORRECT | — | — | — | MED | DROP (Rule 1a: incorrect) |
| F17 | Exploratory user-log incident was hidden | INCORRECT | — | — | — | HIGH | DROP (Rule 1a: incorrect) |

FIX: 10   FIX (with care): 0   SPIN-OFF: 0   DISCUSS: 0   DROP: 7

All confirmed findings were applied as issue-local evidence-harness corrections. No issue command is staged; no product defect survived assessment.
