---
created: 2026-06-12
updated: 2026-06-12
type: chore
status: open
priority: normal
---

# Relocate envelope SCHEMA_VERSION out of error module

## Description

The success-envelope schema_version constant currently lives in crates/octl-cli/src/error.rs as SCHEMA_VERSION. The envelope applies equally to success and error paths, so the error module is the wrong home — version subcommand currently reads its own envelope schema indirectly via crate::error::SCHEMA_VERSION which is a smell. Move it to a protocol/output module (e.g. introduce ENVELOPE_SCHEMA_VERSION and SUPPORTED_ENVELOPE_SCHEMAS) and have error.rs + output.rs both depend on the new location. Surfaced by the multi-LLM review of #version-subcommand — see history/review-version-subcommand.md §4. Depends on: cargo-scaffolding only.
