//! `spinoff approve` — emit `spinoff.approved`, optionally materialize
//! an `issuectl` issue.
//!
//! Auto-materialization is best-effort: a missing `issuectl` binary is
//! intentionally silent (the tool is optional), an `issuectl` *failure*
//! surfaces as a warning entry in the success envelope so the caller
//! can decide whether to retry `issuectl new` themselves. The approval
//! is recorded either way.
//!
//! ## Lock-first materialization (atomicity)
//!
//! The whole approve — the "is this proposal still pending?" recheck,
//! the `issuectl new` subprocess, and the `spinoff.approved` append —
//! runs inside ONE hold of the run flock (see [`run`]). The flock is
//! held across the subprocess on purpose: it is the only thing that
//! makes "materialize at most once" true. The earlier design called
//! `issuectl` *before* the lock, so two concurrent approvers both
//! created external tickets — one canonical, one orphan that no
//! projection pointed at. Holding the lock costs the run's event log a
//! few hundred ms per approve; that is the deliberate trade chosen over
//! orphan tickets.
//!
//! ## Idempotency / retry-safety
//!
//! Two layers make a retried approve safe:
//!
//! 1. The lock-time status recheck: an already-`Approved` proposal
//!    returns its persisted slug and never shells out again.
//! 2. A deterministic per-proposal `issuectl --slug` (see
//!    [`derive_materialization_slug`]). `issuectl new` has no
//!    `--idempotency-key`; it instead *refuses* a duplicate `--slug`.
//!    So if a prior approve created the ticket but crashed before
//!    appending `spinoff.approved`, the retry's `issuectl new` collides
//!    on the known slug — we detect that and re-attach the existing
//!    ticket instead of creating a second one.
//!
//! Passing `--issue-slug <slug>` still skips `issuectl` entirely and
//! binds the approval to a known-existing issue.

use std::process::Command;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use octl_core::{
    append_and_apply_unlocked, read_manifest_opt, read_spinoff_opt, Kind, ProposalId, RunLock,
    SpinoffStatus,
};

use crate::error::CliError;
use crate::output::{self, OutputFormat, OutputSpec};
use crate::proc::{run_with_timeout, TimedOutcome};
use crate::run::{from_core, kind_kebab, parse_proposal_id, run_paths};
use crate::spinoff::require_safe_slug;

pub struct Args<'a> {
    pub run_id: String,
    pub proposal_id: String,
    pub issue_slug: Option<String>,
    pub idempotency_key: Option<String>,
    pub dry_run: bool,
    pub spec: &'a OutputSpec,
    pub warnings: &'a [String],
}

#[derive(Serialize)]
struct ApprovePayload {
    run_id: String,
    proposal_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    issue_slug: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seq: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    idempotent_replay: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dry_run: Option<bool>,
}

/// What the locked critical section decided. Returning a typed enum
/// (rather than `Option<u64>`) so the caller never confuses a race-loss
/// for an idempotent replay of its own request — the loser must surface
/// the *persisted* state, not its locally-computed slug.
enum Outcome {
    Applied {
        seq: u64,
        /// The slug recorded on the `spinoff.approved` event — either the
        /// caller's `--issue-slug` or the freshly materialized one. Computed
        /// *inside* the lock (materialization is now lock-held), so it rides
        /// back out in the outcome rather than being known before the lock.
        issue_slug: Option<String>,
        /// Warnings raised by the in-lock `issuectl` materialization (e.g. a
        /// non-idempotent failure). Surfaced on the success envelope.
        warnings: Vec<String>,
    },
    AlreadyApproved {
        issue_slug: Option<String>,
    },
    AlreadyRejected {
        reason: Option<String>,
    },
    /// The proposal vanished between the unlocked pre-check and the
    /// authoritative re-check inside the lock. Race against deletion
    /// or a corrupt projection.
    ProposalNotFound,
    RunNotFound,
}

pub fn run(args: Args<'_>) -> Result<(), CliError> {
    let run_id = args.run_id.clone();
    let proposal_id = parse_proposal_id(&args.proposal_id)?;
    let issue_slug_arg = match args.issue_slug.as_deref() {
        Some(s) => Some(require_safe_slug(s, "issue-slug")?),
        None => None,
    };

    let root = crate::home::root_dir()?;
    let paths = run_paths(&root, &run_id)?;

    if read_manifest_opt(&paths).map_err(from_core)?.is_none() {
        return Err(
            CliError::user("run_not_found", format!("no run with id {run_id}"))
                .with_invalid_value(&run_id),
        );
    }

    let proposal = match read_spinoff_opt(&paths, &proposal_id).map_err(from_core)? {
        Some(p) => p,
        None => {
            return Err(CliError::user(
                "proposal_not_found",
                format!("no proposal with id {proposal_id} in run {run_id}"),
            )
            .with_invalid_value(proposal_id.as_str()));
        }
    };

    // Pre-lock fast-path responses. The lock-time recheck still
    // enforces these invariants authoritatively below.
    match proposal.status {
        SpinoffStatus::Approved => {
            if let Some(err) = mismatch_error(
                &proposal_id,
                issue_slug_arg.as_deref(),
                proposal.accepted_as_issue_slug.as_deref(),
            ) {
                return Err(err);
            }
            return emit_approved(
                &run_id,
                &proposal_id,
                proposal.accepted_as_issue_slug.clone(),
                None,
                Some(true),
                None,
                args.spec,
                args.warnings,
                &[],
            );
        }
        SpinoffStatus::Rejected => {
            return Err(CliError::user(
                "proposal_already_rejected",
                format!("proposal {proposal_id} was already rejected; cannot approve"),
            )
            .with_invalid_value(proposal_id.as_str()));
        }
        SpinoffStatus::Proposed => {}
    }

    // `--dry-run` is answered without touching the lock or `issuectl`: report
    // the slug the caller pinned (or `None` — auto-materialization is a real
    // side effect we will not perform in a plan).
    if args.dry_run {
        return emit_approved(
            &run_id,
            &proposal_id,
            issue_slug_arg.clone(),
            None,
            None,
            Some(true),
            args.spec,
            args.warnings,
            &[],
        );
    }

    // The deterministic external slug used to make `issuectl new` idempotent.
    // Only consumed on the auto-materialize path (no `--issue-slug`), but
    // computed up front so the lock closure borrows a ready value.
    let materialization_slug = derive_materialization_slug(&proposal_id);

    let outcome = RunLock::with_lock(&paths.lock(), || {
        // Re-validate run + proposal under the lock — the unlocked reads
        // above are advisory. A concurrent run-delete or
        // projection-corruption between unlocked check and lock is rare
        // but valid.
        if read_manifest_opt(&paths)?.is_none() {
            return Ok(Outcome::RunNotFound);
        }
        let cur = match read_spinoff_opt(&paths, &proposal_id)? {
            Some(p) => p,
            None => return Ok(Outcome::ProposalNotFound),
        };
        match cur.status {
            SpinoffStatus::Approved => {
                // Already materialized: return the persisted slug and do NOT
                // shell out again. This is the "has it already been
                // materialized?" check — the proposal's own settled status is
                // the dedup, so a retry (or the loser of a race) never makes a
                // second `issuectl new` call.
                return Ok(Outcome::AlreadyApproved {
                    issue_slug: cur.accepted_as_issue_slug.clone(),
                });
            }
            SpinoffStatus::Rejected => {
                return Ok(Outcome::AlreadyRejected {
                    reason: cur.rejected_reason.clone(),
                });
            }
            SpinoffStatus::Proposed => {}
        }

        // LOCK-FIRST MATERIALIZATION. We hold the run flock across the
        // `issuectl new` subprocess deliberately. Materializing *before* the
        // lock (the old design) let two concurrent approvers each create an
        // external ticket — one canonical, one orphan no projection points at.
        // Inside the lock only the proposal's first approver reaches this code
        // (every later caller short-circuits on the `Approved` status above),
        // so at most one ticket is ever created per proposal.
        let mut warnings: Vec<String> = Vec::new();
        let issue_slug: Option<String> = if let Some(s) = &issue_slug_arg {
            Some(s.clone())
        } else {
            match materialize_via_issuectl(
                &cur.proposed_title,
                cur.proposed_kind,
                cur.rationale.as_deref(),
                &materialization_slug,
            ) {
                Ok(Some(slug)) => Some(slug),
                Ok(None) => None,
                Err(w) => {
                    warnings.push(w);
                    None
                }
            }
        };

        let mut data = serde_json::Map::new();
        data.insert("proposal_id".into(), Value::String(proposal_id.to_string()));
        if let Some(s) = &issue_slug {
            data.insert("issue_slug".into(), Value::String(s.clone()));
        }
        let data = Value::Object(data);

        let seq = append_and_apply_unlocked(
            &paths,
            "spinoff.approved",
            None,
            args.idempotency_key.as_deref(),
            data,
        )?;
        Ok(Outcome::Applied {
            seq,
            issue_slug,
            warnings,
        })
    })
    .map_err(from_core)?;

    match outcome {
        Outcome::Applied {
            seq,
            issue_slug,
            warnings,
        } => emit_approved(
            &run_id,
            &proposal_id,
            issue_slug,
            Some(seq),
            None,
            None,
            args.spec,
            args.warnings,
            &warnings,
        ),
        Outcome::AlreadyApproved {
            issue_slug: persisted,
        } => {
            if let Some(err) = mismatch_error(
                &proposal_id,
                issue_slug_arg.as_deref(),
                persisted.as_deref(),
            ) {
                return Err(err);
            }
            emit_approved(
                &run_id,
                &proposal_id,
                persisted,
                None,
                Some(true),
                None,
                args.spec,
                args.warnings,
                &[],
            )
        }
        Outcome::AlreadyRejected { reason } => Err(CliError::user(
            "proposal_already_rejected",
            format!(
                "proposal {proposal_id} was rejected by a concurrent caller \
                 (reason: {reason:?}); cannot approve"
            ),
        )
        .with_invalid_value(proposal_id.as_str())),
        Outcome::ProposalNotFound => Err(CliError::user(
            "proposal_not_found",
            format!("proposal {proposal_id} disappeared from run {run_id}"),
        )
        .with_invalid_value(proposal_id.as_str())),
        Outcome::RunNotFound => Err(CliError::user(
            "run_not_found",
            format!("run {run_id} disappeared"),
        )
        .with_invalid_value(&run_id)),
    }
}

/// If the proposal is already approved and the caller provided an
/// `--issue-slug` that does not match the recorded slug, return a
/// `proposal_already_approved` error. Silent ignores here would let
/// the caller believe their slug was attached when it wasn't.
fn mismatch_error(
    proposal_id: &ProposalId,
    requested: Option<&str>,
    recorded: Option<&str>,
) -> Option<CliError> {
    let requested = requested?;
    if recorded == Some(requested) {
        return None;
    }
    let (message, expected) = match recorded {
        Some(r) => (
            format!(
                "proposal {proposal_id} is already approved with issue-slug \
                 {r:?}; cannot re-approve with a different slug {requested:?}"
            ),
            Value::String(r.to_string()),
        ),
        None => (
            format!(
                "proposal {proposal_id} is already approved without a recorded \
                 issue-slug; cannot bind {requested:?} retroactively"
            ),
            Value::Null,
        ),
    };
    Some(
        CliError::user("proposal_already_approved", message)
            .with_invalid_value(requested)
            .with_expected(expected),
    )
}

/// Default wall-clock bound on the `issuectl new` subprocess. A wedged
/// `issuectl` (network stall, lock contention, deadlock) must not hang the whole
/// `spinoff approve` indefinitely. Override with [`ISSUECTL_TIMEOUT_ENV`].
const ISSUECTL_TIMEOUT_DEFAULT: Duration = Duration::from_secs(30);

/// Env override for [`ISSUECTL_TIMEOUT_DEFAULT`] (whole seconds; an unparseable
/// value keeps the default). Mirrors the watchdog's `OCTL_SPAWN_GRACE_SECS`-style
/// knob — configurable without growing the CLI flag surface.
const ISSUECTL_TIMEOUT_ENV: &str = "ORCHESTRATECTL_ISSUECTL_TIMEOUT_SECS";

/// Cap on captured stdout/stderr from `issuectl`, each. A binary streaming
/// megabytes of output cannot OOM the CLI; anything past this is dropped with a
/// warning and the retained prefix is treated as a partial capture.
const ISSUECTL_OUTPUT_CAP: usize = 1 << 20; // 1 MiB

/// Env override for the `issuectl` binary path. Tests install a fake; mirrors the
/// `GIT_BIN` / `TMUX_BIN` overrides elsewhere in the CLI.
const ISSUECTL_BIN_ENV: &str = "ISSUECTL_BIN";

/// Map a spin-off `proposed_kind` to the `issuectl --type` value that best fits.
/// `issuectl` accepts `bug | task | feature | improvement | chore | epic`;
/// orchestratectl's *workflow* kinds don't line up 1:1 with issue *types*, so
/// each maps to the closest sensible type. The match is exhaustive — a new
/// [`Kind`] fails to compile until it is mapped here, so this cannot silently
/// drift to the old hardcoded `feature`.
fn issuectl_type(kind: Kind) -> &'static str {
    match kind {
        // The one exact fit: a bug investigation is a bug.
        Kind::Bugfix => "bug",
        // Coding / skill-authoring / orchestrated feature work.
        Kind::Code | Kind::MakeSkill | Kind::Orchestrated | Kind::Orchestrate => "feature",
        // A fire-and-forget spin-off is typically a small enhancement.
        Kind::Spinoff => "improvement",
        // Scoped investigation / decision / fan-out units.
        Kind::Research | Kind::TechnicalDecision | Kind::FanOut => "task",
    }
}

/// Effective `issuectl` timeout: [`ISSUECTL_TIMEOUT_DEFAULT`] unless
/// [`ISSUECTL_TIMEOUT_ENV`] overrides it with a parseable whole-second count.
fn issuectl_timeout() -> Duration {
    match std::env::var(ISSUECTL_TIMEOUT_ENV) {
        Ok(v) => v
            .trim()
            .parse::<u64>()
            .map_or(ISSUECTL_TIMEOUT_DEFAULT, Duration::from_secs),
        Err(_) => ISSUECTL_TIMEOUT_DEFAULT,
    }
}

/// The directory `issuectl` runs in — where it expects to find `issues/`. We set
/// the child's `current_dir` explicitly rather than relying on the inherited cwd
/// so the spawn is deterministic. orchestratectl is invoked from the repo that
/// owns the issues, so the process cwd is the right root; `issuectl` walks up
/// from there to locate `issues/`.
fn workspace_dir() -> Option<std::path::PathBuf> {
    std::env::current_dir().ok()
}

/// Build `issuectl`'s environment explicitly instead of inheriting the parent's
/// (which may carry unrelated secrets). Pass through only what a CLI legitimately
/// needs: the executable search path, the user's identity/home (issuectl writes
/// files and may shell out to git), locale, timezone, and the temp dir. Every
/// `LC_*` is forwarded since that set is open-ended.
fn scrub_env(cmd: &mut Command) {
    const PASS: &[&str] = &["PATH", "HOME", "USER", "LOGNAME", "LANG", "TZ", "TMPDIR"];
    cmd.env_clear();
    for key in PASS {
        if let Some(val) = std::env::var_os(key) {
            cmd.env(key, val);
        }
    }
    for (key, val) in std::env::vars_os() {
        if key.to_str().is_some_and(|k| k.starts_with("LC_")) {
            cmd.env(key, val);
        }
    }
}

/// A short, length-bounded preview of captured bytes for an error message — so a
/// near-cap blob isn't echoed verbatim into a warning.
fn preview(bytes: &[u8]) -> String {
    const MAX_CHARS: usize = 512;
    let s = String::from_utf8_lossy(bytes);
    if s.chars().count() > MAX_CHARS {
        let head: String = s.chars().take(MAX_CHARS).collect();
        format!("{head}… ({} bytes total)", bytes.len())
    } else {
        s.into_owned()
    }
}

/// The deterministic external slug for a proposal's `issuectl` ticket.
///
/// Keyed on the proposal id (globally unique — a ULID or sha-prefix body), so
/// it is stable across retries of the same approve and distinct across
/// proposals. Stability is the whole point: it lets `issuectl new --slug` act
/// as an idempotency key even though `issuectl` has no `--idempotency-key`
/// flag. The `s-` prefix is swapped for `spinoff-` so the external slug reads
/// as a slug, not as an internal id; the body stays `[a-z0-9]`, so the result
/// is valid kebab-case.
fn derive_materialization_slug(proposal_id: &ProposalId) -> String {
    let body = proposal_id
        .as_str()
        .strip_prefix("s-")
        .unwrap_or(proposal_id.as_str());
    format!("spinoff-{body}")
}

/// Does `issuectl new`'s stderr signal "this slug already exists"?
///
/// `issuectl new` rejects a duplicate `--slug` with a `command-failed` error
/// whose message contains `already exists` (verified against its actual
/// output). It carries no machine-stable code distinguishing this from other
/// `command-failed` errors, so we match the message substring. The coupling to
/// issuectl's wording is deliberate and localized here; if `issuectl new` ever
/// grows `--idempotency-key`, prefer that and delete this.
fn stderr_signals_slug_exists(stderr: &[u8]) -> bool {
    String::from_utf8_lossy(stderr).contains("already exists")
}

/// Try to materialize an issue via `issuectl new`. Returns:
///
/// - `Ok(Some(slug))` on success.
/// - `Ok(None)` if `issuectl` is not on PATH — silent because
///   `issuectl` is intentionally optional.
/// - `Err(warning)` if `issuectl` was found but failed (including a timeout);
///   the caller attaches the message to the response `warnings` array.
///
/// `slug` is the deterministic per-proposal slug from
/// [`derive_materialization_slug`]: passing it as `issuectl new --slug` is what
/// makes materialization idempotent (see the module-level "Idempotency" note).
fn materialize_via_issuectl(
    title: &str,
    kind: Kind,
    rationale: Option<&str>,
    slug: &str,
) -> Result<Option<String>, String> {
    let bin = std::env::var(ISSUECTL_BIN_ENV).unwrap_or_else(|_| "issuectl".to_string());
    materialize_via_issuectl_with(
        &bin,
        title,
        kind,
        rationale,
        slug,
        issuectl_timeout(),
        ISSUECTL_OUTPUT_CAP,
    )
}

/// Testable core of [`materialize_via_issuectl`]: explicit binary, timeout, and
/// cap (the public wrapper resolves these from env / constants).
fn materialize_via_issuectl_with(
    bin: &str,
    title: &str,
    kind: Kind,
    rationale: Option<&str>,
    slug: &str,
    timeout: Duration,
    cap: usize,
) -> Result<Option<String>, String> {
    let description = rationale.map_or_else(
        || {
            format!(
                "Auto-materialized spin-off ({}) approved via orchestratectl.",
                kind_kebab(kind)
            )
        },
        str::to_string,
    );
    let issue_type = issuectl_type(kind);

    let mut cmd = Command::new(bin);
    cmd.args(["--json", "new", "--type", issue_type]);
    cmd.args(["--title", title, "--description", &description]);
    // The deterministic slug is the idempotency mechanism: `issuectl new` has no
    // `--idempotency-key`, so a stable `--slug` per proposal is how a retry
    // collides with (instead of duplicating) an already-created ticket.
    cmd.args(["--slug", slug]);
    scrub_env(&mut cmd);
    if let Some(dir) = workspace_dir() {
        cmd.current_dir(dir);
    }

    let (status, stdout, stderr) = match run_with_timeout(cmd, timeout, cap) {
        TimedOutcome::Exited {
            status,
            stdout,
            stderr,
        } => (status, stdout, stderr),
        TimedOutcome::TimedOut => {
            return Err(format!(
                "issuectl timed out after {}s and was killed; no issue created",
                timeout.as_secs()
            ));
        }
        TimedOutcome::SpawnErr(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        TimedOutcome::SpawnErr(e) => return Err(format!("issuectl spawn failed: {e}")),
    };

    if stdout.truncated {
        tracing::warn!(cap, "issuectl stdout exceeded cap; output truncated");
    }
    if stderr.truncated {
        tracing::warn!(cap, "issuectl stderr exceeded cap; output truncated");
    }

    if !status.success() {
        // Idempotent recovery: a non-zero exit whose error says the slug
        // already exists means a *prior* approve for this proposal already
        // created the ticket (and then crashed before appending
        // `spinoff.approved`). Because the slug is deterministic, that ticket
        // is the one we want — re-attach it instead of surfacing an error or
        // creating a duplicate.
        if stderr_signals_slug_exists(&stderr.bytes) {
            return Ok(Some(slug.to_string()));
        }
        return Err(format!(
            "issuectl exited {:?}: {}{}",
            status.code(),
            preview(&stderr.bytes).trim(),
            if stderr.truncated {
                " (stderr truncated at cap)"
            } else {
                ""
            }
        ));
    }
    let v: Value = serde_json::from_slice(&stdout.bytes).map_err(|e| {
        format!(
            "issuectl returned non-JSON: {e}{}; raw: {}",
            if stdout.truncated {
                " (stdout truncated at cap)"
            } else {
                ""
            },
            preview(&stdout.bytes)
        )
    })?;
    match v.get("slug").and_then(Value::as_str) {
        Some(s) => Ok(Some(s.to_string())),
        None => Err(format!("issuectl JSON missing `slug` field: {v}")),
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_approved(
    run_id: &str,
    proposal_id: &ProposalId,
    issue_slug: Option<String>,
    seq: Option<u64>,
    idempotent_replay: Option<bool>,
    dry_run: Option<bool>,
    spec: &OutputSpec,
    warnings: &[String],
    local_warnings: &[String],
) -> Result<(), CliError> {
    let payload = ApprovePayload {
        run_id: run_id.to_string(),
        proposal_id: proposal_id.to_string(),
        issue_slug: issue_slug.clone(),
        seq,
        idempotent_replay,
        dry_run,
    };
    match spec.format {
        OutputFormat::Json | OutputFormat::Jsonl => {
            let merged: Vec<String> = warnings
                .iter()
                .cloned()
                .chain(local_warnings.iter().cloned())
                .collect();
            output::emit_envelope(&payload, spec, &merged)?;
        }
        OutputFormat::Text => {
            println!("run-id:      {}", payload.run_id);
            println!("proposal-id: {}", payload.proposal_id);
            if let Some(s) = &payload.issue_slug {
                println!("issue-slug:  {s}");
            }
            match payload.seq {
                Some(s) => println!("seq:         {s}"),
                None if payload.dry_run == Some(true) => {
                    println!("seq:         (assigned on apply)");
                }
                None => println!("seq:         (no-op; already approved)"),
            }
            if payload.dry_run == Some(true) {
                println!("note:        --dry-run (no filesystem changes)");
            }
            if payload.idempotent_replay == Some(true) {
                println!("note:        idempotent replay (already approved)");
            }
            output::emit_text_warnings(local_warnings);
            output::emit_text_warnings(warnings);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn chmod_exec(p: &Path) {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(p).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(p, perms).unwrap();
    }

    /// Write an executable fake `issuectl` whose body is `script`, and return its
    /// path. The script runs under `/bin/sh`.
    fn fake_issuectl(dir: &Path, script: &str) -> PathBuf {
        let p = dir.join("fake-issuectl.sh");
        std::fs::write(&p, format!("#!/bin/sh\n{script}\n")).unwrap();
        chmod_exec(&p);
        p
    }

    // ---- proposed_kind → --type tabletop --------------------------------

    #[test]
    fn issuectl_type_maps_every_kind() {
        assert_eq!(issuectl_type(Kind::Bugfix), "bug");
        assert_eq!(issuectl_type(Kind::Code), "feature");
        assert_eq!(issuectl_type(Kind::MakeSkill), "feature");
        assert_eq!(issuectl_type(Kind::Orchestrated), "feature");
        assert_eq!(issuectl_type(Kind::Orchestrate), "feature");
        assert_eq!(issuectl_type(Kind::Spinoff), "improvement");
        assert_eq!(issuectl_type(Kind::Research), "task");
        assert_eq!(issuectl_type(Kind::TechnicalDecision), "task");
        assert_eq!(issuectl_type(Kind::FanOut), "task");
        // Every mapped value must be one issuectl accepts.
        const ACCEPTED: &[&str] = &["bug", "task", "feature", "improvement", "chore", "epic"];
        for kind in [
            Kind::Bugfix,
            Kind::Code,
            Kind::MakeSkill,
            Kind::Orchestrated,
            Kind::Orchestrate,
            Kind::Spinoff,
            Kind::Research,
            Kind::TechnicalDecision,
            Kind::FanOut,
        ] {
            assert!(
                ACCEPTED.contains(&issuectl_type(kind)),
                "{kind:?} maps to a type issuectl rejects"
            );
        }
    }

    #[test]
    fn passes_mapped_type_to_issuectl() {
        let dir = tempfile::TempDir::new().unwrap();
        // Echo argv (NUL-joined) so we can assert the --type without quoting
        // headaches, then emit a valid slug envelope.
        let bin = fake_issuectl(
            dir.path(),
            &format!(
                "printf '%s\\0' \"$@\" > {args:?}\necho '{{\"slug\":\"x\"}}'",
                args = dir.path().join("argv")
            ),
        );
        let got = materialize_via_issuectl_with(
            bin.to_str().unwrap(),
            "Fix the bug",
            Kind::Bugfix,
            None,
            "spinoff-01aaaaaaaaaaaaaaaaaaaaaaaa",
            Duration::from_secs(10),
            1 << 20,
        );
        assert_eq!(got, Ok(Some("x".to_string())));
        let argv = std::fs::read(dir.path().join("argv")).unwrap();
        let argv: Vec<String> = argv
            .split(|b| *b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| String::from_utf8_lossy(s).into_owned())
            .collect();
        // `--type bug` must appear as adjacent tokens.
        assert!(
            argv.windows(2)
                .any(|w| w == ["--type".to_string(), "bug".to_string()]),
            "expected `--type bug`, got {argv:?}"
        );
        // The deterministic slug must be forwarded as `--slug <slug>` — this is
        // the idempotency key for `issuectl new`.
        assert!(
            argv.windows(2).any(|w| w
                == [
                    "--slug".to_string(),
                    "spinoff-01aaaaaaaaaaaaaaaaaaaaaaaa".to_string()
                ]),
            "expected `--slug spinoff-01aaaaaaaaaaaaaaaaaaaaaaaa`, got {argv:?}"
        );
    }

    // ---- Timeout --------------------------------------------------------

    #[test]
    fn timeout_fires_and_returns_structured_error() {
        let dir = tempfile::TempDir::new().unwrap();
        // A child that would sleep far past the deadline — and forks a grandchild
        // so the group-kill path is exercised.
        let bin = fake_issuectl(dir.path(), "sleep 30 & sleep 30");
        let start = std::time::Instant::now();
        let got = materialize_via_issuectl_with(
            bin.to_str().unwrap(),
            "stuck",
            Kind::Spinoff,
            None,
            "spinoff-stuck",
            Duration::from_millis(150),
            1 << 20,
        );
        // Structured error, not a panic and not a hang.
        let err = got.expect_err("a wedged issuectl must surface an error");
        assert!(err.contains("timed out"), "unexpected error: {err}");
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "timeout must fire promptly, not wait for the child"
        );
    }

    // ---- Output cap -----------------------------------------------------

    #[test]
    fn oversized_output_is_capped_and_flagged() {
        let dir = tempfile::TempDir::new().unwrap();
        // Emit ~256 KiB of 'A' (no slug JSON) and exit 0.
        let bin = fake_issuectl(dir.path(), "head -c 262144 /dev/zero | tr '\\0' A");
        let got = materialize_via_issuectl_with(
            bin.to_str().unwrap(),
            "noisy",
            Kind::Spinoff,
            None,
            "spinoff-noisy",
            Duration::from_secs(10),
            1024, // cap well below the produced output
        );
        let err = got.expect_err("non-JSON capped output must be an error");
        // The capture truncated (not OOM) and the error says so.
        assert!(
            err.contains("stdout truncated at cap"),
            "expected truncation note, got: {err}"
        );
        // The preview is bounded — the full 256 KiB is never echoed.
        assert!(err.len() < 4096, "error message should not echo the blob");
    }

    // ---- Success / not-found --------------------------------------------

    #[test]
    fn happy_path_extracts_slug() {
        let dir = tempfile::TempDir::new().unwrap();
        let bin = fake_issuectl(dir.path(), "echo '{\"slug\":\"login-redirect-loops\"}'");
        let got = materialize_via_issuectl_with(
            bin.to_str().unwrap(),
            "Login redirect loops",
            Kind::Code,
            None,
            "spinoff-login",
            Duration::from_secs(10),
            1 << 20,
        );
        assert_eq!(got, Ok(Some("login-redirect-loops".to_string())));
    }

    #[test]
    fn missing_binary_is_silent_none() {
        let got = materialize_via_issuectl_with(
            "/nonexistent/issuectl-no-such-binary",
            "x",
            Kind::Spinoff,
            None,
            "spinoff-x",
            Duration::from_secs(5),
            1 << 20,
        );
        assert_eq!(
            got,
            Ok(None),
            "an absent issuectl is optional, not an error"
        );
    }

    #[test]
    fn nonzero_exit_surfaces_stderr() {
        let dir = tempfile::TempDir::new().unwrap();
        let bin = fake_issuectl(dir.path(), "echo 'boom: lock held' 1>&2; exit 3");
        let got = materialize_via_issuectl_with(
            bin.to_str().unwrap(),
            "x",
            Kind::Spinoff,
            None,
            "spinoff-x",
            Duration::from_secs(10),
            1 << 20,
        );
        let err = got.expect_err("non-zero exit is an error");
        assert!(
            err.contains("boom: lock held"),
            "stderr not surfaced: {err}"
        );
        assert!(err.contains("Some(3)"), "exit code not surfaced: {err}");
    }

    // ---- Idempotent recovery (slug already exists) ----------------------

    #[test]
    fn slug_already_exists_is_idempotent_success() {
        // A prior approve created the ticket then crashed before appending
        // `spinoff.approved`. The retry's `issuectl new --slug <det>` collides:
        // issuectl exits non-zero with an `already exists` message. We must
        // recover — return the known slug, NOT an error, and NOT create a
        // duplicate.
        let dir = tempfile::TempDir::new().unwrap();
        let bin = fake_issuectl(
            dir.path(),
            "echo '{\"error\":{\"code\":\"command-failed\",\"message\":\"slug \\\"spinoff-dup\\\" already exists\"}}' 1>&2; exit 1",
        );
        let got = materialize_via_issuectl_with(
            bin.to_str().unwrap(),
            "dup",
            Kind::Spinoff,
            None,
            "spinoff-dup",
            Duration::from_secs(10),
            1 << 20,
        );
        assert_eq!(
            got,
            Ok(Some("spinoff-dup".to_string())),
            "an already-existing slug must recover to that slug, not error"
        );
    }

    #[test]
    fn derive_materialization_slug_is_stable_and_kebab() {
        let id = ProposalId::parse_str("s-01aaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
        let slug = derive_materialization_slug(&id);
        assert_eq!(slug, "spinoff-01aaaaaaaaaaaaaaaaaaaaaaaa");
        // Deterministic: same id → same slug (the idempotency contract).
        assert_eq!(slug, derive_materialization_slug(&id));
        // Valid kebab-case so issuectl / require_safe_slug accept it.
        assert!(crate::spinoff::require_safe_slug(&slug, "slug").is_ok());
    }
}
