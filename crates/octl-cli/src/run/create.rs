//! `run create` — top-level + child-spawn run initialization.
//!
//! Top-level: writes `run.created` to a fresh run dir; supervisor spawn
//! lands in `supervisor-process`. Child-spawn: writes `child.spawned`
//! to the **parent** run's events (per design.md §7.2 step 3) and
//! `run.created` to the child run's events. The MVP CLI never spawns
//! a supervisor — the parent supervisor (once it exists) does it from
//! its tail-follow loop.

use serde::Serialize;
use serde_json::{json, Value};

use octl_core::{ensure_root, new_run_id, Kind, Lifecycle};

use crate::error::CliError;
use crate::idempotency;
use crate::output;
use crate::run::{from_core, lifecycle_for, require_nonempty, run_paths};

pub struct Args<'a> {
    pub kind: Kind,
    pub title: String,
    pub source_repo: Option<String>,
    pub source_branch: Option<String>,
    pub task: Option<String>,
    pub parent_run_id: Option<String>,
    pub parent_node_id: Option<String>,
    pub idempotency_key: Option<String>,
    pub dry_run: bool,
    pub json: bool,
    pub warnings: &'a [String],
}

#[derive(Serialize)]
struct CreatedPayload<'a> {
    run_id: &'a str,
    dir: String,
    supervisor: &'static str,
    kind: KindStr,
    lifecycle: LifecycleStr,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_run_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_node_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    idempotent_replay: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dry_run: Option<bool>,
}

// Local mirrors of the schema enums so the CLI envelope stays stable
// even if the on-disk serde rename ever changes.
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

pub fn run(args: Args<'_>) -> Result<(), CliError> {
    let title = require_nonempty(&args.title, "title")?;

    let is_child = args.parent_run_id.is_some();
    // clap `requires` already enforces both-or-neither, but a manual
    // belt-and-suspenders check protects callers that hand-build args.
    if args.parent_run_id.is_some() ^ args.parent_node_id.is_some() {
        return Err(CliError::user(
            "invalid_arguments",
            "--parent-run-id and --parent-node-id must be set together",
        ));
    }

    if is_child && args.dry_run {
        // Per AGENTS-AI-FIRST-CLI §11: a child-spawn dry-run would have
        // to reserve a run-id without telling the parent supervisor,
        // which can't observe the reservation. Refuse with the canonical
        // envelope rather than fake it.
        return Err(CliError::user(
            "dry-run-unsupported",
            "child-spawn create cannot be truthfully dry-run; use --idempotency-key for safe retry",
        ));
    }

    let root = crate::home::root_dir()?;

    // Idempotency short-circuit applies to both top-level and child-spawn.
    if let Some(key) = args.idempotency_key.as_deref() {
        if let Some(existing) = idempotency::lookup(
            args.source_repo.as_deref(),
            args.source_branch.as_deref(),
            key,
        )? {
            let dir = octl_core::run_dir(&root, &existing);
            return emit(
                &existing,
                dir.display().to_string(),
                args.kind,
                lifecycle_for(args.kind),
                args.parent_run_id.as_deref(),
                args.parent_node_id.as_deref(),
                Some(true),
                None,
                args.json,
                args.warnings,
            );
        }
    }

    let run_id = new_run_id();
    let lifecycle = lifecycle_for(args.kind);
    let child_dir = octl_core::run_dir(&root, &run_id);

    if args.dry_run {
        return emit(
            &run_id,
            child_dir.display().to_string(),
            args.kind,
            lifecycle,
            None,
            None,
            None,
            Some(true),
            args.json,
            args.warnings,
        );
    }

    ensure_root(&root).map_err(from_core)?;

    if is_child {
        // The parent run must already exist on disk — otherwise the
        // `child.spawned` write would land in a fabricated parent dir
        // and the projection would never be observed.
        let parent_run_id = args.parent_run_id.as_deref().unwrap();
        let parent_node_id = args.parent_node_id.as_deref().unwrap();
        let parent_paths = run_paths(&root, parent_run_id);
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
    let paths = run_paths(&root, &run_id);
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
            Value::String(args.parent_run_id.clone().unwrap()),
        );
        data.insert(
            "parent_node_id".into(),
            Value::String(args.parent_node_id.clone().unwrap()),
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

    if let Some(key) = args.idempotency_key.as_deref() {
        idempotency::store(
            args.source_repo.as_deref(),
            args.source_branch.as_deref(),
            key,
            &run_id,
        )?;
    }

    emit(
        &run_id,
        child_dir.display().to_string(),
        args.kind,
        lifecycle,
        args.parent_run_id.as_deref(),
        args.parent_node_id.as_deref(),
        None,
        None,
        args.json,
        args.warnings,
    )
}

#[allow(clippy::too_many_arguments)]
fn emit(
    run_id: &str,
    dir: String,
    kind: Kind,
    lifecycle: Lifecycle,
    parent_run_id: Option<&str>,
    parent_node_id: Option<&str>,
    idempotent_replay: Option<bool>,
    dry_run: Option<bool>,
    json: bool,
    warnings: &[String],
) -> Result<(), CliError> {
    let payload = CreatedPayload {
        run_id,
        dir,
        supervisor: "not-yet-spawned",
        kind: kind.into(),
        lifecycle: lifecycle.into(),
        parent_run_id,
        parent_node_id,
        idempotent_replay,
        dry_run,
    };
    if json {
        output::emit_json(&payload, warnings)
            .map_err(|e| CliError::system("internal_serialize", e.to_string()))?;
    } else {
        println!("run-id: {}", payload.run_id);
        println!("dir:    {}", payload.dir);
        println!("kind:   {}", kind_kebab(kind));
        println!("status: pending  (supervisor: {})", payload.supervisor);
        if let (Some(p), Some(n)) = (parent_run_id, parent_node_id) {
            println!("parent: {}/{}", p, n);
        }
        if idempotent_replay == Some(true) {
            println!("note:   returned from idempotency-key cache");
        }
        if dry_run == Some(true) {
            println!("note:   --dry-run (no filesystem changes)");
        }
        for w in warnings {
            eprintln!("warning: {}", w);
        }
    }
    Ok(())
}

fn kind_kebab(k: Kind) -> &'static str {
    match k {
        Kind::Code => "code",
        Kind::Spinoff => "spinoff",
        Kind::Orchestrated => "orchestrated",
        Kind::Research => "research",
        Kind::TechnicalDecision => "technical-decision",
        Kind::MakeSkill => "make-skill",
        Kind::FanOut => "fan-out",
        Kind::Bugfix => "bugfix",
    }
}

fn lifecycle_kebab(l: Lifecycle) -> &'static str {
    match l {
        Lifecycle::Autonomous => "autonomous",
        Lifecycle::Interactive => "interactive",
    }
}
