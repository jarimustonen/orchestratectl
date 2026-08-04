---
created: 2026-07-25
updated: 2026-08-04
type: feature
reporter: jari
status: fixed
priority: normal
closed: 2026-08-04
---

# run cancel should accept an unambiguous run-id prefix (like run show), not require full 26-char ULID

_Source: run cancel arg parsing_

## Description

OBSERVED (2026-07-25): `orchestratectl run cancel 01kybtpczp1c7bpbmf` (a truncated/typo'd run-id) failed with error.code=invalid_run_id 'is not a valid ULID: expected 26-char lowercase Crockford base32 ULID'. I had to look up the full 26-char ULID and re-run.

Minor UX friction, not a defect. When triaging in-flight runs from `run list` output, one often has a prefix, not the full ULID. `run show` (and git) accept an unambiguous prefix; `run cancel` requiring the exact 26 chars is inconsistent.

REQUEST: `run cancel` (and ideally all run-id-taking subcommands) should resolve an unambiguous prefix to the full run-id, erroring only on ambiguity or no-match — matching `run show` behaviour. Keeps the CLI consistent and reduces copy-paste of full ULIDs.
