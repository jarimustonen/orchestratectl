//! `config show` — tolerant, layered configuration inspection (§8).
//!
//! Unlike execution, inspection does not deserialize through [`Config`]: it
//! parses the file as raw TOML and validates every harness layer independently.
//! This lets a caller see an invalid file value, including one shadowed by
//! `ORCHESTRATECTL_HARNESS`, without weakening the strict resolver used by
//! `run create`. Only unreadable or syntactically invalid TOML is fatal.
//!
//! Each key has an ordered `layers` stack (highest precedence first), an
//! `effective_value` / `effective_source`, and effective-layer validity. File
//! layers carry `origin_key`, which distinguishes a per-kind override from the
//! inherited `[harness] default`. Invalid layers carry `validation_error` and
//! also produce an envelope warning, even when a valid higher layer shadows
//! them.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use octl_core::Kind;
use serde::Serialize;

use crate::config::{config_path, CONFIG_SCHEMA_VERSION};
use crate::error::CliError;
use crate::harness::{select::HARNESS_ENV, DEFAULT_HARNESS, KNOWN_HARNESSES};
use crate::output::{self, OutputFormat, OutputSpec};
use crate::run::kind_kebab;

const REDACTED: &str = "<redacted>";

const CREATABLE_KINDS: &[Kind] = &[
    Kind::Spinoff,
    Kind::Research,
    Kind::TechnicalDecision,
    Kind::FanOut,
];

#[derive(Debug, Serialize)]
struct ConfigShowPayload {
    schema_version_config: u32,
    path: String,
    exists: bool,
    /// Known effective keys followed by any unrecognized harness entries.
    keys: Vec<ConfigKey>,
}

#[derive(Debug, Serialize)]
struct ConfigKey {
    key: String,
    effective_value: String,
    effective_source: &'static str,
    /// Validity of the effective layer. Shadowed layers retain independent
    /// validity in `layers`, so an env override cannot launder a bad file value.
    valid: bool,
    validation_error: Option<String>,
    secret: bool,
    /// Highest precedence first. Exactly one layer is `active`.
    layers: Vec<ConfigLayer>,
}

#[derive(Debug, Clone, Serialize)]
struct ConfigLayer {
    value: String,
    source: &'static str,
    /// Exact TOML key for file layers; absent for env and built-in defaults.
    origin_key: Option<String>,
    valid: bool,
    validation_error: Option<String>,
    active: bool,
}

#[derive(Default)]
struct RawHarness {
    default: Option<toml::Value>,
    per_kind: BTreeMap<String, toml::Value>,
    unknown: BTreeMap<String, toml::Value>,
    section_error: Option<(String, String)>,
}

impl ConfigLayer {
    fn harness(value: String, source: &'static str, origin_key: Option<String>) -> Self {
        let validation_error = harness_validation_error(&value);
        Self {
            value,
            source,
            origin_key,
            valid: validation_error.is_none(),
            validation_error,
            active: false,
        }
    }

    fn invalid_file(value: String, origin_key: String, error: String) -> Self {
        Self {
            value,
            source: "file",
            origin_key: Some(origin_key),
            valid: false,
            validation_error: Some(error),
            active: false,
        }
    }
}

impl ConfigKey {
    fn from_layers(key: impl Into<String>, mut layers: Vec<ConfigLayer>) -> Self {
        let effective = layers
            .first_mut()
            .expect("every known config key has a built-in default layer");
        effective.active = true;
        Self {
            key: key.into(),
            effective_value: effective.value.clone(),
            effective_source: effective.source,
            valid: effective.valid,
            validation_error: effective.validation_error.clone(),
            secret: false,
            layers,
        }
    }

    fn invalid_only(key: impl Into<String>, mut layer: ConfigLayer) -> Self {
        layer.active = true;
        Self {
            key: key.into(),
            effective_value: layer.value.clone(),
            effective_source: layer.source,
            valid: false,
            validation_error: layer.validation_error.clone(),
            secret: false,
            layers: vec![layer],
        }
    }
}

pub fn run(show_secrets: bool, spec: &OutputSpec, warnings: &[String]) -> Result<(), CliError> {
    let path = config_path()?;
    let raw = load_raw_harness(&path)?;
    let env = std::env::var(HARNESS_ENV).ok();
    // Match execution semantics: an empty/whitespace env value is unset.
    let env = env.as_deref().map(str::trim).filter(|v| !v.is_empty());

    let mut keys = Vec::with_capacity(1 + CREATABLE_KINDS.len() + raw.unknown.len());
    keys.push(ConfigKey::from_layers(
        "harness.default",
        harness_layers(
            env,
            raw.default
                .as_ref()
                .map(|value| (value, "harness.default".to_string())),
            None,
        ),
    ));
    for &kind in CREATABLE_KINDS {
        let name = kind_kebab(kind);
        keys.push(ConfigKey::from_layers(
            format!("harness.{name}"),
            harness_layers(
                env,
                raw.per_kind
                    .get(name)
                    .map(|value| (value, format!("harness.per_kind.{name}"))),
                raw.default
                    .as_ref()
                    .map(|value| (value, "harness.default".to_string())),
            ),
        ));
    }

    // Parseable but schema-invalid entries are inspection data, not fatal
    // errors. Preserve them as rows rather than hiding a typo from the caller.
    if let Some((value, error)) = raw.section_error {
        keys.push(ConfigKey::invalid_only(
            "harness",
            ConfigLayer::invalid_file(value, "harness".into(), error),
        ));
    }
    for (name, value) in raw.unknown {
        let origin = format!("harness.{name}");
        let error = if name == "per_kind" {
            format!(
                "expected [harness.per_kind] table, found {}",
                value.type_str()
            )
        } else {
            "unknown key in [harness]; expected default or per_kind".into()
        };
        keys.push(ConfigKey::invalid_only(
            origin.clone(),
            ConfigLayer::invalid_file(raw_value(&value), origin, error),
        ));
    }
    for (name, value) in raw
        .per_kind
        .iter()
        .filter(|(name, _)| !Kind::WIRE_NAMES.contains(&name.as_str()))
    {
        let key = format!("harness.{name}");
        keys.push(ConfigKey::invalid_only(
            key,
            ConfigLayer::invalid_file(
                raw_value(value),
                format!("harness.per_kind.{name}"),
                format!(
                    "unknown run kind '{name}'; valid kinds: {}",
                    Kind::WIRE_NAMES.join(", ")
                ),
            ),
        ));
    }

    let mut command_warnings = warnings.to_vec();
    let mut seen_invalid_layers = BTreeSet::new();
    for key in &keys {
        for layer in key.layers.iter().filter(|layer| !layer.valid) {
            let warning = format!(
                "{} {} value '{}' is invalid: {}",
                layer.origin_key.as_deref().unwrap_or(&key.key),
                layer.source,
                layer.value,
                layer.validation_error.as_deref().unwrap_or("invalid value")
            );
            if seen_invalid_layers.insert(warning.clone()) {
                command_warnings.push(warning);
            }
        }
    }

    let any_secret = keys.iter().any(|key| key.secret);
    if !show_secrets {
        for key in keys.iter_mut().filter(|key| key.secret) {
            key.effective_value = REDACTED.to_string();
            for layer in &mut key.layers {
                layer.value = REDACTED.to_string();
            }
        }
    } else {
        add_show_secrets_warning(any_secret, &mut command_warnings);
    }

    let payload = ConfigShowPayload {
        schema_version_config: CONFIG_SCHEMA_VERSION,
        path: path.display().to_string(),
        exists: path.exists(),
        keys,
    };
    match spec.format {
        OutputFormat::Json | OutputFormat::Jsonl => {
            output::emit_envelope(&payload, spec, &command_warnings)?;
        }
        OutputFormat::Text => {
            println!("path:   {}", payload.path);
            println!("exists: {}", payload.exists);
            for key in &payload.keys {
                let validity = if key.valid { "valid" } else { "INVALID" };
                println!(
                    "{} = {} ({}, {validity})",
                    key.key, key.effective_value, key.effective_source
                );
                for layer in &key.layers {
                    let marker = if layer.active { "*" } else { " " };
                    let origin = layer
                        .origin_key
                        .as_deref()
                        .map(|origin| format!(" {origin}"))
                        .unwrap_or_default();
                    let validity = if layer.valid { "valid" } else { "INVALID" };
                    println!(
                        "  {marker} {:<7} {:<10} ({validity}){}",
                        layer.source, layer.value, origin
                    );
                    if let Some(error) = &layer.validation_error {
                        println!("      validation_error: {error}");
                    }
                }
            }
            output::emit_text_warnings(&command_warnings);
        }
    }
    Ok(())
}

fn harness_layers(
    env: Option<&str>,
    specific_file: Option<(&toml::Value, String)>,
    inherited_file: Option<(&toml::Value, String)>,
) -> Vec<ConfigLayer> {
    let mut layers = Vec::new();
    if let Some(value) = env {
        layers.push(ConfigLayer::harness(value.into(), "env", None));
    }
    if let Some((value, origin)) = specific_file {
        layers.push(file_harness_layer(value, &origin));
    }
    if let Some((value, origin)) = inherited_file {
        layers.push(file_harness_layer(value, &origin));
    }
    layers.push(ConfigLayer::harness(
        DEFAULT_HARNESS.into(),
        "default",
        None,
    ));
    layers
}

fn file_harness_layer(value: &toml::Value, origin: &str) -> ConfigLayer {
    match value.as_str() {
        Some(value) => ConfigLayer::harness(value.into(), "file", Some(origin.into())),
        None => ConfigLayer::invalid_file(
            raw_value(value),
            origin.into(),
            format!("expected a string harness name, found {}", value.type_str()),
        ),
    }
}

fn add_show_secrets_warning(any_secret: bool, warnings: &mut Vec<String>) {
    if any_secret {
        // JSON warnings belong in the stdout envelope (§10), never stderr.
        warnings.push("--show-secrets: secret-valued config keys are shown in plaintext".into());
    }
}

fn harness_validation_error(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        Some(format!(
            "empty harness name; known harnesses: {}",
            KNOWN_HARNESSES.join(", ")
        ))
    } else if KNOWN_HARNESSES.contains(&value) {
        None
    } else {
        Some(format!(
            "unknown harness '{value}'; known harnesses: {}",
            KNOWN_HARNESSES.join(", ")
        ))
    }
}

fn raw_value(value: &toml::Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| serde_json::to_string(value).unwrap_or_else(|_| format!("{value:?}")))
}

fn load_raw_harness(path: &Path) -> Result<RawHarness, CliError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RawHarness::default());
        }
        Err(error) => {
            return Err(CliError::system(
                "config_unreadable",
                format!("could not read config file {}: {error}", path.display()),
            ));
        }
    };
    if text.trim().is_empty() {
        return Ok(RawHarness::default());
    }
    let document: toml::Value = toml::from_str(&text).map_err(|error| {
        CliError::user(
            "invalid_config",
            format!("config file {} is not valid TOML: {error}", path.display()),
        )
    })?;
    let Some(harness) = document.get("harness") else {
        return Ok(RawHarness::default());
    };
    let Some(table) = harness.as_table() else {
        return Ok(RawHarness {
            section_error: Some((
                raw_value(harness),
                format!("expected [harness] table, found {}", harness.type_str()),
            )),
            ..RawHarness::default()
        });
    };

    let mut raw = RawHarness::default();
    for (name, value) in table {
        match name.as_str() {
            "default" => raw.default = Some(value.clone()),
            "per_kind" => match value.as_table() {
                Some(per_kind) => raw.per_kind = per_kind.clone().into_iter().collect(),
                None => {
                    raw.unknown.insert(name.clone(), value.clone());
                }
            },
            _ => {
                raw.unknown.insert(name.clone(), value.clone());
            }
        }
    }
    Ok(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creatable_kinds_match_wire_names() {
        let names: Vec<&str> = CREATABLE_KINDS
            .iter()
            .map(|kind| kind.wire_name())
            .collect();
        assert_eq!(names, Kind::WIRE_NAMES);
    }

    #[test]
    fn show_secrets_warning_is_an_envelope_warning() {
        // Keep this policy pinned even while the current config has no secrets.
        let mut warnings = Vec::new();
        add_show_secrets_warning(true, &mut warnings);
        assert_eq!(
            warnings,
            ["--show-secrets: secret-valued config keys are shown in plaintext"]
        );
    }
}
