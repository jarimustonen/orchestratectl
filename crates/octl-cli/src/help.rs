//! Structured (`--output json|jsonl`) rendering of the `--help` surface.
//!
//! clap's derive ships only a *text* `--help`. AGENTS-AI-FIRST-CLI §14
//! additionally requires every `<tool> ... --help` to emit a
//! machine-readable description of its command surface — subcommands,
//! flags, positionals, defaults, env-var mappings, accepted-value enums,
//! and deprecation status — carrying its own `schema_version`.
//!
//! This module walks the built [`clap::Command`] tree (clap exposes the
//! full metadata via `get_*` / `is_*_set` getters) and projects it onto
//! the serde structs below. The caller in [`crate::cli`] feeds the result
//! through the standard [`crate::output::emit_envelope`] so the help
//! payload rides the same `{schema_version, data}` envelope, format
//! selection (`json` pretty vs `jsonl` single-line), and file routing as
//! every other command.
//!
//! Stability contract (snapshot-tested): flags are sorted by long name,
//! positionals by index, subcommands by name. Field renames/removals are
//! breaking changes — bump `SCHEMA_VERSION_HELP`.

use clap::{Arg, ArgAction, Command};
use serde::Serialize;

use crate::output::{OutputFormat, OutputSpec};

/// Schema version of the help payload itself (independent of the
/// envelope's `schema_version` and of the state-schema version). Bump on
/// any breaking change to the shapes in this module.
pub const SCHEMA_VERSION_HELP: u32 = 1;

/// The `data` body of a `--help --output json|jsonl` response.
#[derive(Debug, Serialize)]
pub struct HelpData {
    /// Version of the help payload schema (this module's shapes).
    pub schema_version_help: u32,
    /// The command surface, starting at the resolved command.
    #[serde(flatten)]
    pub command: CommandNode,
}

/// One node in the command tree — recursive: each subcommand is rendered
/// as a full `CommandNode` so the whole surface is queryable in one call.
#[derive(Debug, Serialize)]
pub struct CommandNode {
    /// Full invocation path, e.g. `orchestratectl run create`.
    pub command: String,
    /// One-line description (`about`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub about: Option<String>,
    /// Long description (`long_about`), when distinct from `about`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub long_about: Option<String>,
    /// Visible aliases for this command.
    pub aliases: Vec<String>,
    /// Command version, when one is set (typically only the root).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Optional (named) flags, sorted by long name.
    pub flags: Vec<FlagInfo>,
    /// Positional arguments, sorted by position.
    pub positionals: Vec<PositionalInfo>,
    /// Child subcommands, sorted by name.
    pub subcommands: Vec<CommandNode>,
}

/// A named (`--long`) flag.
///
/// The several `bool` fields each mirror an independent piece of clap
/// metadata (takes-value / multiple / required / hidden / deprecated);
/// collapsing them into an enum would lose the orthogonality and the
/// stable JSON field names agents read, so the pedantic
/// `struct_excessive_bools` lint is allowed here.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Serialize)]
pub struct FlagInfo {
    /// Long name without the leading `--`.
    pub long: String,
    /// Short name without the leading `-`, when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub short: Option<String>,
    /// One-line help text, when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    /// Value-name placeholders shown in usage (empty for boolean flags).
    pub value_names: Vec<String>,
    /// Whether the flag consumes a value.
    pub takes_value: bool,
    /// Whether the flag may be repeated / accept multiple values.
    pub multiple: bool,
    /// Whether the flag is required.
    pub required: bool,
    /// Whether the flag is hidden from the human text help.
    pub hidden: bool,
    /// Deprecation status. clap exposes no first-class deprecation
    /// metadata, so this is a reserved field that is always `false` until
    /// a deprecation convention is adopted (see issue handoff note). It is
    /// emitted unconditionally so agents can rely on its presence.
    pub deprecated: bool,
    /// Default value(s) applied when the flag is omitted.
    pub defaults: Vec<String>,
    /// Accepted values (the enum) when the flag is value-restricted.
    pub accepted_values: Vec<String>,
    /// Environment variable that supplies this flag (per §8), when mapped.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<String>,
}

/// A positional argument.
#[derive(Debug, Serialize)]
pub struct PositionalInfo {
    /// Argument id.
    pub name: String,
    /// One-line help text, when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    /// Value-name placeholders shown in usage.
    pub value_names: Vec<String>,
    /// 1-based position.
    pub index: usize,
    /// Whether the argument is required.
    pub required: bool,
    /// Whether the argument accepts multiple values.
    pub multiple: bool,
    /// Accepted values (the enum) when the argument is value-restricted.
    pub accepted_values: Vec<String>,
}

/// Detect a structured-help request from raw argv (excluding argv\[0\]).
///
/// Returns the resolved [`OutputSpec`] only when **both** a help flag
/// (`--help` / `-h`) and an explicit non-text `--output json|jsonl`
/// (or a `.json`/`.jsonl` file destination) are present. A bare `--help`
/// (no `--output`) or `--output text` returns `None`, preserving clap's
/// default text rendering (§14 out-of-scope: bare `--help` is unchanged).
pub fn detect_json_help_request(args: &[String]) -> Option<OutputSpec> {
    let mut help = false;
    let mut spec: Option<OutputSpec> = None;
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "--help" || a == "-h" {
            help = true;
            i += 1;
        } else if a == "--output" {
            if let Some(v) = args.get(i + 1) {
                spec = crate::output::parse_output_value(v).ok();
            }
            i += 2;
        } else if let Some(v) = a.strip_prefix("--output=") {
            spec = crate::output::parse_output_value(v).ok();
            i += 1;
        } else {
            i += 1;
        }
    }
    match (help, spec) {
        (true, Some(s)) if s.format != OutputFormat::Text => Some(s),
        _ => None,
    }
}

/// Walk the subcommand path encoded in raw argv and return the deepest
/// matched [`Command`] together with its full invocation path.
///
/// Flags (and the value of `--output`) are skipped; the first token that
/// is not a known subcommand (e.g. a positional value) ends the walk. This
/// mirrors clap's own drill-down: `run create --help` resolves to the
/// `create` leaf regardless of where `--help`/`--output` sit in argv.
pub fn navigate<'a>(root: &'a Command, args: &[String]) -> (&'a Command, String) {
    let mut cur = root;
    let mut path = vec![root.get_name().to_string()];
    let mut i = 0;
    while i < args.len() {
        let tok = &args[i];
        if tok == "--output" {
            i += 2; // skip the flag and its space-separated value
            continue;
        }
        if tok.starts_with('-') {
            // any other flag (--output=…, --help, -h, …): not a subcommand
            i += 1;
            continue;
        }
        match cur.find_subcommand(tok) {
            Some(sc) => {
                cur = sc;
                path.push(sc.get_name().to_string());
                i += 1;
            }
            // A non-flag token that is not a subcommand is a positional
            // value (e.g. a run id); the command is fully resolved.
            None => break,
        }
    }
    (cur, path.join(" "))
}

/// Project a built [`Command`] (and its descendants) onto [`HelpData`].
///
/// `command_path` is the full invocation path of `cmd` (from
/// [`navigate`]); child paths are derived by appending the child name.
#[must_use]
pub fn build_help(cmd: &Command, command_path: &str) -> HelpData {
    HelpData {
        schema_version_help: SCHEMA_VERSION_HELP,
        command: build_node(cmd, command_path),
    }
}

fn build_node(cmd: &Command, command_path: &str) -> CommandNode {
    let mut flags: Vec<FlagInfo> = cmd
        .get_arguments()
        .filter(|a| !a.is_positional())
        .filter_map(build_flag)
        .collect();
    flags.sort_by(|a, b| a.long.cmp(&b.long));

    let mut positionals: Vec<PositionalInfo> =
        cmd.get_positionals().map(build_positional).collect();
    positionals.sort_by_key(|p| p.index);

    let mut subcommands: Vec<CommandNode> = cmd
        .get_subcommands()
        .map(|sc| {
            let child_path = format!("{command_path} {}", sc.get_name());
            build_node(sc, &child_path)
        })
        .collect();
    subcommands.sort_by(|a, b| a.command.cmp(&b.command));

    CommandNode {
        command: command_path.to_string(),
        about: cmd.get_about().map(ToString::to_string),
        long_about: cmd.get_long_about().map(ToString::to_string),
        aliases: cmd.get_visible_aliases().map(ToString::to_string).collect(),
        version: cmd.get_version().map(ToString::to_string),
        flags,
        positionals,
        subcommands,
    }
}

/// Build a [`FlagInfo`], or `None` for flags without a stable long name
/// (per issue default: such flags are skipped from JSON output).
fn build_flag(arg: &Arg) -> Option<FlagInfo> {
    let long = arg.get_long()?.to_string();
    let action = arg.get_action();
    let takes_value = takes_value(action);
    Some(FlagInfo {
        long,
        short: arg.get_short().map(|c| c.to_string()),
        help: arg.get_help().map(ToString::to_string),
        // Boolean flags carry a derived value-name placeholder in clap;
        // suppress it so agents don't read a value where none is taken.
        value_names: if takes_value {
            value_names(arg)
        } else {
            Vec::new()
        },
        takes_value,
        multiple: multiple(arg, action),
        required: arg.is_required_set(),
        hidden: arg.is_hide_set(),
        deprecated: false,
        defaults: default_values(arg),
        accepted_values: accepted_values(arg),
        env: arg.get_env().map(|e| e.to_string_lossy().into_owned()),
    })
}

fn build_positional(arg: &Arg) -> PositionalInfo {
    PositionalInfo {
        name: arg.get_id().as_str().to_string(),
        help: arg.get_help().map(ToString::to_string),
        value_names: value_names(arg),
        index: arg.get_index().unwrap_or(0),
        required: arg.is_required_set(),
        multiple: multiple(arg, arg.get_action()),
        accepted_values: accepted_values(arg),
    }
}

fn value_names(arg: &Arg) -> Vec<String> {
    arg.get_value_names()
        .unwrap_or_default()
        .iter()
        .map(ToString::to_string)
        .collect()
}

fn default_values(arg: &Arg) -> Vec<String> {
    arg.get_default_values()
        .iter()
        .map(|v| v.to_string_lossy().into_owned())
        .collect()
}

/// Accepted (enum) values, excluding any the author hid from help.
fn accepted_values(arg: &Arg) -> Vec<String> {
    arg.get_possible_values()
        .into_iter()
        .filter(|p| !p.is_hide_set())
        .map(|p| p.get_name().to_string())
        .collect()
}

/// Whether an arg consumes a value, derived from its action. `Help` /
/// `Version` / `SetTrue` / `SetFalse` / `Count` are valueless; `Set` /
/// `Append` carry a value. (Positionals always take a value and are
/// handled via this same mapping — their action is `Set`/`Append`.)
fn takes_value(action: &ArgAction) -> bool {
    matches!(action, ArgAction::Set | ArgAction::Append)
}

/// Whether an arg may repeat / accept multiple values: `Append` and
/// `Count` are inherently repeatable; otherwise consult the value count.
fn multiple(arg: &Arg, action: &ArgAction) -> bool {
    matches!(action, ArgAction::Append | ArgAction::Count)
        || arg.get_num_args().is_some_and(|r| r.max_values() > 1)
}
