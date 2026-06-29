---
created: 2026-06-29
updated: 2026-06-29
type: bug
status: fixed
priority: high
closed: 2026-06-29
commits:
- hash: 9a21aef
  summary: replaced by 'run wait' in SKILL templates
---

# SKILL templates: multi-run polling snippet breaks under zsh (no word-split)

## Description

Reported 2026-06-29 from a deutschpad multi-run orchestration session.
An agent followed the documented "Following progress" poll pattern in
`worktree-spinoff` SKILL.template.md (and siblings) and adapted it to
watch three runs:

```bash
ids="01kwa76tn9... 01kwa76x7a... 01kwa76zgb..."
for id in $ids; do ...; done        # <-- silently broken under zsh
```

The agent's background command ran under **zsh** (the login shell), and
zsh does NOT word-split unquoted parameter expansions. The loop iterated
**once** with the entire three-id string as a single value;
`orchestratectl run show "<three ids>"` returned empty; the poll
concluded "not all terminal" and slept forever (`sleep 45` loop). All
three runs had actually finished and merged — the poll just never
noticed.

## Severity

Medium. Caused the "stuck / not progressing" symptom on a real
orchestration session. The agent killed the poll manually and the
underlying work was unaffected, but the failure mode is invisible
("looks like work is still in progress") and easy to re-introduce.

## Root cause

Shell portability. zsh and bash split unquoted `$var` differently. The
SKILL template's poll snippet was written assuming bash semantics
(`for x in $space_separated`) and quietly breaks when an agent runs it
under zsh.

## Fix

In every `crates/octl-cli/skills/*/SKILL.template.md` that includes a
"Following progress" / "Completion polling" / multi-run loop, replace
the `for id in $ids` pattern with one of:

1. **Bash array** (preferred — readable, portable to both shells):
   ```bash
   ids=(01kwa... 01kwa... 01kwa...)
   for id in "${ids[@]}"; do ... ; done
   ```
2. **Explicit bash shell** for the polling block:
   `bash -c '...for id in $ids...'`
3. **zsh opt-in** at the top of the script: `setopt sh_word_split`.

## Likely supersession by `run-wait-subcommand`

If the proposed `orchestratectl run wait <ids...>` subcommand lands
(see issue `run-wait-subcommand`), the SKILL templates' multi-run
polling snippet becomes a single
`orchestratectl run wait <id1> <id2> <id3>` call — the whole bug class
disappears. Keep this issue open as the docs-only fast fix in case
`run wait` slips; close as superseded once `run wait` is wired into
the templates.

## Files to touch

`crates/octl-cli/skills/worktree-spinoff/SKILL.template.md` and any
sibling SKILL.template.md whose progress-polling section uses
`for id in $ids` (grep the skills/ tree). After editing, redeploy with
`orchestratectl skill install --force` and verify
`orchestratectl doctor`.

