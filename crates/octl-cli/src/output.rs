//! Stdout success-payload helpers.
//!
//! Every `--json` payload carries `schema_version: 1` and an optional
//! `warnings: []` array per `AGENTS-AI-FIRST-CLI.md` §10.

use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::error::SCHEMA_VERSION;

/// Wraps a serializable body into the canonical success envelope:
/// `{schema_version, ...body, warnings: [] (omitted if empty)}`.
pub fn emit_json<T: Serialize>(body: &T, warnings: &[String]) -> Result<(), serde_json::Error> {
    let mut envelope: Map<String, Value> = Map::new();
    envelope.insert(
        "schema_version".to_string(),
        Value::Number(SCHEMA_VERSION.into()),
    );

    let body_value = serde_json::to_value(body)?;
    if let Value::Object(map) = body_value {
        for (k, v) in map {
            envelope.insert(k, v);
        }
    } else {
        envelope.insert("result".to_string(), body_value);
    }

    if !warnings.is_empty() {
        envelope.insert(
            "warnings".to_string(),
            json!(warnings.iter().collect::<Vec<_>>()),
        );
    }

    println!("{}", serde_json::to_string(&Value::Object(envelope))?);
    Ok(())
}
