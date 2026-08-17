//! Integration tests for the `config` subcommand (AGENTS-AI-FIRST-CLI §8).
//!
//! `config path` prints the config file location; `config show` prints the
//! effective resolved config with a per-key `source` (`env | file | default`).
//! Both are read-only and never mutate the file. These tests pin the JSON
//! payload shape (a versioned API surface) and the four precedence scenarios
//! the issue calls out: default, file-provided harness, env override, and a
//! missing config file.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use serde_json::Value;
use tempfile::TempDir;

/// A `config`-command invocation in a hermetic home. `ORCHESTRATECTL_HOME`
/// points at a fresh tempdir and the harness env var is cleared, so a
/// developer's shell can never perturb the resolved config.
fn bin(home: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_orchestratectl"));
    c.env("ORCHESTRATECTL_HOME", home);
    c.env_remove("ORCHESTRATECTL_HARNESS");
    c.env_remove("ORCHESTRATECTL_LOG");
    c
}

/// Run to success (exit 0), assert clean stderr, and parse stdout as one JSON
/// envelope. Returns the `data` payload.
fn show_json(cmd: &mut Command) -> Value {
    let out = cmd.output().expect("spawn");
    assert!(out.status.success(), "exit: {:?}", out.status);
    assert!(
        out.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).expect("utf8");
    let v: Value = serde_json::from_str(stdout.trim()).expect("valid JSON envelope");
    assert_eq!(v["schema_version"], 1, "envelope schema: {v}");
    assert_eq!(v["warnings"], serde_json::json!([]));
    v["data"].clone()
}

/// The `harness.<kind>` / `harness.default` row for `key`.
fn key<'a>(data: &'a Value, key: &str) -> &'a Value {
    data["keys"]
        .as_array()
        .expect("keys array")
        .iter()
        .find(|k| k["key"] == key)
        .unwrap_or_else(|| panic!("key {key} present in {data}"))
}

// ----------------------------------------------------------------------
// config path
// ----------------------------------------------------------------------

#[test]
fn config_path_text_prints_bare_path() {
    let home = TempDir::new().unwrap();
    let out = bin(home.path())
        .args(["--output", "text", "config", "path"])
        .output()
        .expect("spawn");
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).expect("utf8");
    let expected = home.path().join("config.toml");
    assert_eq!(stdout.trim(), expected.display().to_string());
}

#[test]
fn config_path_json_pins_payload_shape() {
    let home = TempDir::new().unwrap();
    let data = show_json(bin(home.path()).args(["config", "path", "--output", "json"]));
    let keys: BTreeSet<&str> = data
        .as_object()
        .expect("data object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        BTreeSet::from(["schema_version_config", "path", "exists"]),
        "unexpected config path keys: {keys:?}"
    );
    assert_eq!(data["schema_version_config"], 1);
    assert_eq!(data["exists"], false, "no file written yet");
    assert_eq!(
        data["path"],
        home.path().join("config.toml").display().to_string()
    );
}

#[test]
fn config_path_reports_exists_true_when_present() {
    let home = TempDir::new().unwrap();
    std::fs::write(
        home.path().join("config.toml"),
        "[harness]\ndefault = \"pi\"\n",
    )
    .unwrap();
    let data = show_json(bin(home.path()).args(["config", "path", "--output", "json"]));
    assert_eq!(data["exists"], true);
}

// ----------------------------------------------------------------------
// config show
// ----------------------------------------------------------------------

#[test]
fn config_show_json_pins_payload_and_default_sources() {
    // No config file → every harness key resolves to the built-in default.
    let home = TempDir::new().unwrap();
    let data = show_json(bin(home.path()).args(["config", "show", "--output", "json"]));

    let keys: BTreeSet<&str> = data
        .as_object()
        .expect("data object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        BTreeSet::from(["schema_version_config", "path", "exists", "keys"]),
        "unexpected config show keys: {keys:?}"
    );
    assert_eq!(data["schema_version_config"], 1);
    assert_eq!(data["exists"], false);

    // Exactly the section default + one row per creatable kind.
    let key_names: Vec<&str> = data["keys"]
        .as_array()
        .expect("keys array")
        .iter()
        .map(|k| k["key"].as_str().expect("key string"))
        .collect();
    assert_eq!(
        key_names,
        vec![
            "harness.default",
            "harness.spinoff",
            "harness.research",
            "harness.technical-decision",
            "harness.fan-out",
        ],
    );

    // Every key pins the full row shape and the default source.
    for k in data["keys"].as_array().unwrap() {
        let row_keys: BTreeSet<&str> = k
            .as_object()
            .expect("row object")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            row_keys,
            BTreeSet::from(["key", "value", "source", "secret"]),
            "unexpected row keys: {row_keys:?}"
        );
        assert_eq!(k["value"], "pi");
        assert_eq!(k["source"], "default");
        assert_eq!(k["secret"], false);
    }
}

#[test]
fn config_show_file_provided_harness() {
    // A `[harness]` default plus a per-kind override are both attributed to the
    // `file` layer, and the override wins for its kind.
    let home = TempDir::new().unwrap();
    std::fs::write(
        home.path().join("config.toml"),
        "[harness]\ndefault = \"pi\"\n\n[harness.per_kind]\nresearch = \"claude\"\n",
    )
    .unwrap();
    let data = show_json(bin(home.path()).args(["config", "show", "--output", "json"]));
    assert_eq!(data["exists"], true);

    assert_eq!(key(&data, "harness.default")["value"], "pi");
    assert_eq!(key(&data, "harness.default")["source"], "file");
    // per-kind override shadows the section default for `research` only.
    assert_eq!(key(&data, "harness.research")["value"], "claude");
    assert_eq!(key(&data, "harness.research")["source"], "file");
    // a kind with no override inherits the file default.
    assert_eq!(key(&data, "harness.spinoff")["value"], "pi");
    assert_eq!(key(&data, "harness.spinoff")["source"], "file");
}

#[test]
fn config_show_env_override_wins() {
    // `ORCHESTRATECTL_HARNESS` shadows the file layers entirely: every row
    // reports the env value with `source: "env"` — the honest effective
    // picture, not the shadowed file value.
    let home = TempDir::new().unwrap();
    std::fs::write(
        home.path().join("config.toml"),
        "[harness]\ndefault = \"claude\"\n\n[harness.per_kind]\nresearch = \"claude\"\n",
    )
    .unwrap();
    let mut cmd = bin(home.path());
    cmd.env("ORCHESTRATECTL_HARNESS", "pi");
    let data = show_json(cmd.args(["config", "show", "--output", "json"]));

    for k in data["keys"].as_array().unwrap() {
        assert_eq!(k["value"], "pi", "row: {k}");
        assert_eq!(k["source"], "env", "row: {k}");
    }
}

#[test]
fn config_show_missing_file_is_all_defaults() {
    // Missing config file is not an error — it resolves to defaults with
    // `exists: false`.
    let home = TempDir::new().unwrap();
    let data = show_json(bin(home.path()).args(["config", "show", "--output", "json"]));
    assert_eq!(data["exists"], false);
    assert_eq!(key(&data, "harness.default")["source"], "default");
}

#[test]
fn config_show_invalid_config_file_is_a_hard_error() {
    // A bad harness value in the file fails loudly (§1 strict validation),
    // naming the layer — `config show` must not silently launder it.
    let home = TempDir::new().unwrap();
    std::fs::write(
        home.path().join("config.toml"),
        "[harness]\ndefault = \"gpt\"\n",
    )
    .unwrap();
    let out = bin(home.path())
        .args(["config", "show", "--output", "json"])
        .output()
        .expect("spawn");
    assert!(!out.status.success(), "expected non-zero exit");
    assert!(out.stdout.is_empty(), "errors must not write stdout");
    let stderr = String::from_utf8(out.stderr).expect("utf8");
    let last = stderr.lines().last().expect("stderr non-empty");
    let v: Value = serde_json::from_str(last).expect("error envelope is valid JSON");
    assert_eq!(v["error"]["code"], "invalid_harness");
}

#[test]
fn config_show_text_is_human_readable() {
    let home = TempDir::new().unwrap();
    let out = bin(home.path())
        .args(["--output", "text", "config", "show"])
        .output()
        .expect("spawn");
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).expect("utf8");
    assert!(stdout.contains("path:"), "text output: {stdout:?}");
    assert!(stdout.contains("exists: false"), "text output: {stdout:?}");
    assert!(
        stdout.contains("harness.default"),
        "text output: {stdout:?}"
    );
    // The value column stays aligned even for the widest key
    // (`harness.technical-decision`, 26 chars > any fixed pad): every harness
    // row's value must start at the same column.
    let value_cols: Vec<usize> = stdout
        .lines()
        .filter(|l| l.starts_with("harness."))
        .map(|l| {
            let key_end = l.find(' ').expect("key/value separator");
            l[key_end..].len() - l[key_end..].trim_start().len() + key_end
        })
        .collect();
    assert!(
        value_cols.len() >= 5,
        "expected all harness rows: {stdout:?}"
    );
    assert!(
        value_cols.windows(2).all(|w| w[0] == w[1]),
        "value column not aligned across rows: {stdout:?}"
    );
}

#[test]
fn config_show_secrets_flag_is_accepted_noop() {
    // No config key is secret today, so `--show-secrets` reveals nothing and
    // emits no warning — but the flag must parse and behave identically.
    let home = TempDir::new().unwrap();
    let out = bin(home.path())
        .args(["config", "show", "--show-secrets", "--output", "json"])
        .output()
        .expect("spawn");
    assert!(out.status.success());
    assert!(
        out.stderr.is_empty(),
        "no secret keys → no warning: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
