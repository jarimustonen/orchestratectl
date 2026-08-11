---
created: 2026-08-05
updated: 2026-08-11
type: improvement
reporter: jari
status: in-progress
priority: normal
---

# doctor: detect orphan companion files (installed by a prior binary, no longer bundled)

_Source: crates/octl-cli/src/{doctor/checks/skill.rs,skill.rs}_

## Description

Follow-up spun off from doctor-skill-companion-sync review (Gemini/GPT/Opus consensus). The new skill.sync.<name>.<file> check only audits the FORWARD direction: for each companion the CURRENT binary bundles, it verifies presence + content-sync. It does NOT detect an ORPHAN companion — a file a prior binary installed as a sibling of SKILL.md (e.g. an old AGENTS-*.md) that this binary no longer ships. Such a file lingers on disk forever: the doctor loop iterates only companion_sources(name), and cmd_install's prune removes whole de-registered skill DIRECTORIES, not stray files inside a still-registered skill's directory. Symmetric with skill.orphan.<name> but at file granularity. Doing it correctly (not naive read_dir + warn, which false-positives on a user's own note.md dropped into the managed dir, and whose only fix would be a prune that does not yet exist) needs: (1) record the exact managed file set in the provenance marker or a sidecar manifest at install time; (2) on doctor, warn only on managed files no longer bundled -> skill.orphan.<name>.<file>; (3) teach cmd_install --force to prune those managed-but-de-registered files so the WARN has a working fix. Touches the hot skill-install path, so sequence it.
