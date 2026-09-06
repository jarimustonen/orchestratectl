---
created: 2026-08-06
updated: 2026-08-06
type: task
status: done
priority: normal
closed: 2026-08-06
---

# Preserve sibling wave builds on a hard PipelineError (invariant 5)

## Description


Follow-up from the `immoderately-dirty-cushion` multi-model review (gemini/openai
/opus/deepseek).

In `run_wave_concurrent` (`crates/taskfleet-cli/src/pipeline/live/mod.rs`), when one
build worker returns a hard `PipelineError`, the fold `?`-propagates BEFORE
preserving the wave's other floor-green (`built`) and committed-but-blocked
(`blocked`) sibling worktrees, so `Run::Drop` teardown discards real committed
work. This is deliberately kept consistent with the sequential hard-error path
(which also doesn't preserve), and with the pre-existing merge-phase behaviour —
but reviewers argue the concurrent case is materially different (siblings can have
finished real work by the time one worker errors).

**Task:** preserve every known committed-but-unmerged sibling (`built` + `blocked`)
before propagating a hard error. Consider also the panic/hard-error precedence
(if a wave has BOTH, the hard-error return currently skips panic-path sibling
preservation). Weigh against the desire to stay consistent with the sequential
path. Add tests: (a) one worker hard-errors + a sibling floor-greens → sibling
preserved; (b) hard error + panic in the same wave → siblings preserved.
