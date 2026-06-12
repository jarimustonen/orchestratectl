# Handoff — items to discuss with Jari

Items surfaced by the multi-LLM scaffolding review that warrant a
judgment call rather than a unilateral fix. Each is non-blocking for
the scaffolding itself (closed as `done`) but should be settled before
too many subcommands ossify a choice.

## 1. Envelope `schema_version` constant location

Today the CLI envelope schema lives at `crates/octl-cli/src/error.rs::SCHEMA_VERSION`
and the on-disk state schema lives at `crates/octl-core/src/lib.rs::STATE_SCHEMA_VERSION`.
Both are `1`. Reviewers (anthropic, openai) argued the envelope contract
is shared by every tool that consumes orchestratectl output and
therefore belongs in `octl-core` (or a new `octl-proto` crate) so a
future companion binary, daemon, or skill installer can reuse it.

**Question:** Move `SCHEMA_VERSION` into `octl-core`, keep it in `octl-cli`,
or wait for the supervisor crate split (already on the design board as
post-MVP) and resolve then?

## 2. `--json` vs `--output text|json|jsonl` as the canonical flag

AGENTS-AI-FIRST-CLI §9 names `--output=text|json|jsonl` as the format
selector and §13 reuses `--output FILE.jsonl` as the large-output sink.
Today we ship only the boolean `--json`. Spin-off issue
`output-flag-and-streaming` will implement `--output`, but reviewers
disagreed on whether `--json` should remain as a shorthand alias or be
removed for one canonical name.

**Question:** keep `--json` as a permanent shorthand (matches `gh`,
`kubectl`'s -o flag), or deprecate it once `--output` ships? If
deprecating, declare the removal window now per §10.

## 3. Cross-platform `HOME` resolution

`log_path()` currently reads `HOME` directly. On Windows that env var is
unset by default (it's `USERPROFILE`). Gemini flagged this; deepseek and
anthropic did not. The `home` crate or `directories` crate would solve it.

**Question:** is Windows support a goal for MVP, or are we macOS+Linux
only until v2? If "later," leave as-is; if "yes," add the `home` crate
now before more callsites accumulate.

## 4. Workspace-wide lints policy

Reviewers suggested `[workspace.lints.clippy] pedantic = "warn"` and
`#![warn(missing_docs)]` on `octl-core/lib.rs`. This is real opinion
territory — pedantic lints catch real bugs but also annotate-heavy
patterns. Spin-off issue `ci-and-lints` will land lints, but the policy
(pedantic vs default+`-D warnings` vs custom subset) is your call.

**Question:** which lint level should `ci-and-lints` settle on?

## 5. Error envelope structured fields

Reviewers (openai, anthropic) want `details: Option<Value>` and
`hint: Option<String>` on `ErrorBody` for richer AI-actionable errors.
Today we ship `code`, `message`, `invalid_value`, `expected`. Adding
fields is additive (no schema bump) but `hint` in particular shapes how
subcommand authors think about failures — "did you remember to provide a
hint?" becomes a review question.

**Question:** add `hint` + `details` to `ErrorBody` now (cheap), or wait
until a real subcommand demonstrates the need?
