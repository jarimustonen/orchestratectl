//! User-facing configuration file — the `file` layer of the
//! flag > env > file > default precedence (AGENTS-AI-FIRST-CLI §8) — plus the
//! read-only `config` noun that inspects it.
//!
//! Two responsibilities live here:
//!
//! - [`Config`] / [`config_path`] — the loader for the `file` layer, consumed by
//!   the harness resolver ([`crate::harness::select`]).
//! - The `config` subcommand ([`ConfigAction`], [`dispatch`]) — `config path`
//!   prints the file location and `config show` prints a tolerant, layered view
//!   of raw and effective values (`env > file > default`), including validity
//!   for every layer. This inspection path does not weaken strict execution
//!   validation. Read-only: it never mutates the file. Verbs live in [`show`]
//!   and [`path`].
//!
//! Location: `<ORCHESTRATECTL_HOME or ~/.orchestratectl>/config.toml`. The file
//! is entirely optional; a missing or empty file yields [`Config::default`], so
//! every setting falls through to its built-in default. This is the first
//! config-file layer in the tool — before it, configuration was purely
//! environment-variable driven (see `home.rs`).
//!
//! Today it carries exactly one section, `[harness]`, consumed by
//! `run create --harness` (see [`crate::harness::select`]):
//!
//! ```toml
//! [harness]
//! # Default harness for every run kind unless overridden below.
//! default = "pi"
//!
//! [harness.per_kind]
//! # Per-kind overrides, keyed by the kebab-case run kind. Lets a repo default
//! # autonomous work to pi while interactive `code` stays on the global default.
//! research = "pi"
//! spinoff = "pi"
//! ```
//!
//! Parsing is strict where it catches user mistakes, lenient where strictness
//! would only hurt forward-compat:
//!
//! - A syntactically invalid `config.toml` is a hard [`CliError`] (a silently
//!   ignored config is worse than a loud one — the caller would reason about a
//!   value the file was supposed to set).
//! - An unknown **key** inside `[harness]` (`defualt = …`) is rejected
//!   (`deny_unknown_fields` on [`HarnessConfig`]); a typo'd `[harness.per_kind]`
//!   run-kind key is validated against [`octl_core::Kind::WIRE_NAMES`] and
//!   likewise rejected. Both fail loudly rather than silently no-op'ing.
//! - An unknown top-level **section** is tolerated (no `deny_unknown_fields` on
//!   [`Config`]) so a newer build's future `[section]` never bricks an older
//!   build reading the same home.

use std::collections::BTreeMap;
use std::path::PathBuf;

use clap::Subcommand;
use serde::Deserialize;

use crate::error::CliError;
use crate::output::OutputSpec;

pub mod path;
pub mod show;

/// Schema version of the `config` subcommand's JSON payloads (the
/// `schema_version_config` field on `config path` / `config show`). Bumped
/// independently of the run-state schema so an agent can pin the shape of the
/// config surface (AGENTS-AI-FIRST-CLI §10).
pub const CONFIG_SCHEMA_VERSION: u32 = 2;

/// The `config` noun — read-only layered inspection of the configuration
/// (AGENTS-AI-FIRST-CLI §8). Never mutates `config.toml`.
#[derive(Subcommand, Debug)]
pub enum ConfigAction {
    /// Print the config file location (whether or not the file exists).
    Path,
    /// Print raw layered and effective configuration with per-layer validity.
    /// Invalid values are reported as warnings rather than hiding the value.
    /// Secret-valued keys are redacted unless `--show-secrets` is given.
    Show {
        /// Reveal secret-valued keys instead of redacting them. Emits a
        /// warning to stderr. No config key is secret today, so this is a
        /// forward-compatible no-op on the current surface.
        #[arg(long)]
        show_secrets: bool,
    },
}

/// Dispatch a `config` subcommand to its verb module.
pub fn dispatch(
    action: ConfigAction,
    spec: &OutputSpec,
    warnings: &[String],
) -> Result<(), CliError> {
    match action {
        ConfigAction::Path => path::run(spec, warnings),
        ConfigAction::Show { show_secrets } => show::run(show_secrets, spec, warnings),
    }
}

/// The parsed `config.toml`. All sections optional; an absent section leaves the
/// corresponding settings at their built-in defaults.
///
/// Deliberately NOT `deny_unknown_fields` at the top level: an unknown *section*
/// (e.g. a future `[ui]` written by a newer build sharing the same home) must not
/// brick an older build's `run create`. Strictness lives one level down on
/// [`HarnessConfig`], where an unknown *key* in `[harness]` IS a typo worth
/// failing on.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct Config {
    /// `[harness]` — harness-selection defaults for `run create`.
    #[serde(default)]
    pub harness: HarnessConfig,
}

/// The `[harness]` section: a global default plus per-kind overrides.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HarnessConfig {
    /// Default harness for all run kinds unless a `per_kind` entry overrides it.
    /// `None` (key absent) falls through to the built-in default (`pi`).
    #[serde(default)]
    pub default: Option<String>,
    /// Per-kind overrides, keyed by kebab-case run kind (`research`, `spinoff`,
    /// …). A `BTreeMap` keeps the parse deterministic. Because `deny_unknown_fields`
    /// cannot police map keys, [`Config::load_from`] validates every key against
    /// [`octl_core::Kind::WIRE_NAMES`] at load time — a typo'd kind (`reserach`)
    /// fails loudly instead of silently no-op'ing (which would defeat the whole
    /// point of the override).
    #[serde(default)]
    pub per_kind: BTreeMap<String, String>,
}

/// The config file path under the resolved orchestratectl home. Inspectable so a
/// caller never has to guess where settings come from (AGENTS-AI-FIRST-CLI §8).
pub fn config_path() -> Result<PathBuf, CliError> {
    Ok(crate::home::root_dir()?.join("config.toml"))
}

impl Config {
    /// Load the config from the default [`config_path`]. A missing file is not an
    /// error — it yields [`Config::default`]. A present-but-malformed file (bad
    /// TOML, unknown key, wrong value type) is a hard [`CliError`] naming the
    /// path, so a typo can never silently drop a setting.
    pub fn load() -> Result<Self, CliError> {
        Self::load_from(&config_path()?)
    }

    /// Load from an explicit path (the seam the tests drive).
    pub fn load_from(path: &std::path::Path) -> Result<Self, CliError> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            // Absent config → all defaults. This is the overwhelmingly common
            // case (most users never write a config.toml).
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => {
                return Err(CliError::system(
                    "config_unreadable",
                    format!("could not read config file {}: {e}", path.display()),
                ));
            }
        };
        if text.trim().is_empty() {
            return Ok(Self::default());
        }
        let config: Self = toml::from_str(&text).map_err(|e| {
            CliError::user(
                "invalid_config",
                format!("config file {} is not valid: {e}", path.display()),
            )
        })?;
        // `deny_unknown_fields` guards the `[harness]` struct fields but not the
        // dynamic `per_kind` map keys, so validate them here — a typo'd kind
        // (`reserach = "pi"`) is a mistake the user wants surfaced, not silently
        // ignored at lookup time.
        for key in config.harness.per_kind.keys() {
            if !octl_core::Kind::WIRE_NAMES.contains(&key.as_str()) {
                return Err(CliError::user(
                    "invalid_config",
                    format!(
                        "config file {} has an unknown run kind '{}' in [harness.per_kind]; \
                         valid kinds: {}",
                        path.display(),
                        key,
                        octl_core::Kind::WIRE_NAMES.join(", ")
                    ),
                )
                .with_invalid_value(key.clone()));
            }
        }
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write(dir: &TempDir, body: &str) -> PathBuf {
        let p = dir.path().join("config.toml");
        std::fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn missing_file_is_default() {
        let dir = TempDir::new().unwrap();
        let cfg = Config::load_from(&dir.path().join("nope.toml")).unwrap();
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn empty_file_is_default() {
        let dir = TempDir::new().unwrap();
        let p = write(&dir, "   \n\n");
        assert_eq!(Config::load_from(&p).unwrap(), Config::default());
    }

    #[test]
    fn parses_default_and_per_kind() {
        let dir = TempDir::new().unwrap();
        let p = write(
            &dir,
            r#"
[harness]
default = "pi"

[harness.per_kind]
research = "pi"
spinoff = "claude"
"#,
        );
        let cfg = Config::load_from(&p).unwrap();
        assert_eq!(cfg.harness.default.as_deref(), Some("pi"));
        assert_eq!(
            cfg.harness.per_kind.get("research").map(String::as_str),
            Some("pi")
        );
        assert_eq!(
            cfg.harness.per_kind.get("spinoff").map(String::as_str),
            Some("claude")
        );
    }

    #[test]
    fn unknown_key_is_rejected() {
        let dir = TempDir::new().unwrap();
        let p = write(&dir, "[harness]\ndefualt = \"pi\"\n");
        let err = Config::load_from(&p).unwrap_err();
        assert_eq!(err.code, "invalid_config");
    }

    #[test]
    fn malformed_toml_is_hard_error() {
        let dir = TempDir::new().unwrap();
        let p = write(&dir, "[harness\n");
        let err = Config::load_from(&p).unwrap_err();
        assert_eq!(err.code, "invalid_config");
    }

    #[test]
    fn unknown_per_kind_run_kind_is_rejected() {
        // deny_unknown_fields cannot police map keys, so load-time validation
        // must: a typo'd run kind fails loudly instead of silently no-op'ing.
        let dir = TempDir::new().unwrap();
        let p = write(&dir, "[harness.per_kind]\nreserach = \"pi\"\n");
        let err = Config::load_from(&p).unwrap_err();
        assert_eq!(err.code, "invalid_config");
        assert_eq!(err.invalid_value.as_deref(), Some("reserach"));
    }

    #[test]
    fn valid_per_kind_run_kinds_pass() {
        let dir = TempDir::new().unwrap();
        let p = write(
            &dir,
            "[harness.per_kind]\nresearch = \"pi\"\ntechnical-decision = \"claude\"\n",
        );
        let cfg = Config::load_from(&p).unwrap();
        assert_eq!(
            cfg.harness.per_kind.get("research").map(String::as_str),
            Some("pi")
        );
    }

    #[test]
    fn unknown_top_level_section_is_tolerated() {
        // Forward-compat: a future section from a newer build must not brick an
        // older build reading the same home.
        let dir = TempDir::new().unwrap();
        let p = write(
            &dir,
            "[ui]\ntheme = \"dark\"\n\n[harness]\ndefault = \"pi\"\n",
        );
        let cfg = Config::load_from(&p).unwrap();
        assert_eq!(cfg.harness.default.as_deref(), Some("pi"));
    }
}
