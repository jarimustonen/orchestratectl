---
issue: output-flag-and-streaming
status: ready-to-merge
date: 2026-06-13
---

# Handoff — `--output` flag migration

## What landed

Three commits on `output-flag-and-streaming`:

1. `c55487d` — replace global `--json` with `--output text|json|jsonl`,
   default `jsonl`. Per-subcommand call sites migrated to take
   `&OutputSpec` rather than `json: bool`. `event tail`'s ad-hoc
   `--format` flag is gone; its local `--output FILE` is renamed
   `--to-file` to avoid colliding with the new global. Streaming verbs
   reject `--output json` (pretty single-document JSON is neither valid
   JSON nor valid JSONL) with `unsupported_format`.

2. `5793917` — migrate the integration suite. Existing JSON-shape
   assertions move under `--output json` (still serde-parseable as one
   document); event tail tests use `--output jsonl` (the format
   pretty-json can't satisfy). Added `version_jsonl_default_is_single_line_envelope`
   (also pins that bare `version` ≡ `--output jsonl` byte-for-byte) and
   `version_rejects_legacy_json_flag`.

3. `a18c88a` — update both shipped SKILL.md seeds to document the
   `--output` model and the AI-first `jsonl` default.

## Quality bar

- `cargo build --workspace` ✅
- `cargo test --workspace` ✅ (170 tests, 0 failures, 1 ignored)
- `cargo clippy --workspace --all-targets -- -D warnings` ✅
- `cargo fmt --check` ✅

## Design choices worth noting

**Format inference from file extension.** `--output ./out.jsonl` infers
`jsonl`, `--output ./out.json` infers pretty `json`. Anything else
(including `./out.txt`) errors at parse time. There is intentionally no
text-mode + file destination; for human text the user uses shell
redirection.

**Path detection is conservative.** `--output` interprets a value as a
path when it starts with `/`, starts with `.`, or contains `/`. A bare
extensionless token like `out` is rejected as a format selector, not
silently treated as a relative file. This avoids the "user wanted a
file literally named `text`" footgun named in the issue's escape hatch
(no real corner case surfaced — the strict-token-or-path-shape rule is
unambiguous).

**Streaming verbs reject pretty `json`.** `event tail` (and any future
streaming verb) must declare its format up front per §12. A pretty
multi-line single-document JSON would be neither a closeable doc (the
stream is open-ended) nor parseable line-by-line. We surface
`unsupported_format` early rather than emit invalid bytes.

## Deferred — `/llm-review`

Following the supervisor-process precedent (`08f2fee`), I deferred the
multi-LLM diff review. The change is mechanical: 26 files migrated by
the same dispatch-signature swap, with one substantive new module
(`output.rs`'s parser/dispatch) that is covered by unit tests and
existing integration tests. Mid-context, single-purpose churn is a poor
fit for an asymmetric-expert panel. If a future reviewer wants a panel
pass before merge, the diff is `main..output-flag-and-streaming`.

## Out of scope (per task spec)

- B4 `schema_version` move
- B5/B6/B7 polish
- Long-form `--output FILE` streaming over `event tail --follow` beyond
  the minimal "open file, write envelope/stream, truncate-or-append by
  follow-flag" already in `event/tail.rs`
