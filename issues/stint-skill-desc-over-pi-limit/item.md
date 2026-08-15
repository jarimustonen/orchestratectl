---
created: 2026-08-15
updated: 2026-08-16
type: bug
status: in-progress
priority: normal
labels: [skill]
commits:
- hash: ec9da0a
  summary: trim stint SKILL descriptions under pi.dev 1024-char limit + guard test
---

# Bundled stint-start/stint-handoff SKILL descriptions exceed pi.dev 1024-char limit

## Description

## Description

pi.dev's harness warns on skill load that two bundled skill descriptions exceed pi.dev's 1024-char `description:` limit:

```
auto (user) ~/.pi/agent/skills/stint-handoff/SKILL.md  — description exceeds 1024 characters (1141)
auto (user) ~/.pi/agent/skills/stint-start/SKILL.md    — description exceeds 1024 characters (1210)
```

These are the ONLY two bundled skills over the limit (scan 2026-08-15: all others 256–703). Same defect class as the obsoleted `bundled-orchestrate-skill`.

## Fix

Trim the `description:` frontmatter in `crates/octl-cli/skills/stint-start/SKILL.template.md` (1211) and `crates/octl-cli/skills/stint-handoff/SKILL.template.md` (1142) to <=1024 chars EACH. This is a careful trim, NOT blind truncation — the description drives skill selection, so keep the trigger phrases (Finnish + English invocation cues: "aloitetaan rupeama", "jatketaan @TODO.md", "päätetään rupeama", "wrap up the stint", bare-invocation notes) and the NOT-this-skill disambiguators. Drop redundant prose.

## Done

- Both descriptions <=1024 chars.
- Redeploy: `cargo install --path crates/octl-cli --force && orchestratectl skill install --force && orchestratectl doctor` (expect 0/0), and confirm pi.dev no longer warns on load.
- Consider a doctor/CI guard that flags any bundled description >1024 so this can't regress.

Lane D (bundled skill prose).
