---
created: 2026-08-13
updated: 2026-08-13
type: bug
reporter: jari
status: open
priority: high
---

# Support pi.dev skill installs with companions

## Description

## Problem

`stint-start` references a companion file named `AGENTS-EXECUTION-DAG.md` and instructs agents to stop if it is missing. In pi.dev, the skill was installed at:

```text
/Users/jari/.pi/agent/skills/stint-start/SKILL.md
```

but the companion file was absent:

```text
/Users/jari/.pi/agent/skills/stint-start/AGENTS-EXECUTION-DAG.md  # missing
```

This caused `/stint-start` to abort before Phase 0 with an ENOENT when the agent tried to read the referenced file.

## Evidence

References in the pi-installed skills:

```text
/Users/jari/.pi/agent/skills/stint-start/SKILL.md:32
/Users/jari/.pi/agent/skills/stint-start/SKILL.md:63
/Users/jari/.pi/agent/skills/stint-start/SKILL.md:122
/Users/jari/.pi/agent/skills/stint-start/SKILL.md:133
/Users/jari/.pi/agent/skills/stint-start/SKILL.md:236
/Users/jari/.pi/agent/skills/stint-handoff/SKILL.md:23
/Users/jari/.pi/agent/skills/stint-handoff/SKILL.md:62
```

The companion exists in the Claude install:

```text
/Users/jari/.claude/skills/stint-start/AGENTS-EXECUTION-DAG.md
```

but not in the pi install:

```text
/Users/jari/.pi/agent/skills/stint-start/
└── SKILL.md
```

`orchestratectl doctor` reported the companion sync as OK because it appears to validate the Claude install path, not the pi.dev skill path.

## Expected behavior

`orchestratectl skill install` and `orchestratectl doctor` should support pi.dev as a first-class agent runtime.

At minimum:

- `skill install --agent pi` should install bundled skills into `~/.pi/agent/skills/` using pi's directory skill layout.
- Companion files such as `stint-start/AGENTS-EXECUTION-DAG.md` should be installed beside `SKILL.md`, preserving the relative links used by the skill.
- `doctor` should be able to validate pi-installed skills and companion files, or at least not report an all-clear for an environment where pi's active skill install is incomplete.
- `--agent all` should include pi.dev if pi support is available on the machine.

## Reproduction

On a machine with pi.dev skills under `~/.pi/agent/skills/`:

1. Ensure `stint-start/SKILL.md` is present under `~/.pi/agent/skills/` but `stint-start/AGENTS-EXECUTION-DAG.md` is absent.
2. Invoke `/stint-start` in pi.dev.
3. The skill instructs the agent to read `AGENTS-EXECUTION-DAG.md` relative to the skill directory.
4. The read fails with ENOENT and the skill cannot continue.

## Workaround

A manual workaround exists:

```bash
orchestratectl skill install stint-start \
  --dest /Users/jari/.pi/agent/skills/stint-start/SKILL.md \
  --force
```

Testing this against a temp directory showed that `--dest` does copy the companion next to `SKILL.md`, so the missing piece is an explicit pi.dev agent target plus doctor coverage.
