---
created: 2026-08-04
updated: 2026-08-17
type: bug
status: fixed
priority: normal
lane: skills
lane_seq: 20
commits:
- hash: a06434d738e6bc911aa57fded2bb0bf7d257fbe2
  summary: 'fix(skill): replace dangling symlinks with force'
closed: 2026-08-17
---

# skill install --force aborts on a pre-existing symlink (refused_overwrite)

## Description

## Symptom

`taskfleet skill install --force` aborts the WHOLE install with `refused_overwrite` when any target path already exists as a **symlink**, even though `--force` was passed:

```
{"error":{"code":"refused_overwrite","message":"/Users/jari/.claude/skills/worktree/SKILL.md already exists; pass --force to overwrite","invalid_value":".../worktree/SKILL.md"}}
```

The message says "pass --force to overwrite" — but `--force` *was* passed. It looks like the force path only replaces regular files, not symlinks (and the whole batch aborts on the first offender rather than continuing).

## Repro

1. Have `~/.claude/skills/worktree/SKILL.md` be a symlink (e.g. a stale dotfiles link whose target was removed — a **dangling** symlink triggers it too).
2. `taskfleet skill install --force`
3. Aborts with `refused_overwrite`; no skills installed.

## Expected

`--force` should overwrite an existing **symlink** target (at minimum a dangling one — a broken symlink is never a file worth preserving), the same way it overwrites a regular file. Ideally the batch also continues past a single un-forceable entry instead of aborting the whole install.

## Context / real-world hit

Hit during the taskfleet→Homebrew-tap migration (issue homebrew-tap-distribution): machines still carrying the old dotfiles-linked worktree skills had dangling `~/.claude/skills/<name>/SKILL.md` symlinks (sources removed when those skills became binary-owned). Worked around in the homebase setup hook by pruning broken symlinks in the owned skill dirs BEFORE calling `skill install --force`. That workaround shouldn't be necessary.

## Side note

The `version` subcommand emits JSON by default and *rejects* `--json` (`unexpected argument '--json'`), unlike `skill install` and unlike ossctl's `version --json`. Minor inconsistency, but it bit a downstream script that assumed the `--json` convention. Worth aligning.
