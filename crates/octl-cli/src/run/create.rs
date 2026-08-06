//! `run create` — top-level + child-spawn run initialization.
//!
//! Top-level: initializes the run dir, shells out to create.sh to
//! materialize worktree + tmux window + agent, emits `node.created`
//! with the discovered agent PID, and spawns the supervisor.
//!
//! Child-spawn (`--parent-run-id` + `--parent-node-id`): initializes the
//! child run dir and shells out to create.sh for the child, then — only
//! once create.sh has returned success and the agent PID is verified —
//! emits `child.spawned` on the parent's events. This emit-after-success
//! ordering keeps the parent's DAG bookkeeping transactional: a create.sh
//! failure removes the half-built child run dir and emits NO `child.spawned`,
//! so a failed spawn never leaves a phantom 0-node child in `pending` on the
//! parent (issue: failed-spawn-leaves-phantom-child). The CLI does NOT spawn
//! a supervisor for the child — the parent's supervisor sees `child.spawned`
//! in its tail-follow loop and is the sole spawner of child supervisors
//! (single-arbiter invariant, design.md §7.2).

use std::path::PathBuf;

use serde::Serialize;
use serde_json::{json, Value};

use octl_core::{ensure_root, new_run_id, Kind, Lifecycle};

use crate::error::CliError;
use crate::idempotency;
use crate::output::{self, OutputFormat, OutputSpec};
use crate::run::{
    from_core, kind_kebab, lifecycle_for, lifecycle_kebab, parse_node_id, parse_run_id,
    require_nonempty, run_paths_exact, spawn, supervisor_spawn,
};

/// Drop-releases an idempotency reservation unless disarmed.
///
/// Armed the moment `reserve` wins the key; disarmed once the run is durable
/// enough that a keyed retry should REPLAY it rather than re-spawn. So any `?`
/// early-return between `reserve` and that commit point frees the key on unwind
/// — otherwise an error before the run materialized would strand the key on a
/// phantom run forever (the reservation-leak the review caught). Disarm points:
/// top-level after `run.created` is durable (a recoverable pending run — keep
/// the key so a retry short-circuits to it); a child only on full success after
/// `child.spawned` (a child failure discards the run dir, so its key must be
/// freed). Release is ownership-checked, so this never clobbers another run's
/// key. NOTE: `Drop` does not run on a hard process kill — see the module doc
/// for that documented crash-window limitation.
struct ReservationGuard {
    repo: Option<String>,
    branch: Option<String>,
    key: String,
    run_id: String,
    armed: bool,
}

impl ReservationGuard {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ReservationGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = idempotency::release(
                self.repo.as_deref(),
                self.branch.as_deref(),
                &self.key,
                &self.run_id,
            );
        }
    }
}

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
    /// Seconds create.sh waits for the agent to become discoverable,
    /// forwarded as `--agent-startup-timeout`. Validated to [1, 600] by
    /// clap; defaults to 90 (see the flag docs in `run/mod.rs`).
    pub agent_startup_timeout: u32,
    pub parent_run_id: Option<String>,
    pub parent_node_id: Option<String>,
    /// Completion-notification command (`--notify`). Persisted into the
    /// `run.created` event as `notify_cmd` so the supervisor can run it once
    /// on the terminal transition. `None` for a run created without `--notify`.
    pub notify: Option<String>,
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

    // Validate the optional completion hook up front (fail fast, before we
    // touch disk): an all-whitespace `--notify` is a caller mistake. `None`
    // (flag omitted) is the common case and leaves the run hook-less.
    let notify_cmd = match args.notify.as_deref() {
        Some(raw) => Some(require_nonempty(raw, "notify")?),
        None => None,
    };

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

    // Validate the parent pointers strictly (RunId / NodeId shapes) ONCE, keeping
    // the typed ids so the exact path helper (and the `child.spawned` append)
    // reuse them without a re-parse — a parent pointer is exact-only and must
    // never route through the fuzzy CLI resolver. The `String` forms are derived
    // from the typed ids purely for the event-data payload and downstream
    // `as_deref`.
    let parent_run_id_typed = match args.parent_run_id.as_deref() {
        Some(v) => Some(parse_run_id(v)?),
        None => None,
    };
    let parent_node_id_typed = match args.parent_node_id.as_deref() {
        Some(v) => Some(parse_node_id(v)?),
        None => None,
    };
    let parent_run_id = parent_run_id_typed.as_ref().map(|r| r.as_str().to_string());
    let parent_node_id = parent_node_id_typed
        .as_ref()
        .map(|n| n.as_str().to_string());

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

    // Atomically reserve the idempotency key BEFORE materializing the run.
    // The top-of-function `lookup` is only a fast path; THIS reservation is the
    // authoritative check-and-set that closes the duplicate-create race. The
    // key file becomes visible to other callers only here — the old code stored
    // it after full materialization (seconds later, once create.sh had spawned
    // the whole worktree), so two near-simultaneous same-key calls both missed
    // the lookup and both spawned. `reserve` is an atomic filesystem operation:
    // exactly one concurrent caller wins and materializes; the losers observe
    // the reservation and replay the winner's run instead of spawning a
    // duplicate. Skipped on dry-run (handled above — dry-run persists nothing).
    let mut reservation: Option<ReservationGuard> =
        if let Some(key) = args.idempotency_key.as_deref() {
            match idempotency::reserve(
                args.source_repo.as_deref(),
                args.source_branch.as_deref(),
                key,
                &run_id,
            )? {
                idempotency::Reservation::Reserved => Some(ReservationGuard {
                    repo: args.source_repo.clone(),
                    branch: args.source_branch.clone(),
                    key: key.to_string(),
                    run_id: run_id.clone(),
                    armed: true,
                }),
                idempotency::Reservation::AlreadyReserved(existing) => {
                    // A concurrent same-key call won the race between our `lookup`
                    // and this `reserve`. Replay ITS run, exactly as the top-of-
                    // function fast path would have.
                    let existing_rid = parse_run_id(&existing)?;
                    let dir = octl_core::run_dir(&root, &existing_rid);
                    return emit(EmitInput {
                        run_id: &existing,
                        dir: dir.display().to_string(),
                        kind: args.kind,
                        lifecycle,
                        parent_run_id: parent_run_id.as_deref(),
                        parent_node_id: parent_node_id.as_deref(),
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
        } else {
            None
        };

    ensure_root(&root).map_err(from_core)?;

    if is_child {
        // Validate the parent exists up front (fail fast before we create the
        // child run dir or shell out to create.sh). The `child.spawned` event
        // itself is emitted only AFTER create.sh succeeds — see below — so a
        // create.sh failure never pollutes the parent's DAG bookkeeping.
        let parent_run_id = parent_run_id.as_deref().unwrap();
        // `is_child` ⇒ the parent pointer was validated to a typed `RunId` above;
        // reuse it for the exact path — a parent pointer must never fuzzy-resolve.
        let parent_paths = run_paths_exact(&root, parent_run_id_typed.as_ref().expect("is_child"))?;
        if !parent_paths.manifest().exists() {
            return Err(CliError::user(
                "parent_not_found",
                format!("parent run {parent_run_id} does not exist"),
            )
            .with_invalid_value(parent_run_id));
        }
    }

    // Initialize the child (or top-level) run directory.
    std::fs::create_dir_all(&child_dir).map_err(|e| {
        CliError::system("io_error", format!("mkdir {}: {}", child_dir.display(), e))
    })?;
    let paths = run_paths_exact(&root, &run_id_typed)?;

    // Materialize the prompt file under <run-dir>/prompt.md unless the
    // caller supplied one outside the run dir (in which case we use the
    // file as-is so they keep ownership over it). When
    // `--skip-materialize` is set there's no spawn, no prompt.
    let prompt_path = match prompt_source {
        Some(src) => Some(resolve_prompt_file(&child_dir, src)?),
        None => None,
    };

    // Write run.created BEFORE shelling out so a create.sh crash leaves
    // a recoverable run on disk that `run show`/`run cancel` can see. For a
    // child spawn this run dir is instead removed wholesale on create.sh
    // failure (see the spawn-failure arms below) — nothing references it yet
    // because `child.spawned` is emitted only after success.
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
    // Record the headless session orchestratectl is about to create via
    // create.sh's `--parent-session` so the supervisor can tear it down once
    // its last managed window is gone. `None` for a foreground spawn — that
    // window lives in the user's own session, which is never a teardown target
    // (issue `headless-tmux-session-not-torn-down`).
    if let Some(v) = parent_session.as_deref() {
        data.insert("managed_tmux_session".into(), Value::String(v.into()));
    }
    // Persist the terminal-completion hook so the supervisor can run it once
    // when this run settles (issue `no-completion-notification-to-parent`).
    // Trimmed and empty-rejected up front — an all-whitespace `--notify` is a
    // caller mistake, not a silent no-op hook.
    if let Some(cmd) = notify_cmd.as_deref() {
        data.insert("notify_cmd".into(), Value::String(cmd.into()));
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
    // idempotency reservation (`reserve`, above), not the folded event-log
    // scan — so the append carries no idempotency key.
    octl_core::append_and_apply_event(&paths, "run.created", None, None, Value::Object(data))
        .map_err(from_core)?;

    // Commit point for a TOP-LEVEL run: `run.created` is durable, so the run is
    // a recoverable `pending` run on disk. Disarm the reservation guard — a
    // later create.sh / supervisor failure must now KEEP the key so a keyed
    // retry short-circuits to this run rather than minting a duplicate. A CHILD
    // stays armed until full success (its run dir is discarded on failure).
    if !is_child {
        if let Some(g) = reservation.as_mut() {
            g.disarm();
        }
    }

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

    // Emit `child.spawned` on the parent's log. This is what makes the parent
    // supervisor discover and adopt the child (§7.2). It is emitted only once
    // the child is known-live: for a materialized child that means AFTER
    // create.sh returns success (see the call site below the spawn); for a
    // `--skip-materialize` skeleton child there is no create.sh that can fail,
    // so the child is live the moment its run dir exists. Either way a failed
    // spawn emits no `child.spawned` and leaves no phantom child on the parent.
    let emit_child_spawned = || -> Result<(), CliError> {
        if !is_child {
            return Ok(());
        }
        let parent_paths = run_paths_exact(&root, parent_run_id_typed.as_ref().expect("is_child"))?;
        let child_data = json!({
            "child_run_id": run_id,
            "child_node_id": "n-0001",
            "child_kind": kind_kebab(args.kind),
            "child_title": title,
        });
        // No idempotency key on the parent's `child.spawned`: `run create`'s
        // own dedup is the CLI-level idempotency reservation (key -> run_id)
        // that short-circuits a retry *before* this point. Passing the key here
        // would instead make `append_and_apply_event` scan the parent log and,
        // on a retry that slipped past the reservation, return an idempotent
        // replay of the *first* child — silently orphaning the freshly
        // generated `run_id`.
        octl_core::append_and_apply_event(
            &parent_paths,
            "child.spawned",
            Some(parent_node_id_typed.as_ref().expect("is_child")),
            None,
            child_data,
        )
        .map_err(from_core)?;
        Ok(())
    };

    // `--skip-materialize` short-circuit: skeleton-only run, used by
    // tests that need a run dir without booting a real worktree/agent.
    if skip_materialize {
        // The skeleton child is live as soon as its run dir exists — emit
        // `child.spawned` here. The idempotency key was already reserved before
        // materialization (see `reserve` above), same as the materialized path.
        emit_child_spawned()?;
        // Commit point for a CHILD: run dir + `child.spawned` are durable, so a
        // keyed retry must replay rather than re-spawn. Disarm the guard (no-op
        // for a top-level orchestrate driver, already disarmed after run.created).
        if is_child {
            if let Some(g) = reservation.as_mut() {
                g.disarm();
            }
        }
        // The orchestrate DRIVER has no worktree to materialize, but it STILL
        // needs a supervisor: its `--kind orchestrated` children are
        // parent-pointed and delegate child-supervisor creation to the parent
        // supervisor (single-arbiter, design §7.2). Without a driver supervisor
        // nobody adopts `child.spawned`, forks the child supervisor, consumes
        // the child's terminal `node.report`, rolls the child up to `done`, or
        // tears down its worktree + tmux window — every orchestrated child
        // hangs in `pending` forever (issue orchestrated-children-hang-pending).
        // Spawn it here, mirroring the materialized top-level path below.
        //
        // The test-only skip hatches still produce a pure skeleton with NO
        // supervisor: `--skip-materialize` and `OCTL_TEST_SKIP_MATERIALIZE`
        // both mean "no real spawn", so an in-process unit test never boots a
        // detached supervisor it would have to reap. Production orchestrate
        // drivers set neither, so they get the real supervisor.
        let spawn_driver_supervisor = args.kind == Kind::Orchestrate
            && !is_child
            && !args.skip_materialize
            && std::env::var("OCTL_TEST_SKIP_MATERIALIZE").is_err();
        // The idempotency key was already reserved before materialization
        // began (see `reserve` above), so the run is durably keyed here: if the
        // supervisor fails to confirm, a keyed retry replays THIS run rather
        // than minting a duplicate.
        let supervisor_pid = if spawn_driver_supervisor {
            Some(spawn_supervisor_or_fail(&paths, &run_id)?)
        } else {
            None
        };
        return emit(EmitInput {
            run_id: &run_id,
            dir: child_dir.display().to_string(),
            kind: args.kind,
            lifecycle,
            parent_run_id: parent_run_id.as_deref(),
            parent_node_id: parent_node_id.as_deref(),
            node_id: driver_node_id,
            spawn: None,
            supervisor_pid,
            idempotent_replay: None,
            dry_run: None,
            spec: args.spec,
            warnings: args.warnings,
        });
    }

    let prompt_path = prompt_path.expect("non-skip path resolves prompt source");
    // Shell out to create.sh. For a TOP-LEVEL run a failure here leaves the
    // run on disk in `pending` with no node — `run cancel` / `run show` still
    // work, and a retry with the same `--idempotency-key` short-circuits. For
    // a CHILD spawn we instead remove the half-built run dir on failure: no
    // `child.spawned` has been emitted yet, so nothing references this run, and
    // leaving it would strand a phantom 0-node child in `pending` (the bug this
    // module fixes). The cleanup is helper-wrapped so both failure arms (the
    // create.sh non-zero exit and the PID-liveness re-check) share it.
    let branch_name = derive_branch_name(args.kind, &run_id, &title);
    let spawn_req = spawn::SpawnRequest {
        kind: kind_kebab(args.kind),
        branch: &branch_name,
        prompt_file: &prompt_path,
        layout: args.layout.as_deref(),
        no_hooks: args.no_hooks,
        keep_tmux_on_error: false,
        parent_session: parent_session.as_deref(),
        agent_startup_timeout: args.agent_startup_timeout,
        // Fork the worktree's branch from the named source branch (e.g. an
        // orchestrate integration branch) rather than workmux's default base.
        // `None` for runs without --source-branch keeps the prior behaviour.
        source_branch: args.source_branch.as_deref(),
        // `run create` inherits the caller's cwd (the user's repo); only the
        // supervisor's retry re-spawn needs an explicit repo.
        cwd: None,
    };
    // On any spawn failure for a child, drop the orphan run dir before
    // returning the error. Best-effort: a leftover dir is far less harmful
    // than a panic mid-error-handling, so a remove failure is swallowed.
    let cleanup_orphan_child = || {
        if is_child {
            let _ = std::fs::remove_dir_all(&child_dir);
            // The child's idempotency key (if any) is freed by the still-armed
            // `ReservationGuard` on this function's error unwind — a child is
            // only disarmed on full success — so a keyed retry re-spawns cleanly
            // instead of replaying a run dir we just discarded. (Top-level spawn
            // failures deliberately KEEP the reservation: the run stays on disk
            // in `pending` and a retry short-circuits to it — top-level is
            // disarmed right after `run.created`.)
        }
    };
    let outcome = match spawn::run_create_sh_with_tmux_retry(&spawn_req) {
        Ok(o) => o,
        Err(e) => {
            cleanup_orphan_child();
            return Err(e);
        }
    };
    // V2: re-verify the discovered PID is still alive. If it died
    // between create.sh's check and ours, emit node.failed (top-level) or
    // remove the orphan child run dir, then return a structured error rather
    // than silently recording a dead PID.
    if let Err(e) = spawn::verify_agent_pid(outcome.agent_pid_hint) {
        if is_child {
            cleanup_orphan_child();
            return Err(e);
        }
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

    // Capture the branch's fork point — the tip the worktree was just created
    // from — as an immutable reference for the supervisor's later merge
    // reconciliation. Right after `create.sh`'s `git worktree add`, the new
    // branch's HEAD *is* the fork base; recording it lets the supervisor
    // distinguish "did work that merged into source" from "never diverged"
    // (issues `false-failed-after-merge` /
    // `supervisor-stuck-pending-after-self-merge`). Best-effort: a git failure
    // leaves `base_sha` null and the reconcile fallback simply does not fire.
    let base_sha = capture_base_sha(&outcome.worktree_path);

    // Emit node.created with the discovered metadata. The reducer
    // creates nodes/n-0001.json with these fields wired in.
    let node_data = json!({
        "kind": kind_kebab(args.kind),
        "branch": outcome.branch,
        "base_sha": base_sha,
        "worktree_path": outcome.worktree_path,
        "tmux_window": outcome.tmux_window,
        // Qualified tmux identity (null on a create.sh that predates the
        // fields); the reducer folds these into Node.tmux_identity.
        "tmux_socket": outcome.tmux_socket,
        "tmux_session": outcome.tmux_session,
        "tmux_window_id": outcome.tmux_window_id,
        "tmux_pane_id": outcome.tmux_pane_id,
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

    // Emit-after-success: only now that create.sh returned a live, verified
    // child does the parent learn about it. Emitting `child.spawned` here
    // (rather than before the spawn) makes the parent's DAG bookkeeping
    // transactional — a failed spawn leaves no parent event and no child run
    // dir, so the parent supervisor never tracks a phantom 0-node child. (On
    // that failure the child's still-armed reservation guard releases the key on
    // unwind — so a keyed retry re-spawns cleanly.)
    emit_child_spawned()?;

    // Commit point for a CHILD (materialized path): the run is materialized and
    // `child.spawned` is durable, so disarm — a keyed retry must now replay.
    // Top-level was already disarmed after `run.created`; its supervisor failure
    // below therefore KEEPS the key so a retry replays this recoverable run.
    if is_child {
        if let Some(g) = reservation.as_mut() {
            g.disarm();
        }
    }

    // For top-level runs, spawn the supervisor and wait for its PID
    // file. Child-spawn delegates supervisor creation to the parent
    // supervisor (design.md §7.2 step 6).
    let supervisor_pid = if is_child {
        None
    } else {
        Some(spawn_supervisor_or_fail(&paths, &run_id)?)
    };

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

/// Spawn the run's supervisor and require it to confirm start, turning a
/// silent boot failure into a loud, actionable error.
///
/// [`supervisor_spawn::spawn_for_run`] returns [`SupervisorSpawn::Unconfirmed`]
/// when the detached supervisor never confirms boot over its readiness pipe —
/// it died during init, reported a structured boot error, or the fork/exec
/// failed — the run would otherwise be left in `pending` with a dead/absent
/// supervisor and its envelope would misreport a bogus success (issue
/// `supervisor-spawn-fails-silently-at-run-create`, suggested-fix #1; the
/// timeout ambiguity itself is retired by `supervisor-confirm-readiness-pipe`).
/// We instead return `supervisor_spawn_failed` carrying the run id, so the
/// caller can inspect `supervisor.stderr.log`, `run reattach`, or `run cancel`
/// rather than hang until its own timeout. The run.created / node.created
/// events are already durable on disk (and the idempotency key, if any, was
/// reserved before this call), so the run is fully recoverable and a keyed
/// retry replays it rather than duplicating it.
fn spawn_supervisor_or_fail(paths: &octl_core::RunPaths, run_id: &str) -> Result<u32, CliError> {
    match supervisor_spawn::spawn_for_run(paths, run_id)? {
        supervisor_spawn::SupervisorSpawn::Confirmed { pid } => Ok(pid),
        supervisor_spawn::SupervisorSpawn::Unconfirmed { reason } => Err(CliError::system(
            "supervisor_spawn_failed",
            format!(
                "supervisor for run {run_id} did not confirm boot ({reason}). The run is on \
                 disk in `pending`. Inspect '{}/supervisor.stderr.log', then \
                 `orchestratectl run reattach {run_id}` to retry (a no-op if one is already \
                 live) or `orchestratectl run cancel {run_id}` to tear it down",
                paths.root.display()
            ),
        )
        .with_invalid_value(run_id)),
    }
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
/// Read the current `HEAD` commit SHA of the freshly-created worktree — the
/// branch's fork point — for [`Node::base_sha`](octl_core::Node). Best-effort:
/// returns `None` if git is unavailable, the path is not a worktree, or the
/// output is not a clean SHA, in which case the supervisor's merge-reconcile
/// fallback simply does not fire for this node. Honors the `GIT_BIN` override so
/// tests can stub (or disable) git the same way the supervisor's teardown does.
pub(crate) fn capture_base_sha(worktree_path: &str) -> Option<String> {
    let git = std::env::var("GIT_BIN").unwrap_or_else(|_| "git".to_string());
    let out = std::process::Command::new(git)
        .arg("-C")
        .arg(worktree_path)
        .args(["rev-parse", "HEAD"])
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
    // A full `rev-parse HEAD` yields exactly a 40-char (SHA-1) or 64-char
    // (SHA-256) hex object id. Require that exact shape — an abbreviated or
    // malformed value would persist a base that silently disables or misfires the
    // reconcile check.
    let ok = matches!(sha.len(), 40 | 64) && sha.chars().all(|c| c.is_ascii_hexdigit());
    ok.then_some(sha)
}

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
