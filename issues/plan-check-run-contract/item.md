---
created: 2026-07-22
updated: 2026-07-22
type: improvement
reporter: jari
status: open
priority: normal
epic: code-pipeline
---

# plan.json check.run execution contract is under-specified

_Source: issues/code-pipeline/plan-schema.md_

## Description

plan-schema.md leaves the check.run execution contract open ('a shell command run by the supervisor? a structured {cmd, cwd, expect_exit}?'). T2 (plan-json-v2-schema) modeled check.run as a plain non-empty String, applying repo-relative path-traversal validation only to files_touched (run is a command, not a path). Before T3 (deterministic floor) executes checks, the contract must be locked: shell string vs structured {cmd,cwd,expect_exit}, working directory, timeout, env, and success criterion (exit 0 vs parsed output). Filed per design principle 4 (governed evolution: gap -> reviewed proposal -> versioned schema) rather than inventing undocumented shape.
