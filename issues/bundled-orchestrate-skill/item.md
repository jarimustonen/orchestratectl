---
created: 2026-08-13
updated: 2026-08-13
type: bug
status: open
priority: normal
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
