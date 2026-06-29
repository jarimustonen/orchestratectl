---
created: 2026-06-12
updated: 2026-06-29
type: chore
status: fixed
priority: normal
closed: 2026-06-29
---

# Relocate envelope SCHEMA_VERSION out of error module

## Description

The success-envelope schema_version constant currently lives in crates/octl-cli/src/error.rs as SCHEMA_VERSION. The envelope applies equally to success and error paths, so the error module is the wrong home — version subcommand currently reads its own envelope schema indirectly via crate::error::SCHEMA_VERSION which is a smell. Move it to a protocol/output module (e.g. introduce ENVELOPE_SCHEMA_VERSION and SUPPORTED_ENVELOPE_SCHEMAS) and have error.rs + output.rs both depend on the new location. Surfaced by the multi-LLM review of #version-subcommand — see history/review-version-subcommand.md §4. Depends on: cargo-scaffolding only.

## Resolution (2026-06-29)

Verified obsolete: `SCHEMA_VERSION` already lives in `crates/octl-core/src/envelope.rs` and is re-exported as `octl_core::SCHEMA_VERSION`. Both `crates/octl-cli/src/error.rs` and `crates/octl-cli/src/output.rs` import it from `octl_core` directly — the `crate::error::SCHEMA_VERSION` smell the issue describes no longer exists in tree.

The proposed cosmetic refinement (rename to `ENVELOPE_SCHEMA_VERSION` and add a `SUPPORTED_ENVELOPE_SCHEMAS` slice for forward-compat) is a separate, optional cleanup. Closing this issue as fixed; if the rename is wanted later it can be filed as its own chore.
