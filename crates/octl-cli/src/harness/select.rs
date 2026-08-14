//! Harness selection with **flag > env > config-file > built-in default**
//! precedence, resolved **per run** for `run create --harness`
//! (AGENTS-AI-FIRST-CLI §8).
//!
//! The four layers, highest priority first:
//!
//! 1. **flag** — `--harness <name>` on `run create`.
//! 2. **env** — `ORCHESTRATECTL_HARNESS` (mirrors the flag name per §8). An empty
//!    value counts as unset and falls through.
//! 3. **config file** — `[harness]` in `config.toml`: a `per_kind[<kind>]`
//!    override wins over the section `default`. This is where a repo/user points
//!    autonomous kinds (`research`, `spinoff`) at `pi` while interactive `code`
//!    keeps the global default.
//! 4. **built-in default** — [`super::DEFAULT_HARNESS`] (`claude`), the same for
//!    every kind, so claude stays the default and the interactive driver.
//!
//! The resolved name is validated against [`super::KNOWN_HARNESSES`]; an unknown
//! value is a hard [`CliError`] naming both the offending value and where it came
//! from, so a typo in the flag, the env var, or the config file all fail loudly
//! (and identically) rather than silently launching the wrong agent.

use octl_core::Kind;

use crate::config::Config;
use crate::error::CliError;
use crate::run::kind_kebab;

/// Which precedence layer supplied the resolved harness. Recorded as run
/// provenance (`harness_source` on the `run.created` event) so a caller can
/// reason about *why* a run used the harness it did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HarnessSource {
    /// The `--harness` flag.
    Flag,
    /// The `ORCHESTRATECTL_HARNESS` environment variable.
    Env,
    /// The `config.toml` `[harness]` section (per-kind override or default).
    File,
    /// The built-in default ([`super::DEFAULT_HARNESS`]).
    Default,
}

impl HarnessSource {
    /// Stable lowercase wire spelling for the event log / any future
    /// `config show`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            HarnessSource::Flag => "flag",
            HarnessSource::Env => "env",
            HarnessSource::File => "file",
            HarnessSource::Default => "default",
        }
    }
}

/// A resolved harness selection: the adapter name plus where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessChoice {
    /// The chosen harness name — guaranteed to be one of
    /// [`super::KNOWN_HARNESSES`].
    pub name: String,
    /// The precedence layer that supplied it.
    pub source: HarnessSource,
}

impl HarnessChoice {
    /// The workmux agent to launch for this harness (`None` = workmux default,
    /// i.e. claude). See [`super::workmux_agent`].
    #[must_use]
    pub fn workmux_agent(&self) -> Option<&str> {
        super::workmux_agent(&self.name)
    }
}

/// The environment variable that mirrors `--harness` (§8 flag↔env naming).
pub const HARNESS_ENV: &str = "ORCHESTRATECTL_HARNESS";

/// Resolve the harness for a `run create`, loading the config file and reading
/// the environment. Thin wrapper over [`resolve_with`] that supplies the two
/// ambient inputs; the pure resolver is unit-tested directly.
pub fn resolve(kind: Kind, flag: Option<&str>) -> Result<HarnessChoice, CliError> {
    // The flag is top precedence and must be self-sufficient: a `--harness pi`
    // run never consults config.toml or the env, so we neither read them nor fail
    // on a broken one when the flag is present. This also means an idempotent
    // replay that carries `--harness` is not held hostage to ambient config that
    // drifted after the original run was created. Only the lower layers below
    // touch disk / the environment.
    if let Some(raw) = flag {
        return finish(raw, HarnessSource::Flag);
    }
    let config = Config::load()?;
    let env = std::env::var(HARNESS_ENV).ok();
    resolve_with(kind, None, env.as_deref(), &config)
}

/// The pure precedence resolver: given the explicit flag, the env value, and the
/// loaded config, pick the harness and record its source. Every resolved name is
/// validated; an invalid value at any layer is a [`CliError`] that names the
/// layer.
pub fn resolve_with(
    kind: Kind,
    flag: Option<&str>,
    env: Option<&str>,
    config: &Config,
) -> Result<HarnessChoice, CliError> {
    // 1. flag
    if let Some(raw) = flag {
        return finish(raw, HarnessSource::Flag);
    }
    // 2. env (empty string counts as unset — a common shell footgun that must not
    //    override the config/default with an invalid empty value).
    if let Some(raw) = env {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return finish(trimmed, HarnessSource::Env);
        }
    }
    // 3. config file: per-kind override first, then the section default.
    if let Some(raw) = config.harness.per_kind.get(kind_kebab(kind)) {
        return finish(raw, HarnessSource::File);
    }
    if let Some(raw) = config.harness.default.as_deref() {
        return finish(raw, HarnessSource::File);
    }
    // 4. built-in default. Always valid by construction, but validate anyway so a
    //    future edit to DEFAULT_HARNESS that drops it from KNOWN_HARNESSES fails a
    //    test rather than shipping.
    finish(super::DEFAULT_HARNESS, HarnessSource::Default)
}

/// Validate `raw` against [`super::KNOWN_HARNESSES`] and wrap it with its source,
/// or fail with a structured error that names both the bad value and the layer.
fn finish(raw: &str, source: HarnessSource) -> Result<HarnessChoice, CliError> {
    let name = raw.trim();
    // An empty value (e.g. `--harness ""`, or `ORCHESTRATECTL_HARNESS=` reaching
    // the flag path) deserves a clear message rather than the confusing
    // "unknown harness ''". (An empty env value is already treated as unset one
    // layer up in `resolve_with`; this catches the explicit-empty cases.)
    if name.is_empty() {
        return Err(CliError::user(
            "invalid_harness",
            format!(
                "empty harness name (from {}); known harnesses: {}",
                source.as_str(),
                super::KNOWN_HARNESSES.join(", ")
            ),
        )
        .with_expected(serde_json::json!(super::KNOWN_HARNESSES)));
    }
    if super::KNOWN_HARNESSES.contains(&name) {
        return Ok(HarnessChoice {
            name: name.to_string(),
            source,
        });
    }
    let expected = serde_json::json!(super::KNOWN_HARNESSES);
    Err(CliError::user(
        "invalid_harness",
        format!(
            "unknown harness '{name}' (from {}); known harnesses: {}",
            source.as_str(),
            super::KNOWN_HARNESSES.join(", ")
        ),
    )
    .with_invalid_value(name)
    .with_expected(expected))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn cfg(default: Option<&str>, per_kind: &[(&str, &str)]) -> Config {
        Config {
            harness: crate::config::HarnessConfig {
                default: default.map(str::to_string),
                per_kind: per_kind
                    .iter()
                    .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                    .collect::<BTreeMap<_, _>>(),
            },
        }
    }

    #[test]
    fn default_when_nothing_set() {
        let got = resolve_with(Kind::Spinoff, None, None, &Config::default()).unwrap();
        assert_eq!(got.name, "claude");
        assert_eq!(got.source, HarnessSource::Default);
    }

    #[test]
    fn flag_wins_over_everything() {
        let c = cfg(Some("pi"), &[("spinoff", "pi")]);
        let got = resolve_with(Kind::Spinoff, Some("pi"), Some("claude"), &c).unwrap();
        assert_eq!(got.name, "pi");
        assert_eq!(got.source, HarnessSource::Flag);
    }

    #[test]
    fn env_wins_over_config_and_default() {
        let c = cfg(Some("claude"), &[]);
        let got = resolve_with(Kind::Research, None, Some("pi"), &c).unwrap();
        assert_eq!(got.name, "pi");
        assert_eq!(got.source, HarnessSource::Env);
    }

    #[test]
    fn empty_env_is_ignored() {
        let c = cfg(Some("pi"), &[]);
        let got = resolve_with(Kind::Research, None, Some("   "), &c).unwrap();
        assert_eq!(got.name, "pi");
        assert_eq!(got.source, HarnessSource::File);
    }

    #[test]
    fn per_kind_beats_config_default() {
        let c = cfg(Some("claude"), &[("research", "pi")]);
        let research = resolve_with(Kind::Research, None, None, &c).unwrap();
        assert_eq!(research.name, "pi");
        assert_eq!(research.source, HarnessSource::File);
        // A kind with no per-kind entry falls back to the section default.
        let other = resolve_with(Kind::Spinoff, None, None, &c).unwrap();
        assert_eq!(other.name, "claude");
        assert_eq!(other.source, HarnessSource::File);
    }

    #[test]
    fn config_default_used_when_no_per_kind() {
        let c = cfg(Some("pi"), &[]);
        let got = resolve_with(Kind::Spinoff, None, None, &c).unwrap();
        assert_eq!(got.name, "pi");
        assert_eq!(got.source, HarnessSource::File);
    }

    #[test]
    fn invalid_flag_is_rejected() {
        let e = resolve_with(Kind::Spinoff, Some("gpt"), None, &Config::default()).unwrap_err();
        assert_eq!(e.code, "invalid_harness");
        assert_eq!(e.invalid_value.as_deref(), Some("gpt"));
        assert!(e.message.contains("from flag"));
    }

    #[test]
    fn invalid_env_names_the_layer() {
        let e = resolve_with(Kind::Spinoff, None, Some("gpt"), &Config::default()).unwrap_err();
        assert_eq!(e.code, "invalid_harness");
        assert!(e.message.contains("from env"), "message: {}", e.message);
    }

    #[test]
    fn invalid_config_value_names_the_layer() {
        let c = cfg(Some("gpt"), &[]);
        let e = resolve_with(Kind::Spinoff, None, None, &c).unwrap_err();
        assert_eq!(e.code, "invalid_harness");
        assert!(e.message.contains("from file"), "message: {}", e.message);
    }

    #[test]
    fn invalid_per_kind_config_value_names_the_layer() {
        // A bad harness in a per-kind override is rejected the same way as the
        // section default, and still attributed to the file layer.
        let c = cfg(None, &[("spinoff", "gpt")]);
        let e = resolve_with(Kind::Spinoff, None, None, &c).unwrap_err();
        assert_eq!(e.code, "invalid_harness");
        assert_eq!(e.invalid_value.as_deref(), Some("gpt"));
        assert!(e.message.contains("from file"), "message: {}", e.message);
    }

    #[test]
    fn empty_flag_is_a_clear_error() {
        // `--harness ""` / `--harness "  "` gets a specific message, not the
        // confusing `unknown harness ''`.
        let e = resolve_with(Kind::Spinoff, Some("  "), None, &Config::default()).unwrap_err();
        assert_eq!(e.code, "invalid_harness");
        assert!(
            e.message.contains("empty harness name"),
            "message: {}",
            e.message
        );
    }

    #[test]
    fn workmux_agent_maps_claude_to_none() {
        let claude = HarnessChoice {
            name: "claude".into(),
            source: HarnessSource::Default,
        };
        assert_eq!(claude.workmux_agent(), None);
        let pi = HarnessChoice {
            name: "pi".into(),
            source: HarnessSource::Flag,
        };
        assert_eq!(pi.workmux_agent(), Some("pi"));
    }
}
