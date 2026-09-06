# CI & lint policy — design notes

Captures the lint level chosen for taskfleet and why, so future changes to
the config are deliberate rather than accidental.

## Toolchain

- CI pins `dtolnay/rust-toolchain@stable`. Local dev used stable 1.96 when this
  landed; the workspace declares `rust-version = "1.85"` as the MSRV floor and
  `clippy.toml` sets `msrv = "1.85"` to match.

## Config files

| File | Purpose |
|------|---------|
| `rustfmt.toml` | Stable-only knobs: `edition 2021`, `max_width 100`. Import-grouping options are nightly-only and intentionally omitted. |
| `clippy.toml` | `avoid-breaking-exported-api = false`, `msrv = "1.85"`. Lint *levels* live in `Cargo.toml`, not here. |
| `deny.toml` | cargo-deny: licenses + advisories + bans (sources at defaults). |
| `Cargo.toml` `[workspace.lints.clippy]` | The lint policy below. |

## Clippy level: pedantic = warn (CI: `-D warnings`)

We run **`clippy::pedantic` at `warn`**, opted into per-crate via
`lints.workspace = true`. CI promotes every warning to an error with
`-D warnings`, so pedantic is effectively enforced.

Pedantic floods the current source, so the genuinely noisy / churny lints are
allow-listed (each documented inline in `Cargo.toml`):

- **API-shape noise:** `module_name_repetitions`, `must_use_candidate`,
  `struct_field_names` (the last because the flagged fields — `Node.node_id`,
  `ChildRef.run_id` — are serialized schema fields that must not be renamed).
- **Doc gating deferred until the public API stabilises:** `missing_errors_doc`,
  `missing_panics_doc`.
- **Numeric casts** (`cast_possible_truncation`, `cast_possible_wrap`,
  `cast_sign_loss`, `cast_precision_loss`): deliberate at our value ranges
  (unix timestamps, byte counts, pids).
- **Style preferences that would mean wide rewrites:** `too_many_lines`,
  `manual_let_else`, `needless_pass_by_value`, `items_after_statements`.

`match_same_arms` is **not** globally allowed (multi-model review flagged that a
blanket allow would hide copy-paste bugs in the reducer/dispatch matches). The
four intentional all-variants-enumerated matches carry a local
`#[allow(clippy::match_same_arms)]` instead, keeping the lint active everywhere
else.

Everything else pedantic flags stays active. The machine-applicable findings
(uninlined format args, redundant closures, semicolons, enum glob imports, …)
were fixed via `cargo clippy --fix`; a handful of remaining findings were fixed
by hand (FNV literal separators, `serde_json::Map::default()`) or suppressed
with a documented local `#[allow]` (`trivially_copy_pass_by_ref` on the
length-invariant base32 helper; `used_underscore_binding` on the `cfg(test)`
fault-inject hook). The `cast_*` family is allowed workspace-wide, but the one
hazardous site it was masking — an `agent_pid_hint as u32` truncation of
external create.sh input — was fixed with `u32::try_from` (see review below).

## Rust lints: missing_docs = warn on taskfleet-core only

`taskfleet-core` is the canonical library surface, so it carries
`#![warn(missing_docs)]` (crate-root attribute rather than the `[lints]` table,
which cannot mix `workspace = true` with a `[lints.rust]` override). Every public
item in `error`, `paths`, `projections`, and `schema` is now documented; the
schema field docs double as the on-disk state-format reference. `taskfleet-cli` does
**not** require docs — it is a binary, not a published API.

## cargo-deny

Both `[advisories]` and `[licenses]` pin `version = 2` for stable behaviour
across cargo-deny releases.

- **licenses:** allow-list of MIT / Apache-2.0 (+ LLVM exception) / BSD-2 / BSD-3
  / ISC / Unicode-3.0 / Zlib. The copyleft GPL/AGPL family is therefore denied.
- **advisories:** `yanked = "deny"`; `unmaintained = "all"` and `unsound = "all"`
  (scope = whole dependency tree; in v2 both are fail-by-default advisory classes
  scoped here, not warn/deny knobs). The v1 `vulnerability`/`notice` knobs were
  removed in v2 (cargo-deny#611) — vulnerability advisories now always fail.
- **bans:** `wildcards = "deny"` with `allow-wildcard-paths = true`; both crates
  are `publish = false` so the internal `taskfleet-cli -> taskfleet-core` path dep is
  exempt. Duplicate transitive versions are a warning, not a failure. `deny`
  hard-bans `fs2` (issue cross-platform-lock-validation): a transitive
  reintroduction of the unmaintained/unsound crate fails CI immediately,
  independent of advisory-database state.

## CI jobs (`.github/workflows/ci.yml`)

Six parallel jobs on `push` to `main` and on every PR, all with
`permissions: contents: read` and per-job `timeout-minutes`:

1. `fmt` — `cargo fmt --all --check`
2. `clippy` — `cargo clippy --locked --workspace --all-targets -- -D warnings`
3. `test` — `cargo test --locked --release --workspace`
4. `msrv` — `cargo check --locked --workspace --all-targets` on toolchain `1.85`
   (also provides dev-profile / debug-assertion coverage)
5. `doc` — `cargo doc --locked --workspace --no-deps` with `RUSTDOCFLAGS=-D warnings`
6. `deny` — `cargo-deny check --locked`

All cargo commands use `--locked`. Only the `~/.cargo` registry + git db are
cached via `actions/cache` keyed on `Cargo.lock` — `target/` is deliberately not
cached (its key would lack a rustc-version component, risking stale incremental
restores). `RUSTFLAGS=-D warnings` is **not** set globally (it would deny
warnings in dependency builds and thrash the cache); clippy's `-- -D warnings`
covers our crates.

A `cargo test --release --workspace` run (the shipped profile) is kept per the
issue's success criteria; debug-profile coverage comes from the `msrv` check
job. See `history/review-ci-and-lints.md` for the full multi-model review and
`history/assessment-ci-and-lints.md` for the finding triage (11 FIX applied,
2 spec-mandated DROP, 2 spin-offs filed).
