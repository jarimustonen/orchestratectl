---
created: 2026-06-27
updated: 2026-06-27
type: task
assignee: jari
status: done
priority: normal
epic: orchestratectl-mvp
labels: [review-spinoff, test]
closed: 2026-06-27
---

# supervise_gates: replace fixed sleeps with readiness polling (V8 flake under load)

## Description

Observed during the A3 supervisor-process review-followup merge: tests/supervise_gates.rs v8_reattach_end_to_end fails intermittently under heavy parallel load (e.g. immediately after a cold cargo build, when all gate tests launch at once). Root cause is the pre-existing fixed 'std::thread::sleep(Duration::from_millis(500))' waits that assume the spawned '--once'/reattach supervisor has written supervisor.exited within 500ms — which doesn't hold on a saturated CI runner. It passes 100% on warm/isolated runs. Fix: replace the fixed sleeps in V8 (and audit the others) with a poll-until-condition loop on the expected event/PID-file with a generous deadline (the signal_exit_codes_and_payload test added in this review already does this and is stable). Coverage is unchanged — this is purely wait-robustness, which is why it was filed as a spin-off rather than rewritten inline per the A3 scope rules.

## Agent Runs

### 2026-06-27T18:12:24Z · @jari

Factored a generic poll_until(deadline, FnMut predicate) -> bool test helper (50ms cadence, 30s POLL_DEADLINE) and routed all three detached-process readiness waits through it: V8 event wait (via wait_for_kind), V8 pid-file removal (now fatal with context), and the signal-test pid readiness wait. V2/V3/V7/V9 drive 'supervise --once' synchronously (spawn blocks on exit) so they have no detached wait — intentionally not migrated. Note: commit 9a7cfb6 had already replaced the original fixed 500ms sleeps; remaining work was the helper-unification the issue's scope explicitly asked for. Multi-model /llm-review (Gemini/GPT-5.5/Opus/DeepSeek): applied 5 consensus fixes (bool return, FnMut, elapsed-based timing, single-read wait_for_kind, fatal V8 pid wait); filed 2 spin-offs (supervise-gates-jsonl-poll-tolerance, supervise-gates-signal-wait-hardening). V8 passed 20/20 back-to-back under full 10-core CPU load (twice); workspace fmt/clippy/test clean.
