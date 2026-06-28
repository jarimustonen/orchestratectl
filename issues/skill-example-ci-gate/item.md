---
created: 2026-06-28
updated: 2026-06-28
type: feature
status: fixed
priority: normal
closed: 2026-06-28
---

# CI gate: validate every orchestratectl invocation in SKILLs against the binary

## Description

Feature: add a CI / pre-commit gate that catches skill ↔ binary drift before it ships. Run every `orchestratectl ...` invocation in every bundled SKILL.template.md against the binary (at minimum `--help`-level, ideally `--dry-run`) and fail the build if any flag/kind/positional is unknown.

Why: agents trust SKILLs because the version-check says "same version, proceed normally". When the SKILL drifts from the binary (as documented in issue `skill-binary-doc-sync`), the agent hits `unknown_subcommand_or_flag` mid-workflow and has no escape — the SKILL is supposed to be the operating manual. A CI gate that mechanically validates every example would have caught the drift items #1, #3, #4, #5, #6 reported by the deutschpad-v2 agent on 2026-06-28.

Implementation sketch:

1. New test target: `crates/octl-cli/tests/skill_examples.rs`.
2. For each SKILL.template.md in `crates/octl-cli/skills/<name>/`:
   - Extract every fenced code block tagged ``` or no-tag that contains an `orchestratectl ` line at column 0 (heuristic — refine as needed).
   - Parse the command into argv. Substitute `<run-id>`, `<node-id>`, `<branch>` etc. placeholder values with valid synthetic ones (a fixture).
   - Run with `--dry-run` if the subcommand supports it; otherwise run with `--help` and assert the flags exist in the help output.
   - Fail the test if any invocation returns `unknown_subcommand_or_flag`, `invalid_value`, or any other shape-of-CLI error.
3. Allow-list mechanism: some SKILL examples are intentionally illustrative (e.g. show output formats, not actual invocations). Mark these with a magic comment like `# skill-example-ci: skip` inside the fence so the test skips them.
4. Gate is part of `cargo test`, runs on every PR. Pre-commit hook optional.

Detection of envelope-shape drift (issue `skill-binary-doc-sync` #8, #9, #10) is harder — it requires parsing the example JSON in the SKILL, running the actual command, and structurally diffing. v2 of this gate. v1 catches the flag/kind/positional errors.

Acceptance:
- New test passes on a clean tree (after `skill-binary-doc-sync` is fixed).
- Test fails if any SKILL is edited to use a non-existent flag — e.g. revert one of the `skill-binary-doc-sync` fixes and confirm the test catches it.
- Test documentation explains how to add an allow-list comment to genuinely illustrative examples.

Severity: MEDIUM-HIGH proactive. Without it, every future SKILL edit risks the same class of drift, which the agent caller cannot recover from.

Related:
- `skill-binary-doc-sync` — the immediate one-time cleanup; this issue is the long-term prevention.
- `spinoff-e2e-harness` — runtime verification; this issue is build-time verification.
