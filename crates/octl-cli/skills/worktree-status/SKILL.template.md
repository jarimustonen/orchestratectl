---
name: worktree-status
description: Summarize the current state of an active worktree session in plain product language, for a non-technical decision-maker (product owner, manager, stakeholder) who was not in the room. Use when asked to give a status update on a worktree, summarize where the worktree stands, brief the PO on what's happened so far, or run bare `/worktree-status`. Reads ONLY the current conversation context — does not run git, read files, or modify anything. NOT for PR descriptions, developer-to-developer context handoff (use `/wrap-up`), commit summaries, or daily-activity logs.
version: 1
cli_version: "{{CLI_VERSION}}"
schema_version: 1
---

# Worktree Status — Plain-Language Snapshot

Produce a status snapshot of the current worktree session for a non-technical decision-maker. **Reply inline in the chat — do not write to a file.**

Source of truth is the **current conversation context**: the operator's prompts, the work you've already done, the decisions surfaced, the open questions — **including any round results a conductor (e.g. `/stint`) has surfaced into the conversation**: which units landed, what closed, and what worker worktrees reported back (bug-analysis notes, ready-to-test facts, spin-off recommendations). When such round results are present, **incorporate them** — for a delegated round they are usually where "Ready to test" and "Spin-offs" come from, since the work happened out-of-band in other worktrees. Do not independently run git, read additional files, or collect new sources yourself; but do use everything the caller has already put in front of you.

This skill produces one chat message and stops. It does not modify anything.

**Arguments:** $ARGUMENTS

## Output structure

Each item: **100–200 words** of plain prose (the testing steps in "Ready to test" may be a short numbered list instead). If a section has zero items, **omit the header entirely** — do not write "(none)". The minimal valid output is the Summary alone.

"Ready to test" is the section that turns this snapshot into something the reader can act on: it lists what got built this session and hands them the steps to try it. Include it whenever concrete, user-observable behaviour changed. Omit it only when nothing testable landed yet (pure research, planning, or a session that made no user-facing change).

```markdown
# <product-language title — what's being worked on, not a branch name>

## Summary
<one paragraph, 100–200 words: what this session set out to do in product terms, and where it currently stands. Lead with the *why*. Frame as "what changes for the user/product when this lands".>

## Ready to test
### <short product-language title of the thing that got built>
<Write for a colleague who already knows the product — do NOT spell out basic navigation or steps they'd do in their sleep. Focus on **what changed**: the new or altered behaviour, and what to look at to confirm it works.
- A short lead (1–2 sentences): what now works that didn't before, framed as what the user/product gets.
- Then just enough to point them at the change: the specific feature/screen/flow that's new, the thing to try, and what they should see if it's right (the pass/fail signal). Skip anything obvious to someone who uses the product; only walk through a step if it's genuinely new or non-obvious.
- Call out what only they can judge: data only they have, a device or account only they can reach, or a "does this feel right?" call. If a piece was built but deliberately left unverified, say so and why.
Keep each item focused on one changed thing; give it its own subsection.>

## Decisions needed
### <short product-language title>
<100–200 words: what question is open, why a non-technical reader needs to weigh in, what the realistic options are. Skip questions that are purely mechanical or that the team can answer itself.>

## Discussion points
### <short product-language title>
<100–200 words: things that surfaced during the work but were not resolved — open tensions, observations, things to revisit later. Distinct from "Decisions needed" in that no specific call has to be made now; this is context the PO should be aware of. Each item: what came up, why it matters.>

## Spin-offs
### <short product-language title>
<100–200 words: work deliberately split out — new follow-up tracks, "we should also do X" items, deferred pieces. Each: what got deferred, *why not now*, what the next step is.>
```

The top-level title must describe **what is being built or changed**, in product terms — not the branch name, not a slug. Example: "Two-factor login for staff accounts" — not `auth-2fa-spike`.

Omit any section that has zero items. A typical session that shipped something has a Summary plus "Ready to test"; a pure research or planning session may have only a Summary.

## Language discipline

Audience does NOT know git, branches, skills, worktrees, code review, or this codebase. They DO understand "the product", "the feature", "users", "the team".

Banned in the output: `worktree`, `branch`, `commit`, `merge`, `PR`, `rebase`, `skill`, `SKILL.md`, slash-commands, file paths, commit hashes, branch names. Translate or rephrase. "We did the work in parallel" — not "we spun up a worktree". "Follow-up work" — not "spin-off".

Self-check before sending: "Would a non-technical person know what's happening and why it matters?" If no, rewrite the section.

## Non-goals

- Does NOT write to a file. Output is the chat reply only.
- Does NOT run git, read repo files, or open issues. The current context is the entire source.
- Does NOT decide whether to merge — that's `/worktree-merge`.
- Does NOT replace `/wrap-up` (operator-facing session save) or a PR description.
- Does NOT modify any state.
