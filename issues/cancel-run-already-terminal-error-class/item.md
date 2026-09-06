---
created: 2026-08-04
updated: 2026-08-06
type: bug
status: fixed
priority: normal
closed: 2026-08-06
---

# run cancel: RunAlreadyTerminal classified as system (exit 2) not user (exit 1)

## Description


## Context

From `/llm-review` of `run-cancel-accept-unambiguous-prefix` (GPT-5.6-sol, DeepSeek v4).

`cancel.rs` maps `taskfleet_core::Error::RunAlreadyTerminal` to `CliError::system("run_already_terminal")` (exit 2). A deterministic domain refusal — asking to cancel an already-terminal run — is a **user** error, not a system fault. Exit-code class governs AI-caller retry behavior, so exit 2 can trigger spurious retries of a permanently-refused operation. Should be `CliError::user(...)` (exit 1).

Also: the `expected` hint is `json!("running|pending|blocked")` (a pipe-delimited string); prefer an array `json!(["running","pending","blocked"])` for machine consumption.

Pre-existing (not introduced by the prefix work); deferred because touching it shifts an error surface + insta snapshots outside the prefix PR's scope.
