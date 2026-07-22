---
created: 2026-07-22
updated: 2026-07-22
type: task
status: open
priority: high
---

# CodeHarness execution-control gap: timeout enforcement + cancellation-token trait param (T0 follow-up, blocks live wiring)

## Description

Surfaced by the T0 (`codeharness-adapter-interface`) multi-model `/llm-review`
(all four reviewers, top priority). The `CodeHarness` contract defines
`ChunkOutcome::Timeout` and `ChunkOutcome::Cancelled`, but the T0 shape cannot
*produce* either: `run_chunk` is synchronous, `AiderHarness` uses blocking
`Command::output()` with no deadline, and the trait has no cancellation channel.
A hung `aider` run (or one that drops to an interactive prompt despite
`--yes-always`) would block the supervisor thread forever, and design §9's
resource circuit-breakers (wall-time ceiling, cost kill-switch) have no way to
stop an in-flight chunk and get a structured result back.

This was **deliberately deferred out of T0** (behavior-preserving contract
scaffolding). Execution control is design §9 / breakdown **T6** (resource
circuit-breakers) territory. Filed as a structured gap (design principle 4)
rather than half-built, because it touches the **trait signature** — which
must be settled before T5's staged supervisor binds to it.

## What to resolve (before the harness is wired into any live path — T5/T11)

1. **Timeout enforcement in adapters.** Add a deadline to the request/execution
   context; `AiderHarness` must spawn + wait-with-timeout, kill the child
   **process group** on expiry, drain stdout/stderr without deadlocking, and
   return `ChunkOutcome::Timeout` with the partial transcript. Per-`Check`
   timeouts too (a `cargo test` must not hang the chunk).
2. **Cancellation-token trait parameter.** Decide the shape — `run_chunk(&self,
   req, cancel: &CancelToken)` vs an async trait — so a circuit-breaker can
   cancel an in-flight chunk and receive `ChunkOutcome::Cancelled`. Whatever is
   chosen, thread it through the stub + conformance suite.
3. **`Send + Sync` was added in T0**, but the concurrency model (shared
   `Arc<dyn CodeHarness>` vs per-call state) should be confirmed here.

## Related contract-evolution items also raised by the T0 review (lower priority)

These were judged out of scope for T0's correctness pass; capture for the
governed schema-evolution process (design §13) when the pipeline matures:

- **Bounded output capture** — aider transcript + `CheckResult.stdout/stderr`
  are unbounded `String`s; a noisy tool/check can exhaust memory and bloat the
  serialized provenance. Stream with hard caps + a `truncated` marker
  (resource-safety, T6-adjacent).
- **Structured `Failed` category + retryability** — `ChunkOutcome::Failed {
  reason: String }` / `HarnessError` variants don't tell the supervisor whether
  to retry / rotate provider / repair the worktree. Add a `FailureCategory` +
  `retryable` for the triage loop (design §8).
- **Artifact-ref indirection** — `transcript_ref: Option<PathBuf>` is host-local
  and temp-dir-ephemeral; a remote adapter needs a durable `{uri, sha256,
  size}` artifact reference, and the supervisor needs an ownership/cleanup
  contract for the `/tmp/octl-harness/...` tree.
- **Check-env scrubbing + secret redaction** — checks run via `sh -c` inherit
  the full environment (incl. the provider key) and transcripts may echo
  secrets to `/tmp`. Scrub the check env; redact known-secret patterns before
  persisting transcripts.
- **Reproducibility capture** — record the resolved aider version + full
  effective invocation + commit mode as provenance; guard `AiderConfig::extra_args`
  against overriding the fixed flags.
- **Request-side `schema_version`** — `ChunkResult` carries one; `ChunkRequest`
  does not.

## Context

- Contract lives at `crates/octl-cli/src/harness/` (T0).
- `issues/code-pipeline/design.md` §9 (circuit-breakers), §10 (harness contract).
- Blocks: breakdown **T5** (staged supervisor) / **T6** (circuit-breakers) /
  **T11** (rollout wiring) — must land before auto-merge is enabled.

