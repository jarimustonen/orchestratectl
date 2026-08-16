# Envelope snapshots

These `*.snap` files are [`insta`](https://insta.rs) snapshots produced by
`tests/envelope_snapshots.rs`. They lock the **shape** of every
machine-readable contract the CLI emits, per the `/ai-first-cli-canon` skill:

- **Success envelope** (§10) — `{schema_version, data, warnings?}` on stdout
- **Error envelope** (§10) — `{schema_version, error: {code, message,
  invalid_value?, expected?}}` on stderr, exit 1 (validation) or 2
  (refused-but-actionable / system)
- **Dry-run / planning envelopes** (§11)
- **Format coverage** (§9) — each noun captured in `text`, `json` (pretty)
  and `jsonl` (compact)

Coverage: at least one success snapshot per user-facing noun (`version`,
`skill`, `run`, `event`, `node`, `discussion`, `spinoff`) in each of the
three formats (`text`/`json`/`jsonl`), plus per-noun validation errors,
dry-run **and** wet-write envelopes for the mutating verbs, and one exit-2
refusal (`run reattach` against a live supervisor). It does **not**
snapshot every subcommand × format combination — see the file-level doc
comment for the exact matrix.

> The long-running `supervise` daemon noun is intentionally excluded — it
> has no one-shot envelope to snapshot; its behaviour is covered by
> `tests/supervise_gates.rs`.

**Exit codes.** `run reattach` vs. a live supervisor is the representative
refused-but-actionable (exit 2) path. The other nouns' refusals (unknown
id, empty/oversize value, unknown kind) are validation errors (exit 1) and
are covered as such; they have no distinct exit-2 path.

**POSIX-only.** orchestratectl is a unix tool (`libc::kill`, tmux, git
worktrees), and so is this suite: the `[HOME]` path redaction and the
`run reattach` PID trick assume POSIX paths and `kill(pid, 0)` semantics.
It is not expected to pass on Windows.

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

Dynamic (per-test) filters run **first**, then the global filters as a
safety net — so a temp path or id can't be partially clobbered by a global
pattern. All global patterns are boundary- and case-anchored.

| Token | Redacted to | How |
| --- | --- | --- |
| `run_id` (ULID) | `[RUN_ID]` | per-test dynamic filter (also rewrites the copy inside `dir` paths and error messages) |
| `$ORCHESTRATECTL_HOME` temp dir | `[HOME]` | per-test dynamic filter |
| any other ULID-shaped token (e.g. a future `event_id`) | `[ULID]` | global fallback filter |
| git HEAD hash | `[COMMIT]` | global filter (`\b[0-9a-f]{40}\b`) |
| timestamps (`created_at`, `ts`, …) | `[TS]` | global filter, covers RFC3339 `Z`, `+00:00`, and `… UTC` forms |
| live supervisor PID | `[PID]` | global filter (`(?i)pid \d+`) |

If you add a field that carries a fresh ULID/timestamp/path, extend the
filters rather than letting the raw value into the snapshot — otherwise
the test passes once and fails on the next run.

> ⚠️ **Over-redaction caveat.** Because the `[COMMIT]`/`[TS]`/`[ULID]`/`[PID]`
> filters are regex substitutions on the rendered output, a *deterministic*
> field that happens to contain a 40-hex string, a timestamp, a 26-char
> Crockford token, or a literal `pid <N>` would be silently collapsed to a
> placeholder, hiding a real regression. None of the current outputs do
> this. If you add a noun/field whose deterministic value could match,
> scope the filter (or switch that case to structured redaction) rather
> than relying on the global net. The structural envelope assertions in
> `ok_stdout`/`err_stderr` (checking `schema_version`/`data`/`error`
> independently of the snapshot) backstop the most important case — a
> silently-blessed schema bump.

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
