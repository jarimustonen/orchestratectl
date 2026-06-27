---
created: 2026-06-27
updated: 2026-06-27
type: task
assignee: jari
status: in-progress
priority: normal
epic: orchestratectl-mvp
labels: [review-spinoff, test]
---

# supervise_gates: replace fixed sleeps with readiness polling (V8 flake under load)

## Description

Observed during the A3 supervisor-process review-followup merge: tests/supervise_gates.rs v8_reattach_end_to_end fails intermittently under heavy parallel load (e.g. immediately after a cold cargo build, when all gate tests launch at once). Root cause is the pre-existing fixed 'std::thread::sleep(Duration::from_millis(500))' waits that assume the spawned '--once'/reattach supervisor has written supervisor.exited within 500ms — which doesn't hold on a saturated CI runner. It passes 100% on warm/isolated runs. Fix: replace the fixed sleeps in V8 (and audit the others) with a poll-until-condition loop on the expected event/PID-file with a generous deadline (the signal_exit_codes_and_payload test added in this review already does this and is stable). Coverage is unchanged — this is purely wait-robustness, which is why it was filed as a spin-off rather than rewritten inline per the A3 scope rules.
