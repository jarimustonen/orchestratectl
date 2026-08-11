//! User-facing configuration file — the `file` layer of the
//! flag > env > file > default precedence (AGENTS-AI-FIRST-CLI §8).
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
//! default = "claude"
//!
//! [harness.per_kind]
//! # Per-kind overrides, keyed by the kebab-case run kind. Lets a repo default
//! # autonomous work to pi while interactive `code` stays on the global default.
//! research = "pi"
//! spinoff = "pi"
//! ```
//!
//! Parsing is strict: a syntactically invalid `config.toml` is a hard
//! [`CliError`] (a silently ignored config is worse than a loud one — the caller
//! would reason about a value the file was supposed to set). Unknown keys are
//! rejected (`deny_unknown_fields`) so a typo'd setting fails loudly rather than
//! silently doing nothing.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Deserialize;

use crate::error::CliError;

/// The parsed `config.toml`. All sections optional; an absent section leaves the
/// corresponding settings at their built-in defaults.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
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
    /// `None` (key absent) falls through to the built-in default (`claude`).
    #[serde(default)]
    pub default: Option<String>,
    /// Per-kind overrides, keyed by kebab-case run kind (`research`, `spinoff`,
    /// …). A `BTreeMap` keeps the parse deterministic; keys are validated as run
    /// kinds only lazily, when a matching run is created (an unrelated key never
    /// blocks an unrelated run).
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
        toml::from_str(&text).map_err(|e| {
            CliError::user(
                "invalid_config",
                format!("config file {} is not valid: {e}", path.display()),
            )
        })
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
code = "claude"
"#,
        );
        let cfg = Config::load_from(&p).unwrap();
        assert_eq!(cfg.harness.default.as_deref(), Some("pi"));
        assert_eq!(
            cfg.harness.per_kind.get("research").map(String::as_str),
            Some("pi")
        );
        assert_eq!(
            cfg.harness.per_kind.get("code").map(String::as_str),
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
}
