# log-delivery-hardening — review triage & decisions

Multi-model `/llm-review` (gemini-3.1-pro, gpt-5.5, claude-opus-4-7,
deepseek-v4-pro) on commits `da1aad8..` (the implementation). Raw reviews:
`history/review-log-delivery-hardening.md` (gitignored scratch).

## Applied (commit "address multi-model review findings")

| # | Finding | Action |
|---|---------|--------|
| 1 | Metric buried in a prose `warnings` string — agents must regex-parse it (all 4) | Added additive `dropped_log_events: Option<u64>` envelope field (no schema bump). Kept the human string too. |
| 2 | `TASKFLEET_TEST_SLOW_LOG_WRITES` ships in release → stray env var stalls logging (all 4) | Gated `slow_log_write_delay()` behind `cfg(debug_assertions)`; release stub returns `None`. Hook is inert in shipped binaries; still live for the debug test binary. |
| 3 | Supervisor drop `warn!` rides the same lossy channel it reports on, so the SOS can itself be dropped (gpt, opus, deepseek) | `maybe_warn_dropped` now also `eprintln!`s the warning (reliable stderr → `supervisor.stderr.log`). |
| 4 | `SlowLogWriter::write` mishandles short writes → truncated JSONL (opus, gpt, deepseek) | Use `write_all`, return full len. |
| 5 | `duration_since` brittle with injected clock (gpt) | `saturating_duration_since`. |
| 6 | "1 log event(s)" grammar (deepseek) | Singular/plural. |
| 7 | Overflow test `> 0` too loose (opus, gemini) | Assert `>= 40` of 50. |
| 8 | `LOG_DROPPED` set-once looks racy on re-init (all 4) | No code change — set-once is correct (counter only published on the one successful `try_init`; re-init reaches `finish_logging` with `None`). Clarified in comment. |
| 9 | Shutdown breadcrumb justified "to pass the test" (gemini, gpt, opus, deepseek) | Reframed comment to lead with operational value (records *why* the supervisor stopped in the process log); test reliance is secondary. |

## Rejected (verified false)

- **DeepSeek: "`SlowSink` missing → won't compile."** It exists in the
  `cli.rs` test module (line ~731). Build + tests pass.
- **GPT: broken intra-doc links (`[output::emit_envelope]`).**
  `RUSTDOCFLAGS=-D warnings cargo doc` passes — `output` is in scope via
  `use crate::output`.

## Deferred (out of scope — candidate follow-ups)

- **Error-envelope drop surfacing (gpt #4):** drops are surfaced on the
  success envelope + supervisor warn, but a command that *fails* after
  dropping logs won't show the count in its error envelope. Worth a
  follow-up issue if error-path log loss matters.
- **`emit_text_warnings` not a type-enforced chokepoint (gpt #5):**
  pre-existing architecture; every command opts in. Not introduced here.
- **Configurable `DROPPED_WARN_INTERVAL` (opus #11):** YAGNI for MVP.
- **Panic-hook flush (opus #22):** `panic = "abort"` still loses logs;
  documented limitation, unchanged.

## Note on the envelope schema

Adding `dropped_log_events` is additive (new optional field, omitted when
zero) → no `SCHEMA_VERSION` bump, per AGENTS-AI-FIRST-CLI §10.
