//! `config path` — print the config file location (AGENTS-AI-FIRST-CLI §8).
//!
//! Read-only. The file need not exist; the path itself is what the caller wants
//! ("where would I write settings?"), so a missing file is reported via
//! `exists: false` rather than being an error.

use serde::Serialize;

use crate::config::{config_path, CONFIG_SCHEMA_VERSION};
use crate::error::CliError;
use crate::output::{self, OutputFormat, OutputSpec};

/// `config path` JSON payload.
#[derive(Debug, Serialize)]
struct ConfigPathPayload {
    /// Schema version of the `config` payloads (§10).
    schema_version_config: u32,
    /// Absolute config file path under the resolved Taskfleet home.
    path: String,
    /// Whether a file currently exists at [`Self::path`].
    exists: bool,
}

pub fn run(spec: &OutputSpec, warnings: &[String]) -> Result<(), CliError> {
    let path = config_path()?;
    let payload = ConfigPathPayload {
        schema_version_config: CONFIG_SCHEMA_VERSION,
        path: path.display().to_string(),
        exists: path.exists(),
    };
    match spec.format {
        OutputFormat::Json | OutputFormat::Jsonl => {
            output::emit_envelope(&payload, spec, warnings)?;
        }
        OutputFormat::Text => {
            // Bare path on its own line so `config path` is pipeable
            // (`$(taskfleet config path --output text)`).
            println!("{}", payload.path);
            output::emit_text_warnings(warnings);
        }
    }
    Ok(())
}
