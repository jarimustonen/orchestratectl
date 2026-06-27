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

use clap::parser::ValueSource;
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
    /// Whether the command is hidden from the human text help. Hidden
    /// commands are still listed (agents may invoke them) but flagged.
    pub hidden: bool,
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

/// Synthetic id for the global help flag injected into the lenient-parse
/// clone (see [`resolve_help_request`]). Double-underscore prefix keeps it
/// clear of any real arg id.
const HELP_FLAG_ID: &str = "__octl_help_request";

/// Id of the global `--output` arg, read back from the lenient parse.
const OUTPUT_ARG_ID: &str = "output";

/// Outcome of inspecting raw argv for a structured-help request.
pub enum HelpRequest {
    /// Not a JSON help request — the caller falls through to clap's normal
    /// dispatch. Covers no `--help`, a bare `--help` (no explicit
    /// `--output`), and `--output text`, all of which keep clap's text
    /// rendering (§14: bare `--help` is unchanged).
    None,
    /// Structured help requested for the resolved subcommand path
    /// (canonical subcommand names, root excluded).
    Render { spec: OutputSpec, path: Vec<String> },
    /// Structured help requested, but a token in subcommand position is not
    /// a known subcommand. The caller emits an error envelope (exit 1)
    /// rather than falling back to root help.
    UnknownSubcommand { token: String },
}

/// Resolve a structured-help request from raw argv (excluding argv\[0\])
/// via a clap *lenient* parse.
///
/// This replaces the former hand-rolled argv scan. `root` is the real
/// (canonical) command tree; it is cloned and reconfigured so a single
/// tolerant parse recovers the subcommand path **and** the `--output`
/// value exactly as clap would — robust against value-taking flags at any
/// level, short-flag clusters (`-vh`), non-canonical aliases, and `--`
/// (handled by clap for free).
///
/// Returns [`HelpRequest::Render`] only when **both** an explicit help flag
/// (`--help`/`-h`) and an explicit non-text `--output` are present; a bare
/// `--help` or `--output text` returns [`HelpRequest::None`]. An unknown
/// subcommand returns [`HelpRequest::UnknownSubcommand`].
#[must_use]
pub fn resolve_help_request(root: &Command, args: &[String]) -> HelpRequest {
    let mut lenient = root
        .clone()
        // Tolerate unknown flags/values so the path and `--output` resolve
        // even when `--help` sits where clap would normally reject it.
        .ignore_errors(true)
        // Suppress clap's built-in `--help` (its action short-circuits to
        // text); we detect help via the injected flag below instead.
        .disable_help_flag(true)
        // Surface an unknown subcommand as an external subcommand in the
        // matches rather than silently dropping it, so we can error on it.
        .allow_external_subcommands(true)
        .arg(
            Arg::new(HELP_FLAG_ID)
                .long("help")
                .short('h')
                .action(ArgAction::SetTrue)
                .global(true),
        );

    // clap expects argv[0] to be the program name.
    let with_prog = std::iter::once(root.get_name().to_string()).chain(args.iter().cloned());
    let Ok(matches) = lenient.try_get_matches_from_mut(with_prog) else {
        // `ignore_errors` makes a hard error practically unreachable; if one
        // slips through we simply decline the request.
        return HelpRequest::None;
    };

    // A help request requires an explicit `--help`/`-h`.
    if !matches.get_flag(HELP_FLAG_ID) {
        return HelpRequest::None;
    }

    // ...and an explicit non-text `--output`. The `jsonl` default does not
    // count (`value_source` distinguishes it), so a bare `--help` keeps
    // clap's text rendering.
    let spec = match matches.value_source(OUTPUT_ARG_ID) {
        Some(ValueSource::CommandLine) => matches.get_one::<OutputSpec>(OUTPUT_ARG_ID).cloned(),
        _ => None,
    };
    let Some(spec) = spec else {
        return HelpRequest::None;
    };
    if spec.format == OutputFormat::Text {
        return HelpRequest::None;
    }

    // Walk the resolved subcommand path, validating each name against the
    // real tree. With `allow_external_subcommands`, an unknown token in
    // subcommand position surfaces here as a subcommand whose name the real
    // tree does not know — that is the unknown-subcommand signal.
    let mut cur = root;
    let mut path = Vec::new();
    let mut node = &matches;
    while let Some((name, sub)) = node.subcommand() {
        match cur.find_subcommand(name) {
            Some(child) => {
                cur = child;
                path.push(name.to_string());
                node = sub;
            }
            None => {
                return HelpRequest::UnknownSubcommand {
                    token: name.to_string(),
                }
            }
        }
    }

    HelpRequest::Render { spec, path }
}

/// Walk a canonical subcommand-name path (root excluded) through a built
/// command tree, returning the deepest command and its full invocation
/// path. The names come from [`resolve_help_request`]'s clap parse, so
/// every lookup succeeds; an unknown name simply stops the walk (defensive,
/// not expected).
#[must_use]
pub fn navigate_path<'a>(root: &'a Command, names: &[String]) -> (&'a Command, String) {
    let mut cur = root;
    let mut path = vec![root.get_name().to_string()];
    for name in names {
        let Some(sc) = cur.find_subcommand(name) else {
            break;
        };
        cur = sc;
        path.push(sc.get_name().to_string());
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

    let about = cmd.get_about().map(ToString::to_string);
    // Only surface `long_about` when it adds something over `about`; clap
    // returns the same text for both when only `about` was set.
    let long_about = cmd
        .get_long_about()
        .map(ToString::to_string)
        .filter(|l| Some(l) != about.as_ref());

    CommandNode {
        command: command_path.to_string(),
        about,
        long_about,
        aliases: cmd.get_visible_aliases().map(ToString::to_string).collect(),
        hidden: cmd.is_hide_set(),
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
        // `build()` assigns every positional a 1-based index before we
        // walk, so this is always `Some`. Emitting a bogus `0` instead
        // would silently mis-sort and lie about the position.
        index: arg
            .get_index()
            .expect("positional has an index after Command::build()"),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A miniature command tree mirroring the real one's shape: a global
    /// `--output` (custom `OutputSpec` parser, `jsonl` default) plus a noun
    /// (`run`) with a leaf verb (`create`).
    fn test_root() -> Command {
        Command::new("tool")
            .arg(
                Arg::new(OUTPUT_ARG_ID)
                    .long("output")
                    .global(true)
                    .default_value("jsonl")
                    .value_parser(crate::output::parse_output_value),
            )
            .subcommand(Command::new("run").subcommand(Command::new("create")))
    }

    fn args(parts: &[&str]) -> Vec<String> {
        parts.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn render_resolves_leaf_path_and_output() {
        let req = resolve_help_request(
            &test_root(),
            &args(&["run", "create", "--help", "--output", "json"]),
        );
        match req {
            HelpRequest::Render { spec, path } => {
                assert_eq!(spec.format, OutputFormat::Json);
                assert_eq!(path, vec!["run".to_string(), "create".to_string()]);
            }
            _ => panic!("expected Render"),
        }
    }

    #[test]
    fn output_can_precede_the_subcommand_path() {
        // A robustness win over the old scan: `--output` resolves wherever
        // it sits, including before any subcommand.
        let req =
            resolve_help_request(&test_root(), &args(&["--output", "jsonl", "run", "--help"]));
        match req {
            HelpRequest::Render { spec, path } => {
                assert_eq!(spec.format, OutputFormat::Jsonl);
                assert_eq!(path, vec!["run".to_string()]);
            }
            _ => panic!("expected Render"),
        }
    }

    #[test]
    fn bare_help_without_output_is_none() {
        // Default `--output jsonl` must NOT count — clap's text help stands.
        assert!(matches!(
            resolve_help_request(&test_root(), &args(&["run", "create", "--help"])),
            HelpRequest::None
        ));
    }

    #[test]
    fn output_text_with_help_is_none() {
        assert!(matches!(
            resolve_help_request(&test_root(), &args(&["run", "--help", "--output", "text"])),
            HelpRequest::None
        ));
    }

    #[test]
    fn no_help_flag_is_none() {
        assert!(matches!(
            resolve_help_request(&test_root(), &args(&["run", "--output", "json"])),
            HelpRequest::None
        ));
    }

    #[test]
    fn double_dash_suppresses_detection() {
        // After `--`, a trailing `--help` is positional data, not a request.
        assert!(matches!(
            resolve_help_request(
                &test_root(),
                &args(&["run", "--", "--help", "--output", "json"])
            ),
            HelpRequest::None
        ));
    }

    #[test]
    fn unknown_subcommand_after_flags_is_flagged() {
        // Flag-first ordering surfaces the bad token as an external
        // subcommand the real tree rejects.
        match resolve_help_request(
            &test_root(),
            &args(&["--help", "--output", "json", "bogus"]),
        ) {
            HelpRequest::UnknownSubcommand { token } => assert_eq!(token, "bogus"),
            other => panic!("expected UnknownSubcommand, got {:?}", DebugReq(&other)),
        }
    }

    // Tiny helper so the panic message above can name the variant without a
    // `Debug` impl on `HelpRequest` (which would otherwise be dead weight).
    struct DebugReq<'a>(&'a HelpRequest);
    impl std::fmt::Debug for DebugReq<'_> {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self.0 {
                HelpRequest::None => write!(f, "None"),
                HelpRequest::Render { path, .. } => write!(f, "Render({path:?})"),
                HelpRequest::UnknownSubcommand { token } => write!(f, "Unknown({token})"),
            }
        }
    }
}
