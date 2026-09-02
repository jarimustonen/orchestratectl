# R7 validation

Validated on 2026-09-02 from implementation commits `028005a` and `007dd40`.

## Release/distribution topology

- cargo-dist 0.28.2 `generate --check` and normalized plan validation passed.
- `scripts/test-distribution-topology.sh` passed end to end. It ran a real native
  cargo-dist build, inspected the archive/shell installer/formula/collected
  compatibility stub, cloned both taps at pinned heads, and exercised disposable
  Homebrew 6.0.21 cross-tap resolution without installation.
- The normalized plan is
  `receipts/cargo-dist-plan.json`; exact native artifact digests and members are
  in `receipts/native-artifact-build.json`.
- A credential-free `dist host --steps=create` experiment changed neither the
  public release-list nor tag-ref digest; see `receipts/host-create-no-mutation.json`.
- The canonical tap still points at empty-tree proof commit
  `db12bb163e47617f0b941a35d3896b6ba0548892` and contains no files. The old tap
  remains unchanged at `85ce830378f38cf17283efddd966d5754354e403`.
- `shipshape contract validate --json`, `shipshape audit --json` (zero blocking
  gaps), `scripts/test-shipshape-release.sh`, and
  `scripts/test-publish-crates.sh` passed.
- Exact Shipshape 0.10.1 commit
  `3e46568d6969701c5fea82fb134b62aa17121cbe` passed the real held-tag/local-resume
  protocol at fixture `v0.6.0`; its production remote remained untouched.

## Review and corrections

A four-model `/llm-review` completed with three responses and one model transport
failure. `/assess-findings` classified six confirmed findings and three incorrect
or out-of-scope claims. Confirmed findings were fixed: actual cargo-dist artifact
collection is tested, rejected non-dry dispatches cancel the whole release run,
future tag events fail closed, broad proof credentials were removed, runner
selection was narrowed, and all evidence limitations/later gates are explicit.
No new issue was filed because remaining R8-R11 work already has named ADR gates.

## Full repository gate

All required post-implementation checks passed:

```text
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo nextest run --locked --release --workspace       # 1141 passed, 1 skipped
cargo test --locked --release --workspace --doc
RUSTDOCFLAGS="-D warnings" cargo doc --locked --workspace --no-deps
cargo package --workspace --no-verify
```

The identity ledger regenerated and validated. `issuectl doctor --json` reported
no parse errors, schema violations, or broken references. Its known derived
AGENTS drift remains unrelated repository maintenance; no generated policy file
was rewritten in this focused issue.

## Deliberately not performed

No source release/tag, crate publication, repository rename, formula activation,
old-tap mutation, Homebrew install, Taskfleet binary/skill installation, or
release activation occurred. Distribution and release authority remain blocked
on R8, R9, and R10.
