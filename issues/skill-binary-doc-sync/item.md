---
created: 2026-06-28
updated: 2026-06-28
type: bug
status: fixed
priority: normal
closed: 2026-06-28
commits:
- hash: 5eb862a
  summary: sync SKILL examples with binary CLI surface
---

# Bundled SKILLs document flag forms, terminology, and envelope shapes that don't match the binary

## Description

Symptom: multiple bundled SKILL.md files document `taskfleet` subcommands using flag forms / parameter names / envelope shapes that DO NOT match the actual binary's CLI surface or output. The skill-catalog version-check passes ("proceed normally" — same version 0.0.1 on both sides), but the SKILL's instructions fail at runtime.

Inventory, from an agent running /orchestrate on a real campaign in deutschpad-v2 (2026-06-28, binary commit 849d658, skills 0.0.1):

**Flag form errors:**
- `/orchestrate` §4/§6, "Following progress"; `worktree-orchestrated` "Following progress" use `event tail --run <id>` and `node list --run <id>`. Binary requires run-id as a **positional**: `event tail <RUN_ID> --follow`, `node list <RUN_ID>`. The skill flag form returns `unknown_subcommand_or_flag`.
- `taskfleet-overview` "verbs" section: `taskfleet run create --kind <kind> --prompt "..."`. The `--prompt` flag does not exist; the binary takes `--task <TASK>` or `--prompt-file <PATH>`.

**Terminology drift:**
- `/orchestrate` §5/§6 uses `event create`. Same skill's "Install or upgrade" section calls it `event append`. The binary's help text reads "Read (`tail`) or append (`create`)" — i.e. `create` is the append operation, but the word "append" appears in skill prose as if it were a verb. Pick one name (`create` is what the binary exposes) and remove the other everywhere.

**Envelope-shape errors:**
- `worktree-orchestrated` §3 documents child supervisor as `"supervisor": {"note": "child supervisor spawned by parent"}` (object). Reality is `"supervisor": "delegated-to-parent-supervisor"` (plain string). Type mismatch — breaks any parser keying on `.supervisor.note`.
- Same skill §3 examples: `tmux_window: "🎼 wt/<title>"`, `branch: "wt/<title>"`, `worktree_path: ".../worktrees/<title>"`. Reality: `tmux_window: "🇩🇪 🎼 wt-01kw7gc5t1-f-schema-v2"` (repo-prefix emoji + `wt-<short-runid>-<title>`), `branch: "wt/<short-runid>-<title>"`, `worktree_path: "<repo>__worktrees/wt-<short-runid>-<title>"`. The skill's own run-id-extraction snippet (`sed -E 's#^wt/([0-9a-z]{10}).*#\1#'`) is compatible with the actual format (so cleanup works), but the §3 example contradicts the snippet — confusing readers.

**Lifecycle enum mismatch:**
- `taskfleet-overview` says lifecycle is one of `pending | running | paused | completed | failed | cancelled`. Reality: `run show` for a `--kind orchestrated` child reports `lifecycle: "autonomous"`, `status: "pending"`. The string `"autonomous"` is not in the documented enum — the field actually carries the kind's lifecycle classification (autonomous vs interactive), not the run's progress state. The overview's "lifecycle is the only authoritative field" advice is misleading because that field is not actually the progress signal it implies.

Fix direction — three coordinated edits to the bundled SKILLs:

1. **Flag form audit**: grep every SKILL.template.md for `--run `, `--prompt `, and verify each `taskfleet ...` example against `--help`. Correct positional vs flag usage. Add a CI gate (see issue `skill-example-ci-gate` if filed) so future skill edits cannot drift again.

2. **Envelope shape sync**: update `worktree-orchestrated` §3 (and any other affected SKILLs) so the documented JSON shape matches the binary's actual output. Either fix the SKILL to match the binary, OR fix the binary to match the SKILL — pick per case but be deliberate. For the `supervisor` field: prefer making both sides agree on a single shape (likely keep the string for simplicity, drop the `{"note": "..."}` object).

3. **Lifecycle field semantics**: clarify in `taskfleet-overview` what `lifecycle` actually carries (classification: autonomous/interactive) vs what `status` carries (progress: pending/running/etc.). If both fields exist with separate semantics, document both. If the intent was that `lifecycle` IS the progress field, then `--kind orchestrated` should not report `autonomous` there — that's a CLI bug to fix instead.

Acceptance:
- Every `taskfleet ...` invocation in every SKILL.template.md runs cleanly when executed (at least `--help`-checkable; ideally `--dry-run`-checkable).
- Envelope examples in SKILLs match `run create --kind <X> --dry-run` output byte-for-byte (or are noted as illustrative with "shape may vary").
- `taskfleet-overview` correctly distinguishes lifecycle (classification) from status (progress).

Severity: HIGH for usability. Each individual gotcha is small, but together they make first-time agent usage frustrating — version-check says "fine, proceed" while the actual commands fail.
