---
created: 2026-08-13
updated: 2026-08-14
type: bug
status: wontfix
priority: normal
closed: 2026-08-14
---

# Bundled orchestrate skill description exceeds pi limit

## Description

## Problem

pi reports a skill validation warning for the installed orchestrate skill:

```text
auto (user) ~/.pi/agent/skills/orchestrate/SKILL.md
  description exceeds 1024 characters (1063)
```

The installed `~/.pi/agent/skills/orchestrate/SKILL.md` and `~/.claude/skills/orchestrate/SKILL.md` contain a frontmatter `description:` that is 1063 characters long. pi validates Agent Skills descriptions with a 1024-character maximum.

## Expected

`orchestratectl skill install` should install a pi-compatible skill with frontmatter description <= 1024 characters.

## Comments

This appears to originate from orchestratectl's bundled skill template rather than homebase symlinked skills. Homebase-local YAML errors were fixed separately.

## Resolution

### 2026-08-14T13:46:07Z · @issuectl

Obsoleted by cut-run-kinds-discussion-machinery: the /orchestrate bundled skill was removed in the 0.2 subtractive cut, so its pi description-length concern no longer applies.
