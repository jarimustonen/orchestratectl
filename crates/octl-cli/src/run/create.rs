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
    from_core, kind_kebab, lifecycle_for, lifecycle_kebab, require_nonempty, require_safe_id,
    run_paths, spawn, supervisor_spawn,
};

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
    let skip_materialize =
        args.skip_materialize || std::env::var("OCTL_TEST_SKIP_MATERIALIZE").is_ok();
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

    let parent_run_id = match args.parent_run_id.as_deref() {
        Some(v) => Some(require_safe_id(v, "parent-run-id")?),
        None => None,
    };
    let parent_node_id = match args.parent_node_id.as_deref() {
        Some(v) => Some(require_safe_id(v, "parent-node-id")?),
        None => None,
    };

    let root = crate::home::root_dir()?;

    if let Some(key) = args.idempotency_key.as_deref() {
        if let Some(existing) = idempotency::lookup(
            args.source_repo.as_deref(),
            args.source_branch.as_deref(),
            key,
        )? {
            let dir = octl_core::run_dir(&root, &existing);
            return emit(EmitInput {
                run_id: &existing,
                dir: dir.display().to_string(),
                kind: args.kind,
                lifecycle: lifecycle_for(args.kind),
                parent_run_id: parent_run_id.as_deref(),
                parent_node_id: parent_node_id.as_deref(),
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
    let lifecycle = lifecycle_for(args.kind);
    let child_dir = octl_core::run_dir(&root, &run_id);

    if args.dry_run {
        return emit(EmitInput {
            run_id: &run_id,
            dir: child_dir.display().to_string(),
            kind: args.kind,
            lifecycle,
            parent_run_id: None,
            parent_node_id: None,
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
        octl_core::append_and_apply(
            &parent_paths,
            "child.spawned",
            Some(parent_node_id),
            args.idempotency_key.as_deref(),
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
    octl_core::append_and_apply(
        &paths,
        "run.created",
        None,
        args.idempotency_key.as_deref(),
        Value::Object(data),
    )
    .map_err(from_core)?;

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
    };
    let outcome = spawn::run_create_sh(&spawn_req)?;
    // V2: re-verify the discovered PID is still alive. If it died
    // between create.sh's check and ours, emit node.failed and return
    // a structured error rather than silently recording a dead PID.
    if let Err(e) = spawn::verify_agent_pid(outcome.agent_pid_hint) {
        let _ = octl_core::append_and_apply(
            &paths,
            "node.failed",
            Some("n-0001"),
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
        "agent_pid": outcome.agent_pid_hint,
        "task": args.task,
        "parent_node_id": parent_node_id,
    });
    octl_core::append_and_apply(&paths, "node.created", Some("n-0001"), None, node_data)
        .map_err(from_core)?;

    // For top-level runs, spawn the supervisor and wait for its PID
    // file. Child-spawn delegates supervisor creation to the parent
    // supervisor (design.md §7.2 step 6).
    let supervisor_pid = if !is_child {
        Some(supervisor_spawn::spawn_for_run(&paths, &run_id)?.pid)
    } else {
        None
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
        spawn: Some(&spawn_result),
        supervisor_pid,
        idempotent_replay: None,
        dry_run: None,
        spec: args.spec,
        warnings: args.warnings,
    })
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
        .filter(|c| c.is_ascii_alphanumeric())
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
    format!("wt/{}-{}", short, slug)
}

struct EmitInput<'a> {
    run_id: &'a str,
    dir: String,
    kind: Kind,
    lifecycle: Lifecycle,
    parent_run_id: Option<&'a str>,
    parent_node_id: Option<&'a str>,
    spawn: Option<&'a SpawnResult>,
    supervisor_pid: Option<u32>,
    idempotent_replay: Option<bool>,
    dry_run: Option<bool>,
    spec: &'a OutputSpec,
    warnings: &'a [String],
}

fn emit(i: EmitInput<'_>) -> Result<(), CliError> {
    let supervisor = match (i.supervisor_pid, i.dry_run, i.idempotent_replay) {
        (Some(pid), _, _) => SupervisorField::Pid(pid),
        (None, Some(true), _) => SupervisorField::Note("not-spawned-dry-run"),
        (None, _, Some(true)) => SupervisorField::Note("recorded-on-prior-run"),
        // Child-spawn: parent supervisor handles supervisor creation.
        (None, _, _) => SupervisorField::Note("delegated-to-parent-supervisor"),
    };
    let payload = CreatedPayload {
        run_id: i.run_id,
        dir: i.dir,
        supervisor,
        kind: i.kind.into(),
        lifecycle: i.lifecycle.into(),
        parent_run_id: i.parent_run_id,
        parent_node_id: i.parent_node_id,
        node_id: i.spawn.map(|_| "n-0001"),
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
                SupervisorField::Pid(p) => println!("status: running  (supervisor pid {})", p),
                SupervisorField::Note(n) => println!("status: pending  (supervisor: {})", n),
            }
            if let Some(b) = payload.branch {
                println!("branch: {}", b);
            }
            if let Some(w) = payload.worktree_path {
                println!("path:   {}", w);
            }
            if let Some(t) = payload.tmux_window {
                println!("tmux:   {}", t);
            }
            if let (Some(p), Some(n)) = (payload.parent_run_id, payload.parent_node_id) {
                println!("parent: {}/{}", p, n);
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
}
