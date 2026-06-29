//! `run create` — top-level + child-spawn run initialization.
//!
//! Top-level: initializes the run dir, shells out to create.sh to
//! materialize worktree + tmux window + agent, emits `node.created`
//! with the discovered agent PID, and spawns the supervisor.
//!
//! Child-spawn (`--parent-run-id` + `--parent-node-id`): emits
//! `child.spawned` on the parent's events (per design.md §7.2 step 3),
//! initializes the child run dir, then shells out to create.sh for the
//! child. The CLI does NOT spawn a supervisor for the child — the
//! parent's supervisor sees `child.spawned` in its tail-follow loop and
//! is the sole spawner of child supervisors (single-arbiter invariant,
//! design.md §7.2).

use std::path::PathBuf;

use serde::Serialize;
use serde_json::{json, Value};

use octl_core::{ensure_root, new_run_id, Kind, Lifecycle};

use crate::error::CliError;
use crate::idempotency;
use crate::output::{self, OutputFormat, OutputSpec};
use crate::run::{
    from_core, kind_kebab, lifecycle_for, lifecycle_kebab, parse_node_id, parse_run_id,
    require_nonempty, run_paths, spawn, supervisor_spawn,
};

/// A flat 1:1 mirror of `run create`'s clap flags (skip-materialize,
/// no-hooks, headless, dry-run), so the pedantic `struct_excessive_bools`
/// lint is allowed here — same allowance as the help-module arg bags.
#[allow(clippy::struct_excessive_bools)]
pub struct Args<'a> {
    pub skip_materialize: bool,
    pub kind: Kind,
    pub title: String,
    pub source_repo: Option<String>,
    pub source_branch: Option<String>,
    pub task: Option<String>,
    pub prompt_file: Option<String>,
    pub layout: Option<String>,
    pub no_hooks: bool,
    /// Place the worker's tmux window in a detached "headless" session.
    pub headless: bool,
    /// Explicit tmux session name; implies headless and overrides the
    /// `--headless` default name.
    pub tmux_session: Option<String>,
    pub parent_run_id: Option<String>,
    pub parent_node_id: Option<String>,
    pub idempotency_key: Option<String>,
    pub dry_run: bool,
    pub spec: &'a OutputSpec,
    pub warnings: &'a [String],
}

#[derive(Serialize)]
struct CreatedPayload<'a> {
    run_id: &'a str,
    dir: String,
    supervisor: SupervisorField,
    kind: KindStr,
    lifecycle: LifecycleStr,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_run_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_node_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    node_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tmux_window: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    worktree_path: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    branch: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    idempotent_replay: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dry_run: Option<bool>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum SupervisorField {
    Pid(u32),
    Note(&'static str),
}

#[derive(Serialize, Clone, Copy)]
#[serde(rename_all = "kebab-case")]
enum KindStr {
    Code,
    Spinoff,
    Orchestrated,
    Research,
    TechnicalDecision,
    MakeSkill,
    FanOut,
    Bugfix,
    Orchestrate,
}
impl From<Kind> for KindStr {
    fn from(k: Kind) -> Self {
        match k {
            Kind::Code => KindStr::Code,
            Kind::Spinoff => KindStr::Spinoff,
            Kind::Orchestrated => KindStr::Orchestrated,
            Kind::Research => KindStr::Research,
            Kind::TechnicalDecision => KindStr::TechnicalDecision,
            Kind::MakeSkill => KindStr::MakeSkill,
            Kind::FanOut => KindStr::FanOut,
            Kind::Bugfix => KindStr::Bugfix,
            Kind::Orchestrate => KindStr::Orchestrate,
        }
    }
}

#[derive(Serialize, Clone, Copy)]
#[serde(rename_all = "kebab-case")]
enum LifecycleStr {
    Autonomous,
    Interactive,
}
impl From<Lifecycle> for LifecycleStr {
    fn from(l: Lifecycle) -> Self {
        match l {
            Lifecycle::Autonomous => LifecycleStr::Autonomous,
            Lifecycle::Interactive => LifecycleStr::Interactive,
        }
    }
}

/// Successful spawn details captured for both the node.created event
/// and the emit payload. Kept Option-free so callers handle the
/// "no spawn happened" (dry-run, idempotent replay) case explicitly.
struct SpawnResult {
    branch: String,
    worktree_path: String,
    tmux_window: String,
    agent_pid: i32,
}

pub fn run(args: Args<'_>) -> Result<(), CliError> {
    let title = require_nonempty(&args.title, "title")?;

    // Resolve the optional headless target session up-front so a malformed
    // `--tmux-session` fails before we touch disk or the parent log — same
    // fail-fast contract as the prompt-source resolution below. `None` keeps
    // the existing foreground-spawn behavior (opt-in only).
    let parent_session = resolve_parent_session(args.headless, args.tmux_session.as_deref())?;

    let is_child = args.parent_run_id.is_some();
    if args.parent_run_id.is_some() ^ args.parent_node_id.is_some() {
        return Err(CliError::user(
            "invalid_arguments",
            "--parent-run-id and --parent-node-id must be set together",
        ));
    }

    // Resolve the prompt source up-front so a missing `--task` /
    // `--prompt-file` fails before we touch disk or the parent log.
    // Dry-run still requires it: rejecting late would let CI scripts
    // pass invalid configs as long as they always run --dry-run.
    // `--skip-materialize` is the test-only escape hatch that produces
    // only the run skeleton; there's no agent to prompt so neither
    // flag is required.
    // Test-only env override: integration tests use a bare run dir
    // without a real create.sh available. The env var implies
    // `--skip-materialize` and is set by the `bin()` helper in
    // `tests/`. Production callers never set it.
    // `Kind::Orchestrate` is the top-level DAG driver — the orchestrator
    // agent runs in the user's main conversation, not in a detached
    // worktree, so there is nothing for `create.sh` to materialize and
    // nothing for the watchdog to supervise. Force-skip materialization
    // for this kind so the driver run is just a run dir + event log +
    // manifest, ready for the orchestrator to append decisions and spawn
    // `Kind::Orchestrated` children that reference it.
    let skip_materialize = args.skip_materialize
        || args.kind == Kind::Orchestrate
        || std::env::var("OCTL_TEST_SKIP_MATERIALIZE").is_ok();
    let prompt_source = if skip_materialize {
        None
    } else {
        Some(resolve_prompt_source(
            args.task.as_deref(),
            args.prompt_file.as_deref(),
        )?)
    };

    if is_child && args.dry_run {
        return Err(CliError::user(
            "dry_run_unsupported",
            "child-spawn create cannot be truthfully dry-run; use --idempotency-key for safe retry",
        ));
    }

    // Validate the parent pointers strictly (RunId / NodeId shapes) but keep
    // them as `String` for the event-data payload and downstream `as_deref`.
    let parent_run_id = match args.parent_run_id.as_deref() {
        Some(v) => Some(parse_run_id(v)?.to_string()),
        None => None,
    };
    let parent_node_id = match args.parent_node_id.as_deref() {
        Some(v) => Some(parse_node_id(v)?.to_string()),
        None => None,
    };

    let root = crate::home::root_dir()?;

    if let Some(key) = args.idempotency_key.as_deref() {
        if let Some(existing) = idempotency::lookup(
            args.source_repo.as_deref(),
            args.source_branch.as_deref(),
            key,
        )? {
            // `existing` is a previously-stored run id; validate it before it
            // composes a path (run_dir only accepts a typed RunId).
            let existing_rid = parse_run_id(&existing)?;
            let dir = octl_core::run_dir(&root, &existing_rid);
            return emit(EmitInput {
                run_id: &existing,
                dir: dir.display().to_string(),
                kind: args.kind,
                lifecycle: lifecycle_for(args.kind),
                parent_run_id: parent_run_id.as_deref(),
                parent_node_id: parent_node_id.as_deref(),
                // An orchestrate driver always carries its `n-0001` driver node,
                // so a replay can truthfully report it. Worker replays don't
                // re-read their node here, so they stay `None` (unchanged).
                node_id: (args.kind == Kind::Orchestrate).then_some("n-0001"),
                spawn: None,
                supervisor_pid: None,
                idempotent_replay: Some(true),
                dry_run: None,
                spec: args.spec,
                warnings: args.warnings,
            });
        }
    }

    let run_id = new_run_id();
    // Validate the freshly generated id (infallible in practice) so run_dir
    // gets a typed RunId rather than a raw &str.
    let run_id_typed = parse_run_id(&run_id)?;
    let lifecycle = lifecycle_for(args.kind);
    let child_dir = octl_core::run_dir(&root, &run_id_typed);

    if args.dry_run {
        return emit(EmitInput {
            run_id: &run_id,
            dir: child_dir.display().to_string(),
            kind: args.kind,
            lifecycle,
            parent_run_id: None,
            parent_node_id: None,
            // Dry-run writes nothing to disk, so there is no node to report.
            node_id: None,
            spawn: None,
            supervisor_pid: None,
            idempotent_replay: None,
            dry_run: Some(true),
            spec: args.spec,
            warnings: args.warnings,
        });
    }

    ensure_root(&root).map_err(from_core)?;

    if is_child {
        let parent_run_id = parent_run_id.as_deref().unwrap();
        let parent_node_id = parent_node_id.as_deref().unwrap();
        let parent_paths = run_paths(&root, parent_run_id)?;
        if !parent_paths.manifest().exists() {
            return Err(CliError::user(
                "parent_not_found",
                format!("parent run {parent_run_id} does not exist"),
            )
            .with_invalid_value(parent_run_id));
        }
        let child_data = json!({
            "child_run_id": run_id,
            "child_node_id": "n-0001",
            "child_kind": kind_kebab(args.kind),
            "child_title": title,
        });
        // No idempotency key on the parent's `child.spawned`: `run create`'s
        // own dedup is the CLI-level `idempotency::store` (key -> run_id) that
        // short-circuits a retry *before* this point. Passing the key here
        // would instead make `append_and_apply_event` scan the parent log and,
        // on a retry that slipped past the store (e.g. a crash before
        // `idempotency::store`), return an idempotent replay of the *first*
        // child — silently orphaning the freshly generated `run_id`.
        octl_core::append_and_apply_event(
            &parent_paths,
            "child.spawned",
            Some(&parse_node_id(parent_node_id)?),
            None,
            child_data,
        )
        .map_err(from_core)?;
    }

    // Initialize the child (or top-level) run directory.
    std::fs::create_dir_all(&child_dir).map_err(|e| {
        CliError::system("io_error", format!("mkdir {}: {}", child_dir.display(), e))
    })?;
    let paths = run_paths(&root, &run_id)?;

    // Materialize the prompt file under <run-dir>/prompt.md unless the
    // caller supplied one outside the run dir (in which case we use the
    // file as-is so they keep ownership over it). When
    // `--skip-materialize` is set there's no spawn, no prompt.
    let prompt_path = match prompt_source {
        Some(src) => Some(resolve_prompt_file(&child_dir, src)?),
        None => None,
    };

    // Write run.created BEFORE shelling out so a create.sh crash leaves
    // a recoverable run on disk that `run show`/`run cancel` can see.
    let mut data = serde_json::Map::new();
    data.insert("kind".into(), Value::String(kind_kebab(args.kind).into()));
    data.insert(
        "lifecycle".into(),
        Value::String(lifecycle_kebab(lifecycle).into()),
    );
    data.insert("title".into(), Value::String(title.clone()));
    if let Some(v) = args.source_repo.as_deref() {
        data.insert("source_repo".into(), Value::String(v.into()));
    }
    if let Some(v) = args.source_branch.as_deref() {
        data.insert("source_branch".into(), Value::String(v.into()));
    }
    if let Some(v) = args.task.as_deref() {
        data.insert("task".into(), Value::String(v.into()));
    }
    if is_child {
        data.insert(
            "parent_run_id".into(),
            Value::String(parent_run_id.clone().unwrap()),
        );
        data.insert(
            "parent_node_id".into(),
            Value::String(parent_node_id.clone().unwrap()),
        );
    }
    // As with `child.spawned` above, `run create` dedups via the CLI-level
    // `idempotency::store` (below), not the folded event-log scan — so the
    // append carries no idempotency key.
    octl_core::append_and_apply_event(&paths, "run.created", None, None, Value::Object(data))
        .map_err(from_core)?;

    // Orchestrate driver: synthesize the `n-0001` driver node. The
    // orchestrator agent runs in the user's main conversation (no worktree
    // to materialize), but children spawned via `--kind orchestrated` REQUIRE
    // a `--parent-node-id`. Without a node on disk the orchestrator would have
    // to guess that id. Emitting `node.created` here makes `n-0001` the
    // discoverable, programmatic answer: it lands in the envelope's `node_id`,
    // bumps `manifest.node_count` to 1, and shows up in `node list`. The node
    // carries no tmux/branch/pid metadata — it is the DAG root, not a worker.
    let driver_node_id = if args.kind == Kind::Orchestrate {
        octl_core::append_and_apply_event(
            &paths,
            "node.created",
            Some(&parse_node_id("n-0001").expect("n-0001 is a valid node id")),
            None,
            json!({
                "kind": kind_kebab(args.kind),
                "task": args.task,
            }),
        )
        .map_err(from_core)?;
        Some("n-0001")
    } else {
        None
    };

    // `--skip-materialize` short-circuit: skeleton-only run, used by
    // tests that need a run dir without booting a real worktree/agent.
    if skip_materialize {
        if let Some(key) = args.idempotency_key.as_deref() {
            idempotency::store(
                args.source_repo.as_deref(),
                args.source_branch.as_deref(),
                key,
                &run_id,
            )?;
        }
        return emit(EmitInput {
            run_id: &run_id,
            dir: child_dir.display().to_string(),
            kind: args.kind,
            lifecycle,
            parent_run_id: parent_run_id.as_deref(),
            parent_node_id: parent_node_id.as_deref(),
            node_id: driver_node_id,
            spawn: None,
            supervisor_pid: None,
            idempotent_replay: None,
            dry_run: None,
            spec: args.spec,
            warnings: args.warnings,
        });
    }

    let prompt_path = prompt_path.expect("non-skip path resolves prompt source");
    // Shell out to create.sh. A failure here leaves the run on disk in
    // `pending` with no node — `run cancel` / `run show` still work,
    // and a retry with the same `--idempotency-key` short-circuits.
    let branch_name = derive_branch_name(args.kind, &run_id, &title);
    let spawn_req = spawn::SpawnRequest {
        kind: kind_kebab(args.kind),
        branch: &branch_name,
        prompt_file: &prompt_path,
        layout: args.layout.as_deref(),
        no_hooks: args.no_hooks,
        keep_tmux_on_error: false,
        parent_session: parent_session.as_deref(),
        // Fork the worktree's branch from the named source branch (e.g. an
        // orchestrate integration branch) rather than workmux's default base.
        // `None` for runs without --source-branch keeps the prior behaviour.
        source_branch: args.source_branch.as_deref(),
    };
    let outcome = spawn::run_create_sh(&spawn_req)?;
    // V2: re-verify the discovered PID is still alive. If it died
    // between create.sh's check and ours, emit node.failed and return
    // a structured error rather than silently recording a dead PID.
    if let Err(e) = spawn::verify_agent_pid(outcome.agent_pid_hint) {
        let _ = octl_core::append_and_apply_event(
            &paths,
            "node.failed",
            Some(&parse_node_id("n-0001").expect("n-0001 is a valid node id")),
            None,
            json!({
                "reason": "agent-pid-discovery-failed",
                "agent_pid_hint": outcome.agent_pid_hint,
                "branch": outcome.branch,
                "tmux_window": outcome.tmux_window,
            }),
        );
        return Err(e);
    }

    // Emit node.created with the discovered metadata. The reducer
    // creates nodes/n-0001.json with these fields wired in.
    let node_data = json!({
        "kind": kind_kebab(args.kind),
        "branch": outcome.branch,
        "worktree_path": outcome.worktree_path,
        "tmux_window": outcome.tmux_window,
        // Qualified tmux identity (null on a create.sh that predates the
        // fields); the reducer folds these into Node.tmux_identity.
        "tmux_socket": outcome.tmux_socket,
        "tmux_session": outcome.tmux_session,
        "tmux_window_id": outcome.tmux_window_id,
        "agent_pid": outcome.agent_pid_hint,
        "task": args.task,
        "parent_node_id": parent_node_id,
    });
    octl_core::append_and_apply_event(
        &paths,
        "node.created",
        Some(&parse_node_id("n-0001").expect("n-0001 is a valid node id")),
        None,
        node_data,
    )
    .map_err(from_core)?;

    // For top-level runs, spawn the supervisor and wait for its PID
    // file. Child-spawn delegates supervisor creation to the parent
    // supervisor (design.md §7.2 step 6).
    let supervisor_pid = if is_child {
        None
    } else {
        Some(supervisor_spawn::spawn_for_run(&paths, &run_id)?.pid)
    };

    if let Some(key) = args.idempotency_key.as_deref() {
        idempotency::store(
            args.source_repo.as_deref(),
            args.source_branch.as_deref(),
            key,
            &run_id,
        )?;
    }

    let spawn_result = SpawnResult {
        branch: outcome.branch,
        worktree_path: outcome.worktree_path,
        tmux_window: outcome.tmux_window,
        agent_pid: outcome.agent_pid_hint as i32,
    };
    let _ = spawn_result.agent_pid; // recorded on the node via reducer; unused in payload

    emit(EmitInput {
        run_id: &run_id,
        dir: child_dir.display().to_string(),
        kind: args.kind,
        lifecycle,
        parent_run_id: parent_run_id.as_deref(),
        parent_node_id: parent_node_id.as_deref(),
        node_id: Some("n-0001"),
        spawn: Some(&spawn_result),
        supervisor_pid,
        idempotent_replay: None,
        dry_run: None,
        spec: args.spec,
        warnings: args.warnings,
    })
}

/// Default detached session name used when `--headless` is set without an
/// explicit `--tmux-session`. The user attaches with `tmux attach -t headless`.
const DEFAULT_HEADLESS_SESSION: &str = "headless";

/// Resolve the optional `--parent-session` value forwarded to create.sh from
/// the `--headless` / `--tmux-session` pair:
///
/// - `--tmux-session <name>` always wins (it also implies headless placement),
/// - `--headless` alone yields the [`DEFAULT_HEADLESS_SESSION`] name,
/// - neither yields `None` — the existing foreground spawn (opt-in only).
///
/// An explicit name is validated strictly: a tmux session name must be
/// non-empty and may not contain whitespace or tmux's `:`/`.` target
/// separators, which would otherwise be silently mis-parsed by tmux as a
/// `session:window.pane` target. The default `headless` name trivially passes.
fn resolve_parent_session(
    headless: bool,
    tmux_session: Option<&str>,
) -> Result<Option<String>, CliError> {
    match tmux_session {
        Some(raw) => {
            let name = raw.trim();
            if name.is_empty() {
                return Err(CliError::user(
                    "invalid_value",
                    "--tmux-session must not be empty or whitespace-only",
                )
                .with_invalid_value(raw));
            }
            if name
                .chars()
                .any(|c| c.is_whitespace() || c == ':' || c == '.')
            {
                return Err(CliError::user(
                    "invalid_value",
                    "--tmux-session must not contain whitespace or the ':'/'.' tmux target separators",
                )
                .with_invalid_value(raw));
            }
            Ok(Some(name.to_string()))
        }
        None if headless => Ok(Some(DEFAULT_HEADLESS_SESSION.to_string())),
        None => Ok(None),
    }
}

#[derive(Debug)]
enum PromptSource {
    Task(String),
    File(PathBuf),
}

fn resolve_prompt_source(
    task: Option<&str>,
    prompt_file: Option<&str>,
) -> Result<PromptSource, CliError> {
    match (task, prompt_file) {
        (Some(t), None) => {
            let t = t.trim();
            if t.is_empty() {
                return Err(CliError::user(
                    "invalid_value",
                    "--task must not be empty or whitespace-only",
                ));
            }
            Ok(PromptSource::Task(t.to_string()))
        }
        (None, Some(p)) => {
            let path = PathBuf::from(p);
            if !path.exists() {
                return Err(CliError::user(
                    "prompt_file_not_found",
                    format!("--prompt-file does not exist: {}", path.display()),
                )
                .with_invalid_value(p));
            }
            Ok(PromptSource::File(path))
        }
        (Some(_), Some(_)) => Err(CliError::user(
            "invalid_arguments",
            "--task and --prompt-file are mutually exclusive",
        )),
        (None, None) => Err(CliError::user(
            "missing-task-or-prompt-file",
            "either --task <text> or --prompt-file <path> is required",
        )),
    }
}

fn resolve_prompt_file(run_dir: &std::path::Path, src: PromptSource) -> Result<PathBuf, CliError> {
    match src {
        PromptSource::Task(t) => spawn::write_prompt_file(run_dir, &t),
        PromptSource::File(p) => Ok(p),
    }
}

/// Build the branch name we hand to create.sh. We keep the convention
/// the skill family uses (`wt/<short-id>-<slug>`) so windows produced
/// by `orchestratectl` and `/worktree-code` look identical in tmux.
fn derive_branch_name(kind: Kind, run_id: &str, title: &str) -> String {
    let short = run_id
        .to_ascii_lowercase()
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(10)
        .collect::<String>();
    let slug: String = title
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let slug = if slug.is_empty() {
        kind_kebab(kind).to_string()
    } else {
        slug
    };
    let slug: String = slug.chars().take(40).collect();
    format!("wt/{short}-{slug}")
}

struct EmitInput<'a> {
    run_id: &'a str,
    dir: String,
    kind: Kind,
    lifecycle: Lifecycle,
    parent_run_id: Option<&'a str>,
    parent_node_id: Option<&'a str>,
    /// The id of this run's own primary node, surfaced as the envelope's
    /// `node_id` so callers can discover it without guessing. `Some("n-0001")`
    /// once the node exists on disk (materialized worker, or the synthetic
    /// orchestrate driver node); `None` for dry-run / test-skip skeletons.
    node_id: Option<&'a str>,
    spawn: Option<&'a SpawnResult>,
    supervisor_pid: Option<u32>,
    idempotent_replay: Option<bool>,
    dry_run: Option<bool>,
    spec: &'a OutputSpec,
    warnings: &'a [String],
}

fn emit(i: EmitInput<'_>) -> Result<(), CliError> {
    let supervisor = match (i.supervisor_pid, i.dry_run, i.idempotent_replay, i.kind) {
        (Some(pid), _, _, _) => SupervisorField::Pid(pid),
        (None, Some(true), _, _) => SupervisorField::Note("not-spawned-dry-run"),
        (None, _, Some(true), _) => SupervisorField::Note("recorded-on-prior-run"),
        // Orchestrate driver runs in the user's main conversation — there
        // is no detached worker for a supervisor to watch. Children
        // (`Kind::Orchestrated`) spawn their own supervisors.
        (None, _, _, Kind::Orchestrate) => {
            SupervisorField::Note("orchestrator-in-main-conversation")
        }
        // Child-spawn: parent supervisor handles supervisor creation.
        (None, _, _, _) => SupervisorField::Note("delegated-to-parent-supervisor"),
    };
    let payload = CreatedPayload {
        run_id: i.run_id,
        dir: i.dir,
        supervisor,
        kind: i.kind.into(),
        lifecycle: i.lifecycle.into(),
        parent_run_id: i.parent_run_id,
        parent_node_id: i.parent_node_id,
        node_id: i.node_id,
        tmux_window: i.spawn.map(|s| s.tmux_window.as_str()),
        worktree_path: i.spawn.map(|s| s.worktree_path.as_str()),
        branch: i.spawn.map(|s| s.branch.as_str()),
        idempotent_replay: i.idempotent_replay,
        dry_run: i.dry_run,
    };
    match i.spec.format {
        OutputFormat::Json | OutputFormat::Jsonl => {
            output::emit_envelope(&payload, i.spec, i.warnings)?;
        }
        OutputFormat::Text => {
            println!("run-id: {}", payload.run_id);
            println!("dir:    {}", payload.dir);
            println!("kind:   {}", kind_kebab(i.kind));
            match &payload.supervisor {
                SupervisorField::Pid(p) => println!("status: running  (supervisor pid {p})"),
                SupervisorField::Note(n) => println!("status: pending  (supervisor: {n})"),
            }
            if let Some(b) = payload.branch {
                println!("branch: {b}");
            }
            if let Some(w) = payload.worktree_path {
                println!("path:   {w}");
            }
            if let Some(t) = payload.tmux_window {
                println!("tmux:   {t}");
            }
            if let (Some(p), Some(n)) = (payload.parent_run_id, payload.parent_node_id) {
                println!("parent: {p}/{n}");
            }
            if payload.idempotent_replay == Some(true) {
                println!("note:   returned from idempotency-key cache");
            }
            if payload.dry_run == Some(true) {
                println!("note:   --dry-run (no filesystem changes)");
            }
            output::emit_text_warnings(i.warnings);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_branch_basic() {
        let b = derive_branch_name(
            Kind::Spinoff,
            "01JX1NS7H8AAAAA9BBBBBBBBBB",
            "Login redirect bug",
        );
        assert!(b.starts_with("wt/"));
        assert!(b.ends_with("login-redirect-bug"), "got {b}");
    }

    #[test]
    fn derive_branch_empty_title_uses_kind() {
        let b = derive_branch_name(Kind::Bugfix, "01JX1234567890ABCDE", "   !!!  ");
        assert!(b.ends_with("bugfix"));
    }

    #[test]
    fn missing_task_and_prompt_file_errors() {
        let e = resolve_prompt_source(None, None).unwrap_err();
        assert_eq!(e.code, "missing-task-or-prompt-file");
    }

    #[test]
    fn task_and_prompt_file_conflict() {
        let e = resolve_prompt_source(Some("x"), Some("/tmp/p.md")).unwrap_err();
        assert_eq!(e.code, "invalid_arguments");
    }

    #[test]
    fn empty_task_rejected() {
        let e = resolve_prompt_source(Some("   "), None).unwrap_err();
        assert_eq!(e.code, "invalid_value");
    }

    #[test]
    fn parent_session_defaults_to_none() {
        assert_eq!(resolve_parent_session(false, None).unwrap(), None);
    }

    #[test]
    fn headless_yields_default_session() {
        assert_eq!(
            resolve_parent_session(true, None).unwrap().as_deref(),
            Some("headless")
        );
    }

    #[test]
    fn explicit_tmux_session_wins_and_implies_headless() {
        // Explicit name overrides the default even with --headless unset.
        assert_eq!(
            resolve_parent_session(false, Some("campaign"))
                .unwrap()
                .as_deref(),
            Some("campaign")
        );
        assert_eq!(
            resolve_parent_session(true, Some("campaign"))
                .unwrap()
                .as_deref(),
            Some("campaign")
        );
    }

    #[test]
    fn tmux_session_trimmed() {
        assert_eq!(
            resolve_parent_session(false, Some("  bg  "))
                .unwrap()
                .as_deref(),
            Some("bg")
        );
    }

    #[test]
    fn empty_tmux_session_rejected() {
        let e = resolve_parent_session(false, Some("   ")).unwrap_err();
        assert_eq!(e.code, "invalid_value");
    }

    #[test]
    fn tmux_session_with_separator_rejected() {
        for bad in ["a:b", "a.b", "a b"] {
            let e = resolve_parent_session(false, Some(bad)).unwrap_err();
            assert_eq!(e.code, "invalid_value", "expected reject for {bad:?}");
        }
    }
}
