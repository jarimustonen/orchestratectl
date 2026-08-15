//! `config show` — the effective resolved configuration with a per-key
//! `source` (AGENTS-AI-FIRST-CLI §8).
//!
//! Read-only. Every row is the *effective* value an agent would actually get,
//! plus the precedence layer that supplied it (`env | file | default`), so a
//! caller can reason about **why** a value is what it is without guessing. The
//! only configurable surface today is harness selection, resolved through
//! [`crate::harness::select`], so the harness resolver's own precedence is
//! reused verbatim — `config show` never re-implements it.
//!
//! Keys:
//! - `harness.default` — the section-level default (what a run kind with no
//!   per-kind override lands on), via [`select::resolve_default`].
//! - `harness.<kind>` — one row per creatable run kind, the effective harness
//!   for that kind, via [`select::resolve_with`] (per-kind override → section
//!   default → built-in). When `ORCHESTRATECTL_HARNESS` is set it shadows the
//!   file layers, so every row reports `source: "env"` — the honest effective
//!   picture, not the shadowed file value.
//!
//! Secret redaction (§8): each key carries a `secret` flag; a secret key's
//! value is `"<redacted>"` unless `--show-secrets` is passed (which warns on
//! stderr). No key is secret today, so the redaction machinery is present and
//! exercised structurally but currently a no-op.

use octl_core::Kind;
use serde::Serialize;

use crate::config::{config_path, Config, CONFIG_SCHEMA_VERSION};
use crate::error::CliError;
use crate::harness::select::{self, HarnessChoice, HARNESS_ENV};
use crate::output::{self, OutputFormat, OutputSpec};
use crate::run::kind_kebab;

/// Placeholder rendered in place of a secret key's value when it is not
/// revealed (§8).
const REDACTED: &str = "<redacted>";

/// Every creatable run kind, in declaration order — the set `config show`
/// enumerates as `harness.<kind>` rows. A `CREATABLE_KINDS`/[`Kind::WIRE_NAMES`]
/// drift is caught by the `creatable_kinds_match_wire_names` unit test.
const CREATABLE_KINDS: &[Kind] = &[
    Kind::Spinoff,
    Kind::Research,
    Kind::TechnicalDecision,
    Kind::FanOut,
];

/// `config show` JSON payload.
#[derive(Debug, Serialize)]
struct ConfigShowPayload {
    /// Schema version of the `config` payloads (§10).
    schema_version_config: u32,
    /// Absolute config file path (mirrors `config path`).
    path: String,
    /// Whether a file currently exists at [`Self::path`]. `false` means every
    /// key falls through to `env`/`default`.
    exists: bool,
    /// The effective keys, deterministically ordered.
    keys: Vec<ConfigKey>,
}

/// One effective configuration key: its value, the precedence layer that
/// supplied it, and whether it is a secret (§8).
#[derive(Debug, Serialize)]
struct ConfigKey {
    /// Dotted key path, e.g. `harness.default`, `harness.spinoff`.
    key: String,
    /// The effective value — or [`REDACTED`] when `secret` and not revealed.
    value: String,
    /// The precedence layer that supplied the value: `env | file | default`
    /// (the `flag` layer is per-invocation and never a `config show` source).
    source: &'static str,
    /// Whether this key holds a secret. Redacted by default when `true`.
    secret: bool,
}

impl ConfigKey {
    /// Build a non-secret harness row from a resolved [`HarnessChoice`].
    fn harness(key: impl Into<String>, choice: &HarnessChoice) -> Self {
        ConfigKey {
            key: key.into(),
            value: choice.name.clone(),
            source: choice.source.as_str(),
            secret: false,
        }
    }
}

pub fn run(show_secrets: bool, spec: &OutputSpec, warnings: &[String]) -> Result<(), CliError> {
    let path = config_path()?;
    // Load config once and read the env once; every row is resolved against the
    // same ambient snapshot so the picture is internally consistent.
    let config = Config::load()?;
    let env = std::env::var(HARNESS_ENV).ok();
    let env = env.as_deref();

    let mut keys = Vec::with_capacity(1 + CREATABLE_KINDS.len());
    keys.push(ConfigKey::harness(
        "harness.default",
        &select::resolve_default(env, &config)?,
    ));
    for &kind in CREATABLE_KINDS {
        let choice = select::resolve_with(kind, None, env, &config)?;
        keys.push(ConfigKey::harness(
            format!("harness.{}", kind_kebab(kind)),
            &choice,
        ));
    }

    // Redaction pass (§8): a secret key is `<redacted>` unless revealed. No key
    // is secret today, so this is a structural no-op, but it keeps the contract
    // in one place for when a secret-valued key lands.
    let any_secret = keys.iter().any(|k| k.secret);
    if !show_secrets {
        for k in keys.iter_mut().filter(|k| k.secret) {
            k.value = REDACTED.to_string();
        }
    }
    // §8: `--show-secrets` must warn on stderr — but only when it actually
    // reveals something, to avoid a spurious warning on a secret-free surface.
    if show_secrets && any_secret {
        eprintln!("warning: --show-secrets: secret-valued config keys are shown in plaintext");
    }

    let payload = ConfigShowPayload {
        schema_version_config: CONFIG_SCHEMA_VERSION,
        path: path.display().to_string(),
        exists: path.exists(),
        keys,
    };
    match spec.format {
        OutputFormat::Json | OutputFormat::Jsonl => {
            output::emit_envelope(&payload, spec, warnings)?;
        }
        OutputFormat::Text => {
            println!("path:   {}", payload.path);
            println!("exists: {}", payload.exists);
            for k in &payload.keys {
                println!("{:<24} {:<10} ({})", k.key, k.value, k.source);
            }
            output::emit_text_warnings(warnings);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creatable_kinds_match_wire_names() {
        // Guard against `CREATABLE_KINDS` drifting from the canonical creatable
        // set — an added `Kind` must appear in both or `config show` silently
        // omits a `harness.<kind>` row.
        let names: Vec<&str> = CREATABLE_KINDS.iter().map(|k| k.wire_name()).collect();
        assert_eq!(names, Kind::WIRE_NAMES);
    }
}
