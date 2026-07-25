---
created: 2026-07-25
updated: 2026-07-25
type: bug
status: fixed
priority: normal
closed: 2026-07-25
---

# harness bakeoff: aider + pi adapters fail on a live run (aider leaves changes uncommitted; pi exits 1); needed only for a full 4-way loop comparison

## Description


## Resolution (2026-07-25) — both adapters fixed and live-verified

Live-verified against real binaries (aider 0.86.2, pi 0.82.0) with a DeepSeek
backend via `orchestratectl harness bakeoff --only aider --only pi`. Both now
report **committed**.

### pi — exit-1 root cause: unsupported `--` terminator
The adapter appended `--` as an option terminator, but pi's parser rejects it
(`Error: Unknown option: --`) and exits non-zero. Fix: drop `--`, pass the prompt
as the sole trailing positional (`crates/octl-cli/src/harness/pi.rs`). Also added a
leading-space guard for a dash-leading brief (pi has no `--` escape). Live-verified:
pi reaches the model and produces a committed result whose self-check passes. Note
pi exits 0 even on an auth error, so `--` was the *only* cause of the non-zero exit.

### aider — leaves changes uncommitted
Reproduced directly: aider exits 0 but does NOT auto-commit — it leaves the
deliverable + `.gitignore` untracked (dirty tree), which the shared skeleton mapped
to Failed. Fix: internal `AgentLaunch::commits_in_agent()` hook (default true, so the
Claude family + pi are unchanged); aider returns false and `support::run_chunk`
commits aider's leftover edits after a clean exit. The staging excludes `.aider*`
scratch files; the commit is deterministic (explicit identity, `--no-verify`,
`--no-gpg-sign`). Live-verified: aider produces a committed, gate-able result.

### Quality gate
`/llm-review` (4 models) → `/assess-findings`: 4 confirmed findings applied
(pi dash-guard, `.aider*` exclusion, deterministic commit, auditable message);
8 dropped (verified incorrect or deliberate design tradeoff — see
`history/assessment-bakeoff-aider-pi-adapters.md`). Green: fmt, clippy, full
workspace test suite (74 harness tests incl. 6 new).

### Remaining (non-blocking)
- pi usage parsing does not match pi 0.82's real `--mode json` shape
  (`usage.{input,output,totalTokens}`, `usage.cost.total`), so the bakeoff cost
  column shows `-` for pi. Usage is best-effort provenance (design §10), not a gate,
  and does not affect run completion — a small follow-up, not part of this fix.
- A brief that literally begins with `-` still can't be passed to pi verbatim (it is
  space-guarded, which is invisible to the model). Inherent pi CLI limitation.

The full 4-way loop comparison (claude + claude-deepseek + aider + pi) is now
achievable. (aider's *self-check* can still fail on a given brief when its model's
edit doesn't satisfy the check — that is model-output quality, a valid comparison
datapoint, not an adapter defect.)
