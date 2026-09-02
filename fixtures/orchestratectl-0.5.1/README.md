# Sanitized orchestratectl 0.5.1 compatibility home

This commit-safe fixture freezes the state/config/skill shapes that Taskfleet must read during the bounded migration in ADR 0002. It was captured on 2026-09-02 with the installed published `orchestratectl 0.5.1` binary (`commit f0c52ab232706fb480a51bfd45f2171c6b7aa056`) in disposable `HOME` and `ORCHESTRATECTL_HOME` directories. No user installation, installed skill, pi configuration, or real `~/.orchestratectl` state was read or changed.

## Contents

| Run | Shape | Required compatibility |
|---|---|---|
| `01j…001` | terminal `done`, successful agent report | Adopt/read without changing event bytes or terminal truth. |
| `01j…002` | non-terminal running node | Detect as active; do not move its root. |
| `01j…003` | running node with `pending_merge` | Preserve transaction OIDs, op id, branch and recovery semantics. |
| `01j…004` | historical removed kind `code` | Deserialize as the read-only `unknown` kind; never rewrite it as a current kind. |

`home/orchestratectl/config.toml` includes the 0.5.1 harness/profile surface and an unknown top-level section which the forward-compatible loader accepts. `repo/.orchestratectl.toml` freezes repository profile selection. `state/pi-installed-skills.json` plus the disposable Claude, Codex and pi copies freeze installed-skill provenance and managed-marker formats for one branded skill.

## Capture and sanitization

- `run create --skip-materialize` (the published binary's test-only path) created skeletons.
- Public `event create` and `node report` calls produced completed/non-terminal projections.
- A disposable git repository/worktree and an `OCTL_MERGE_SH` fault stub killed `run merge` after its durable `merge.started` append, yielding a real 0.5.1 pending transaction.
- `skill install orchestratectl-overview --agent all --force` ran only with disposable homes.
- Run ids, timestamps, paths, PIDs, operation ids and git OIDs were replaced with deterministic non-secret values. JSON key ordering/formatting may differ from the original atomic writer, but event ordering, fields, schema versions and semantic values are preserved. Skill bytes and their recorded SHA-256 values were not altered.

These are byte/readability fixtures, not proof of live-process quiescence or successful Git recovery. R3/R5/R8 must create runtime held-lock, old-process, real-Git recovery and edited/unmanaged-skill scenarios in disposable environments. Additional 0.5.1 branded skill copies remain reproducible with `skill install <name> --agent all --force` under disposable `HOME` and `ORCHESTRATECTL_HOME`; repository work must never run that command against real homes.

The fixtures are evidence, not writable test homes. Tests must copy `home/` and `repo/` to a temporary directory first. Run `./fixtures/orchestratectl-0.5.1/verify.sh [binary]` for the exact published baseline (the default binary is `orchestratectl`). `verify.sh --compat <new-binary>` skips only the exact version/commit assertion so a later Taskfleet reader can exercise the same protected corpus. The verifier enforces `SHA256SUMS`, permits runtime output only under the copied `logs/` subtree, and requires every other fixture byte to remain unchanged.
