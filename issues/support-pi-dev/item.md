---
created: 2026-08-13
updated: 2026-08-13
type: bug
reporter: jari
status: fixed
priority: high
closed: 2026-08-13
closed_by: agent-spinoff
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

`taskfleet doctor` reported the companion sync as OK because it appears to validate the Claude install path, not the pi.dev skill path.

## Expected behavior

`taskfleet skill install` and `taskfleet doctor` should support pi.dev as a first-class agent runtime.

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
taskfleet skill install stint-start \
  --dest /Users/jari/.pi/agent/skills/stint-start/SKILL.md \
  --force
```

Testing this against a temp directory showed that `--dest` does copy the companion next to `SKILL.md`, so the missing piece is an explicit pi.dev agent target plus doctor coverage.

## Resolution

### 2026-08-13T12:23:28Z · @agent-spinoff

pi.dev skill install now mirrors companion resources as siblings of the pi SKILL.md (byte-identical, per-skill dir like claude, no link rewrite), with out-of-band provenance tracking (companions map, schema v2), --force reconciliation of dropped companions, prune (companions-first, defer-on-failure), and doctor coverage (skill.sync/skill.orphan .pi.<file>). Verified: skill install --force installs the companion + doctor reports all skill.sync.* ok. Reviewed via /llm-review (4 models) + /assess-findings: 9 FIX applied, F11 spun off (pi-provenance-flat-file-model), 3 dropped.
