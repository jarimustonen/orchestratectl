# CI & lint policy — design notes

Captures the lint level chosen for orchestratectl and why, so future changes to
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
  `manual_let_else`, `needless_pass_by_value`, `match_same_arms`,
  `items_after_statements`.

Everything else pedantic flags stays active. The machine-applicable findings
(uninlined format args, redundant closures, semicolons, enum glob imports, …)
were fixed via `cargo clippy --fix`; a handful of remaining findings were fixed
by hand (FNV literal separators, `serde_json::Map::default()`) or suppressed
with a documented local `#[allow]` (`trivially_copy_pass_by_ref` on the
length-invariant base32 helper; `used_underscore_binding` on the `cfg(test)`
fault-inject hook).

## Rust lints: missing_docs = warn on octl-core only

`octl-core` is the canonical library surface, so it carries
`#![warn(missing_docs)]` (crate-root attribute rather than the `[lints]` table,
which cannot mix `workspace = true` with a `[lints.rust]` override). Every public
item in `error`, `paths`, `projections`, and `schema` is now documented; the
schema field docs double as the on-disk state-format reference. `octl-cli` does
**not** require docs — it is a binary, not a published API.

## cargo-deny

- **licenses:** allow-list of MIT / Apache-2.0 (+ LLVM exception) / BSD-2 / BSD-3
  / ISC / Unicode-3.0 / Zlib. The copyleft GPL/AGPL family is therefore denied.
- **advisories:** `yanked = "deny"`; unmaintained surfaced as a workspace warning.
- **bans:** `wildcards = "deny"` with `allow-wildcard-paths = true`; both crates
  are `publish = false` so the internal `octl-cli -> octl-core` path dep is
  exempt. Duplicate transitive versions are a warning, not a failure.

## CI jobs (`.github/workflows/ci.yml`)

Four parallel jobs on `push` to `main` and on every PR: `rustfmt --check`,
`clippy --workspace --all-targets -D warnings`, `test --release --workspace`,
and `cargo-deny check`. Cargo registry + `target` are cached via
`actions/cache` keyed on `Cargo.lock`.
