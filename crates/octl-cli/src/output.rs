//! Stdout success-payload helpers.
//!
//! Every `--json` payload is shaped as:
//!
//! ```json
//! {"schema_version": 1, "data": {...subcommand body...}, "warnings": [...]?}
//! ```
//!
//! The body lives under a dedicated `data` key so the envelope can grow
//! reserved fields (`warnings`, `dry_run`, `trace_id`, ...) over time
//! without colliding with payload field names. This contract is shared
//! by every subcommand — issue #cargo-scaffolding locked it.

use serde::Serialize;

use crate::error::SCHEMA_VERSION;

#[derive(Serialize)]
struct SuccessEnvelope<'a, T: Serialize> {
    schema_version: u32,
    data: &'a T,
    #[serde(skip_serializing_if = "<[String]>::is_empty")]
    warnings: &'a [String],
}

/// Serialize `body` inside the canonical success envelope and print it
/// to stdout.
pub fn emit_json<T: Serialize>(body: &T, warnings: &[String]) -> Result<(), serde_json::Error> {
    let envelope = SuccessEnvelope {
        schema_version: SCHEMA_VERSION,
        data: body,
        warnings,
    };
    println!("{}", serde_json::to_string(&envelope)?);
    Ok(())
}
