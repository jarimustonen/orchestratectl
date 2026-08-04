---
created: 2026-08-04
updated: 2026-08-04
type: bug
status: open
priority: normal
---

# `octl-spawn-spinoff` SKILL.md still carries a "NOT IMPLEMENTED" preview banner — but the surface ships in 0.1.0

_Source: issuectl stint 2026-08-04 — the spinoff/orchestrate surface was used successfully all session._

## Observed

`octl-spawn-spinoff`'s `SKILL.md` opens with a prominent blocking banner:

> ## ⚠️ PREVIEW — DO NOT INVOKE BLINDLY
> The `orchestratectl run create --kind spinoff` surface documented here is
> **not yet implemented**. It lands in the `all-kinds-spawn` issue. Until then:
> 1. Call `orchestratectl --help` and confirm the `run` subcommand is listed…
> 2. …invoke the `/worktree-spinoff` slash-command skill instead…
> 3. Otherwise … tell the user the spinoff surface is not yet shipped and stop.

But in the installed **`orchestratectl 0.1.0`** the surface **is** implemented:

    $ orchestratectl run create --kind spinoff --headless --title … --task … --source-branch main
    {"schema_version":1,"data":{"run_id":"…","kind":"spinoff", …}}

`run create --kind` accepts `spinoff`, `orchestrate`, `orchestrated`, `research`,
`technical-decision`, `make-skill`, `fan-out`, `bugfix`, `code` — all worked
(spinoff and orchestrate both used this session, landing real merged commits).

## Impact

The stale banner tells an agent to distrust the documented invocation and fall
back to `/worktree-spinoff`, or to "stop" — friction on the now-shipped happy
path. The whole point of the skill (drive `orchestratectl` directly) is
undercut by its own warning.

## Suggested fix

Remove the PREVIEW banner (and the three-step fallback gate) now that
`run create --kind spinoff` ships in 0.1.0. If a version floor is still wanted,
replace the banner with a normal "requires orchestratectl ≥ 0.1.0" note rather
than a "not implemented" stop-gate. Check the sibling skills
(`orchestratectl-overview`, `octl-run-overview`, the `worktree-*` family) for the
same stale "not yet implemented" language and the `all-kinds-spawn` reference.

## Severity

Low — documentation/agent-guidance, but actively misleading since the feature
shipped.
