# Envelope snapshots

These `*.snap` files are [`insta`](https://insta.rs) snapshots produced by
`tests/envelope_snapshots.rs`. They lock the **shape** of every
machine-readable contract the CLI emits, per
[`AGENTS-AI-FIRST-CLI.md`](../../../../AGENTS-AI-FIRST-CLI.md):

- **Success envelope** (§10) — `{schema_version, data, warnings?}` on stdout
- **Error envelope** (§10) — `{schema_version, error: {code, message,
  invalid_value?, expected?}}` on stderr, exit 1 (validation) or 2
  (refused-but-actionable / system)
- **Dry-run / planning envelopes** (§11)
- **Format coverage** (§9) — each noun captured in `text`, `json` (pretty)
  and `jsonl` (compact)

Coverage spans all seven user-facing nouns (`version`, `skill`, `run`,
`event`, `node`, `discussion`, `spinoff`) × the three formats, plus
per-noun validation errors, dry-run envelopes, and one exit-2 refusal
(`run reattach` against a live supervisor).

> The long-running `supervise` daemon noun is intentionally excluded — it
> has no one-shot envelope to snapshot; its behaviour is covered by
> `tests/supervise_gates.rs`.

## Why a snapshot suite

The per-subcommand suites (`run.rs`, `event.rs`, …) assert *semantics*
field-by-field. This suite is complementary: it catches **envelope
drift** — a renamed field, changed nesting, a dropped `schema_version`,
or a reformatted text renderer — the moment it happens, in one diff,
regardless of which subcommand a contributor touched.

## Determinism / redactions

Non-deterministic values are redacted with `insta` filters (regex
substitution on the rendered output) *before* comparison, so the
snapshots are stable across machines and runs. See the `snapshot` helper
in `envelope_snapshots.rs`:

| Token | Redacted to | How |
| --- | --- | --- |
| `run_id` (ULID) | `[RUN_ID]` | per-test dynamic filter (also rewrites the copy inside `dir` paths and error messages) |
| `$ORCHESTRATECTL_HOME` temp dir | `[HOME]` | per-test dynamic filter |
| git HEAD hash | `[COMMIT]` | global filter |
| timestamps (`created_at`, `ts`, …) | `[TS]` | global filter, covers RFC3339 `Z`, `+00:00`, and `… UTC` forms |
| live supervisor PID | `[PID]` | global filter |

If you add a field that carries a fresh ULID/timestamp/path, extend the
filters rather than letting the raw value into the snapshot — otherwise
the test passes once and fails on the next run.

## Updating snapshots

A snapshot mismatch is **either** an intended envelope change (accept the
new snapshot) **or** an accidental regression (fix the code). Review the
diff before accepting.

### With `cargo-insta` (recommended)

```sh
cargo install cargo-insta          # one-time
cargo insta test --review -p octl-cli --release --test envelope_snapshots
# 'a' to accept, 'r' to reject, per snapshot
```

`cargo insta accept` accepts all pending snapshots non-interactively.

### Without `cargo-insta`

Pending snapshots are written next to the `.snap` files as `.snap.new`.
To regenerate in place:

```sh
INSTA_UPDATE=always cargo test -p octl-cli --release --test envelope_snapshots
```

Then **inspect `git diff`** on the `.snap` files and commit only the
intended changes.

## Running

Snapshots are generated and verified in `--release` (matching how the
other suites run in CI), but they pass in debug too:

```sh
cargo test -p octl-cli --release --test envelope_snapshots
```
