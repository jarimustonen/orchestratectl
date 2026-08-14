//! Harness-specific **worker-prompt preamble** (the pi translation shim).
//!
//! Background: a worker's prompt is the `--task` brief the orchestrator authored
//! following a bundled SKILL, materialized verbatim to `<run-dir>/prompt.md` and
//! handed to the agent (`create.sh` → `workmux add -P`). Those briefs are
//! Claude-Code-flavored: they lean on the Skill tool, sub-agents / the Agent tool,
//! MCP, and `/worktree-*` / `/llm-*` slash commands — none of which the
//! `pi` agent has (pi is AGENTS.md-native, the Agent-Skills standard).
//! Most of the orchestration is already external — the closing `orchestratectl run
//! merge` is a plain CLI call — so the gap is narrow: the worker just needs the
//! Claude-only references mapped to their bash/CLI equivalent so it can complete
//! the loop (work → `run merge` → report).
//!
//! This module produces a short operating-note **preamble** that `run create`
//! prepends to the worker's prompt when the resolved harness needs the
//! translation. It is deliberately scoped to **ONE autonomous kind** —
//! [`Kind::Research`] under the `pi` harness — matching the issue's done bar (one
//! kind working end-to-end). Every other `(harness, kind)` pair returns `None`, so
//! the claude path is byte-identical to before and un-shimmed pi kinds are left
//! exactly as they were (an explicit, honest out-of-scope boundary rather than a
//! half-translated brief). Extending the shim to another kind is a one-arm change
//! in [`worker_prompt_preamble`].
//!
//! Unlike the static bundled SKILLs (which cannot know the run id), this preamble
//! is generated in-process *by* `run create`, which already holds the exact run
//! id. It is templated in directly (`{RUN_ID}` sentinel), so the pi worker's
//! closing call is a literal `orchestratectl run merge <run-id>` with **no**
//! `ls ~/.orchestratectl/runs | grep` discovery to get wrong — the single biggest
//! reliability win over hand-copying the SKILL's discovery snippet (llm-review
//! consensus).

use octl_core::Kind;

/// Sentinel replaced with the concrete run id when the preamble is rendered.
const RUN_ID_SENTINEL: &str = "{RUN_ID}";

/// The pi research-worker operating-note template. Rendered by
/// [`worker_prompt_preamble`], which substitutes [`RUN_ID_SENTINEL`] with the run
/// id. Prepended before the orchestrator's research brief so a pi worker (no
/// Skill/Agent tools, no Claude slash commands) can still complete the
/// merge-and-report loop.
///
/// The closing uses a **quoted** heredoc (`<<'JSON'`) so a summary containing `$`,
/// backticks, or backslashes is written literally instead of being shell-expanded,
/// and it templates the exact run id so there is no run-id discovery step to fail.
const PI_RESEARCH_PREAMBLE_TEMPLATE: &str = r#"# Operating note — pi research worker (read first)

You are an autonomous **research** worker launched by `orchestratectl` inside a
dedicated git worktree. Your deliverable is a **sourced markdown report** committed
to this repo (typically `research/<slug>.md`). Work directly with the shell, your
editor, and your web tools.

You do **not** have Claude Code's Skill tool, sub-agent / Agent tool, MCP
connectors, or any `/worktree-*` / `/llm-*` slash commands. The task brief below
was written for a Claude worker and may reference them. Translate, and never try to
invoke them:

- `/worktree-merge`, `/complex-rebase`, "merge yourself back", "self-merge" → run
  the **Closing** steps at the end of this note. The `orchestratectl run merge`
  call there is the entire merge-and-report step.
- `/llm-review`, `/assess-findings`, "spawn a sub-agent", "use the Skill/Agent
  tool" → you have no such tool; skip it and either do the equivalent yourself
  in-line or record it as a follow-up in the report's `spinoff_proposals`.
- Any other `/name` slash command → it is Claude-Code-only. Ignore it and use the
  plain CLI/bash equivalent.

## Closing (mandatory — this is how the run ends)

Your run id is `{RUN_ID}` — it is already filled into the commands below. Do not
try to rediscover it.

Once the report file is written **and committed**, close the loop:

1. Write the terminal report. **Replace `<one-line outcome>` with a real one-line
   summary of your findings**, and populate the arrays if you have follow-ups
   (leave them `[]` otherwise). Run exactly:

   ```bash
   cat > /tmp/node-report-{RUN_ID}.json <<'JSON'
   {
     "success": true,
     "summary": "<one-line outcome>",
     "discussion_items": [],
     "spinoff_proposals": [],
     "wrap_up_recommendations": []
   }
   JSON
   ```

2. Merge and report in one call:

   ```bash
   orchestratectl run merge {RUN_ID} --report-file /tmp/node-report-{RUN_ID}.json
   ```

`orchestratectl run merge` rebases + merges this worktree's branch into its source
branch and submits the terminal report in the same call; the supervisor then tears
down the worktree, tmux window, and branch. Do **not** run `git worktree remove`,
`git branch -d`, or `tmux kill-window` yourself.

On a merge conflict the call exits non-zero with `error.code: "merge_failed"` and
submits **no** report. Resolve the conflict, commit, then re-run **only** the
`orchestratectl run merge` command from step 2 — the report file from step 1 is
already on disk, so do not recreate it.

---

The original task brief follows.
"#;

/// The worker-prompt preamble for a resolved harness + run kind, or `None` when no
/// translation is needed.
///
/// `None` for the default `claude` harness (the prompt is passed through
/// byte-for-byte) and for every pi kind except [`Kind::Research`] — the single
/// kind this shim covers end-to-end. A `Some` value is prepended to the worker's
/// prompt by `run create` before it is materialized to `prompt.md`; `run_id` is
/// the exact run id (already known to `run create`) templated into the closing
/// call so the pi worker needs no run-id discovery.
#[must_use]
pub fn worker_prompt_preamble(harness: &str, kind: Kind, run_id: &str) -> Option<String> {
    match (harness, kind) {
        ("pi", Kind::Research) => {
            Some(PI_RESEARCH_PREAMBLE_TEMPLATE.replace(RUN_ID_SENTINEL, run_id))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUN_ID: &str = "01JXAAAA0000BBBBCCCCDDDDEE";

    #[test]
    fn claude_never_gets_a_preamble() {
        for kind in [Kind::Research, Kind::Spinoff, Kind::Code] {
            assert!(
                worker_prompt_preamble("claude", kind, RUN_ID).is_none(),
                "claude must pass the prompt through unchanged for {kind:?}"
            );
        }
    }

    #[test]
    fn pi_research_gets_the_shim() {
        let p =
            worker_prompt_preamble("pi", Kind::Research, RUN_ID).expect("pi research shim present");
        // Establishes the AGENTS.md-native operating context.
        assert!(p.contains("pi research worker"));
        // Maps the Claude-only merge slash command to the bash closing.
        assert!(p.contains("/worktree-merge"));
        // Carries the self-contained closing call so the worker never depends on
        // the brief phrasing the close as a slash command.
        assert!(p.contains("orchestratectl run merge"));
        // Neutralizes the review / sub-agent references pi cannot honor.
        assert!(p.contains("/llm-review"));
        // Ends by handing off to the original brief (which is appended after it).
        assert!(p.trim_end().ends_with("The original task brief follows."));
    }

    #[test]
    fn pi_research_templates_the_exact_run_id_and_leaves_no_sentinel() {
        let p = worker_prompt_preamble("pi", Kind::Research, RUN_ID).unwrap();
        // The concrete run id is substituted everywhere: the report path and the
        // `run merge` positional both carry it, so there is no discovery step.
        assert!(p.contains(&format!("orchestratectl run merge {RUN_ID} --report-file")));
        assert!(p.contains(&format!("/tmp/node-report-{RUN_ID}.json")));
        // No leftover sentinel, and — critically — no fragile `ls | grep` run-id
        // discovery (the llm-review consensus blocker).
        assert!(
            !p.contains(RUN_ID_SENTINEL),
            "sentinel must be fully substituted"
        );
        assert!(
            !p.contains("~/.orchestratectl/runs/"),
            "must not fall back to ls/grep run-id discovery"
        );
    }

    #[test]
    fn pi_research_closing_uses_a_quoted_heredoc() {
        // A quoted heredoc delimiter keeps the shell from expanding `$`/backticks
        // inside a model-authored summary — an unquoted `<<JSON` would corrupt the
        // report or run injected commands.
        let p = worker_prompt_preamble("pi", Kind::Research, RUN_ID).unwrap();
        assert!(p.contains("<<'JSON'"), "heredoc delimiter must be quoted");
        assert!(
            !p.contains("<<JSON\n"),
            "an unquoted heredoc must not appear"
        );
    }

    #[test]
    fn pi_non_research_kinds_are_out_of_scope() {
        // The shim is deliberately narrow: only research is translated end-to-end.
        // Other pi kinds return None (documented out-of-scope) rather than a
        // half-applied translation.
        for kind in [Kind::Spinoff, Kind::Code] {
            assert!(
                worker_prompt_preamble("pi", kind, RUN_ID).is_none(),
                "only research is in scope for the pi shim, not {kind:?}"
            );
        }
    }

    #[test]
    fn unknown_harness_gets_no_preamble() {
        // A harness this build does not special-case is passed through unchanged.
        assert!(worker_prompt_preamble("aider", Kind::Research, RUN_ID).is_none());
    }
}
