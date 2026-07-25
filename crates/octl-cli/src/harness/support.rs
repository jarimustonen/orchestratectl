//! Shared machinery for the git-inspecting [`CodeHarness`] adapters
//! (`aider`, `claude`, `claude-deepseek`, …).
//!
//! Every adapter in this family follows the same shape (design.md §10): shell
//! out to a coding agent non-interactively in a worktree forked at
//! `base_commit`, let it edit files and **commit** (never merge), then read the
//! outcome from the resulting *git* state — never from tool prose. The parts
//! that differ per tool (which binary, which flags, how the brief is passed,
//! how usage is reported, which credential must be present) are captured by the
//! [`AgentLaunch`] trait; everything else — the dirty/base preconditions, the
//! bounded subprocess run, the git-outcome mapping, the self-check phase, and
//! transcript capture — lives here once so the adapters cannot drift apart.
//!
//! [`run_chunk`] is the whole skeleton; an adapter's `run_chunk` is a one-line
//! delegation to it (see [`super::aider::AiderHarness`]).

use std::path::{Path, PathBuf};
use std::process::Command;

use super::{
    CancelToken, Check, CheckResult, ChunkOutcome, ChunkRequest, ChunkResult, HarnessCapabilities,
    HarnessError, Usage, HARNESS_CONTRACT_VERSION,
};
use crate::proc::{run_with_control, CappedStream, ControlledOutcome, StopReason};

/// Cap on captured output (agent transcript, per-check stdout/stderr) so a noisy
/// tool cannot exhaust memory or bloat the serialized provenance. Generous enough
/// that a normal transcript is retained whole; a runaway producer is truncated.
pub(super) const OUTPUT_CAP: usize = 8 * 1024 * 1024;

/// The tool-specific half of a git-inspecting adapter. Implementors describe
/// *how* to launch their agent and read its usage; [`run_chunk`] supplies the
/// harness-neutral *what* and *where* (design §10).
pub(super) trait AgentLaunch {
    /// What this adapter supports, surfaced verbatim as [`CodeHarness::capabilities`].
    ///
    /// [`CodeHarness::capabilities`]: super::CodeHarness::capabilities
    fn capabilities(&self) -> HarnessCapabilities;

    /// Whether the adapter expects the agent to commit its own edits in-process.
    /// Returns `true` when the agent is relied on to `git commit` itself — the
    /// Claude family and pi, which [`commit_framed_prompt`] instructs to do so.
    /// Returns `false` when the adapter must commit the agent's leftovers itself:
    /// aider is *expected* to auto-commit but a live run can finish with edits left
    /// uncommitted (aider's auto-commit disabled by its own heuristics/config).
    ///
    /// When `false`, [`run_chunk`] commits any edits the agent left after a clean
    /// exit — landing a dirty tree as a real commit so the git-outcome mapping
    /// yields [`ChunkOutcome::Committed`] instead of [`ChunkOutcome::Failed`],
    /// matching the contract the committing adapters already satisfy (design §10:
    /// the adapter presents a committed result read from git). Default `true`.
    ///
    /// [`ChunkOutcome::Committed`]: super::ChunkOutcome::Committed
    /// [`ChunkOutcome::Failed`]: super::ChunkOutcome::Failed
    fn commits_in_agent(&self) -> bool {
        true
    }

    /// Verify any credential the provider needs is present, failing fast with
    /// [`HarnessError::MissingCredential`] before the agent is spawned. Adapters
    /// that source their own credentials (or use an ambient login) return
    /// `Ok(())`.
    fn check_credentials(&self) -> Result<(), HarnessError>;

    /// The full instruction text the agent executes. Written verbatim to the
    /// per-attempt brief file for provenance, and — for tools that take the
    /// prompt on argv — handed to [`build_command`](AgentLaunch::build_command).
    /// An adapter may return `req.brief` unchanged (aider) or wrap it with
    /// implement/self-check/commit framing (the Claude family).
    fn build_prompt(&self, req: &ChunkRequest) -> String;

    /// Build the (unspawned) command that launches the agent in `worktree`.
    /// `brief_file` holds `prompt` on disk; the adapter passes whichever it
    /// needs (a `--message-file <brief_file>` flag, or `prompt` as an argv
    /// token). stdin/stdout/stderr and the process group are set authoritatively
    /// by [`run_with_control`]; do not override them here.
    fn build_command(
        &self,
        worktree: &Path,
        brief_file: &Path,
        prompt: &str,
        req: &ChunkRequest,
    ) -> Command;

    /// Best-effort token/cost extraction from the captured transcript. Never
    /// fails (usage is provenance, not a gate — design §10); return `None` when
    /// nothing could be read.
    fn parse_usage(&self, transcript: &str) -> Option<Usage>;

    /// Short human label for this tool, used in `Failed` reasons ("aider left
    /// uncommitted changes…"). Lowercase binary-ish name (e.g. `"aider"`).
    fn tool_label(&self) -> &'static str;

    /// The binary invocation for the [`HarnessError::ProviderFailure`] spawn
    /// message (e.g. the resolved `OCTL_AIDER_BIN`).
    fn bin_display(&self) -> String;
}

/// Wrap `req.brief` with the implement / self-check / **commit** framing the
/// general-purpose coding agents (Claude Code, pi) need — aider auto-commits and
/// so returns its brief verbatim instead. The agent edits files in the worktree,
/// runs each requested [`Check`] before committing, and finishes with a single
/// commit on the current branch (never a push/merge). The harness still reads the
/// outcome from git, so this framing is guidance, not a contract the agent could
/// violate to fake success (design §10).
pub(super) fn commit_framed_prompt(req: &ChunkRequest) -> String {
    use std::fmt::Write as _;
    let mut p = String::new();
    p.push_str(req.brief.trim_end());
    p.push_str("\n\n---\n\n");
    p.push_str(
        "You are running NON-INTERACTIVELY inside a throwaway git worktree. \
         Implement the task above by editing files in the current working directory.\n\n",
    );
    if !req.checks.is_empty() {
        p.push_str(
            "Before you commit, run each of these self-check commands and make sure it passes:\n",
        );
        for c in &req.checks {
            // `write!` to a String is infallible.
            let _ = writeln!(p, "  - {} — `{}`", c.desc, c.run);
        }
        p.push('\n');
    }
    p.push_str(
        "When the work is complete, stage and commit ALL of your changes on the CURRENT branch \
         with `git add -A && git commit`. Do NOT push and do NOT merge — commit only. \
         If there is genuinely nothing to change, make no commit.\n",
    );
    p
}

/// Whether a credential env var is present **and non-empty**. `std::env::var`
/// (and hence a bare `.is_ok()`) treats `VAR=""` as present, which would sail
/// past a fast-fail check and hand an empty key to the provider — exactly the
/// opaque failure the check exists to pre-empt. Uses `var_os` so a non-UTF-8
/// value still counts as present (the child, not us, consumes it).
pub(super) fn credential_present(var: &str) -> bool {
    std::env::var_os(var).is_some_and(|v| !v.is_empty())
}

/// `git` binary, honouring the crate-wide `GIT_BIN` override (mirrors
/// `supervise::cleanup::git_bin`).
pub(super) fn git_bin() -> String {
    std::env::var("GIT_BIN").unwrap_or_else(|_| "git".to_string())
}

/// Best-effort [`Usage`] extraction from a JSON-emitting agent's transcript,
/// shared by the Claude family and pi (both run their agent with a
/// machine-readable `--output-format json` / `--mode json`). The transcript is
/// stdout-then-stderr concatenated, so we can't assume the whole blob is one JSON
/// value: scan each `{`-leading line for a parseable object (then the whole
/// trimmed transcript as a fallback) and lift usage from either a nested
/// `usage.{input,output}_tokens` block or flat top-level token/cost keys. Handles
/// the common field spellings across agents (`total_cost_usd`/`cost_usd`/`cost`).
///
/// Selection matters for **streaming** agents (pi's `--mode json` emits an event
/// stream where early events carry partial/cumulative usage and the final total
/// lands last): prefer the terminal `type == "result"` summary object (both
/// Claude Code and pi tag it that way), and otherwise take the *last*
/// usage-bearing object — never the first, which would undercount. Never fails —
/// usage is provenance, not a gate (design §10).
pub(super) fn parse_json_usage(transcript: &str) -> Option<Usage> {
    let objects: Vec<serde_json::Value> = transcript
        .lines()
        .filter_map(|l| {
            let t = l.trim();
            t.starts_with('{')
                .then(|| serde_json::from_str::<serde_json::Value>(t).ok())
                .flatten()
        })
        .collect();

    let pick = |objs: &[serde_json::Value]| {
        // The terminal result summary wins; else the last object with any usage.
        objs.iter()
            .rev()
            .find(|v| v.get("type").and_then(serde_json::Value::as_str) == Some("result"))
            .and_then(usage_from_value)
            .or_else(|| objs.iter().rev().find_map(usage_from_value))
    };

    pick(&objects).or_else(|| {
        // Whole-transcript fallback: a single JSON blob with no surrounding prose
        // (e.g. one-line output captured with no trailing newline).
        serde_json::from_str::<serde_json::Value>(transcript.trim())
            .ok()
            .as_ref()
            .and_then(usage_from_value)
    })
}

/// Lift a [`Usage`] out of one parsed agent result object, or `None` when it
/// carries no recognisable token/cost fields. Looks in a nested `usage` block
/// first, then at the top level, so both `{"usage":{"input_tokens":…}}` and a
/// flat `{"input_tokens":…}` shape are covered.
fn usage_from_value(v: &serde_json::Value) -> Option<Usage> {
    let u = v.get("usage").unwrap_or(v);
    let get_u64 = |obj: &serde_json::Value, keys: &[&str]| {
        keys.iter()
            .find_map(|k| obj.get(*k).and_then(serde_json::Value::as_u64))
    };
    let get_f64 = |obj: &serde_json::Value, keys: &[&str]| {
        keys.iter()
            .find_map(|k| obj.get(*k).and_then(serde_json::Value::as_f64))
    };
    let input = get_u64(u, &["input_tokens", "prompt_tokens"]);
    let output = get_u64(u, &["output_tokens", "completion_tokens"]);
    // Cost may live at the top level (Claude's `total_cost_usd`) or in the usage
    // block — check both, top level first.
    let cost = get_f64(v, &["total_cost_usd", "cost_usd", "cost"])
        .or_else(|| get_f64(u, &["total_cost_usd", "cost_usd", "cost"]));

    if input.is_none() && output.is_none() && cost.is_none() {
        return None;
    }
    // An explicit combined total wins; else sum the two, guarding against a
    // hostile/absurd JSON value overflowing (panics in debug, wraps in release).
    let total = get_u64(u, &["total_tokens", "tokens"])
        .or_else(|| input.and_then(|i| output.and_then(|o| i.checked_add(o))));

    Some(Usage {
        input_tokens: input,
        output_tokens: output,
        total_tokens: total,
        cost_usd: cost,
    })
}

/// Run a git subcommand in `worktree`, returning trimmed stdout on success.
fn git(worktree: &Path, args: &[&str]) -> Result<String, HarnessError> {
    let out = Command::new(git_bin())
        .arg("-C")
        .arg(worktree)
        .args(args)
        .output()
        .map_err(|e| HarnessError::InvalidWorktree {
            message: format!("could not run git in {}: {e}", worktree.display()),
        })?;
    if !out.status.success() {
        return Err(HarnessError::InvalidWorktree {
            message: format!(
                "git {} failed in {}: {}",
                args.join(" "),
                worktree.display(),
                String::from_utf8_lossy(&out.stderr).trim()
            ),
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Land any uncommitted edits the agent left in `worktree` as a single commit,
/// for an adapter that does not commit in-process ([`AgentLaunch::commits_in_agent`]
/// is `false` — aider). Best-effort: on a clean tree it is a no-op (the agent
/// already committed); a `git` failure is logged and swallowed so the caller's
/// git-outcome mapping stays the single source of truth (a still-dirty tree then
/// maps to [`ChunkOutcome::Failed`] as before).
///
/// The staging pathspec excludes `.aider*`: the only non-committing adapter is
/// aider, which litters the worktree with `.aider.chat.history.md` /
/// `.aider.tags.cache.*` scratch files. Excluding them keeps the chunk commit to
/// the agent's real deliverables no matter whether the repo's own `.gitignore`
/// happens to cover them (it need not) — the pathspec is a harmless no-op in any
/// repo without such files.
///
/// The commit itself is deterministic: an explicit committer identity, `--no-verify`
/// and `--no-gpg-sign` mean a machine-generated chunk commit never depends on
/// ambient git config, a pre-commit hook, or gpg-signing setup — any of which
/// would fail the commit and wrongly map a good run to `Failed`. This mirrors
/// aider's own `--no-verify` auto-commit behaviour.
fn commit_leftover_changes(worktree: &Path, req: &ChunkRequest, tool: &str) {
    match worktree_status(worktree) {
        // Clean tree — the agent's own auto-commit already ran; nothing to do.
        Ok(status) if status.is_empty() => return,
        Ok(_) => {}
        Err(e) => {
            tracing::warn!(error = %e, tool, "could not read worktree status before adapter commit");
            return;
        }
    }
    if let Err(e) = git(worktree, &["add", "-A", "--", ".", ":(exclude).aider*"]) {
        tracing::warn!(error = %e, tool, "adapter `git add` failed; leaving tree for the Failed mapping");
        return;
    }
    let message = format!(
        "{tool}: chunk {chunk} attempt {attempt} (adapter-committed)",
        chunk = req.chunk_id,
        attempt = req.attempt_id,
    );
    let args = [
        "-c",
        "user.name=orchestratectl",
        "-c",
        "user.email=orchestratectl@localhost",
        "commit",
        "-q",
        "--no-verify",
        "--no-gpg-sign",
        "-m",
        message.as_str(),
    ];
    if let Err(e) = git(worktree, &args) {
        tracing::warn!(error = %e, tool, "adapter `git commit` failed; leaving tree for the Failed mapping");
    }
}

/// The current `HEAD` oid, validated to a full hex object id.
fn head_oid(worktree: &Path) -> Result<String, HarnessError> {
    let sha = git(worktree, &["rev-parse", "HEAD"])?;
    let ok = matches!(sha.len(), 40 | 64) && sha.chars().all(|c| c.is_ascii_hexdigit());
    if ok {
        Ok(sha)
    } else {
        Err(HarnessError::InvalidWorktree {
            message: format!("`git rev-parse HEAD` returned a non-oid value: {sha:?}"),
        })
    }
}

/// Whether the worktree has uncommitted changes (tracked or untracked).
fn worktree_status(worktree: &Path) -> Result<String, HarnessError> {
    git(worktree, &["status", "--porcelain"])
}

/// Files changed between two commits. Uses `-z` (NUL-delimited) so paths with
/// newlines/tabs/spaces survive intact. Propagates a git failure rather than
/// masking it as an empty diff: a `Committed` result with a silently-empty file
/// list would corrupt provenance (design §7).
fn changed_files(worktree: &Path, base: &str, head: &str) -> Result<Vec<PathBuf>, HarnessError> {
    let out = git(
        worktree,
        &["diff", "--name-only", "-z", &format!("{base}..{head}")],
    )?;
    Ok(out
        .split('\0')
        .filter(|l| !l.is_empty())
        .map(PathBuf::from)
        .collect())
}

/// Whether `ancestor` is an ancestor of `descendant` (i.e. HEAD only moved
/// forward). `git merge-base --is-ancestor` exits 0 for yes, 1 for no; only a
/// spawn/other failure is an error.
fn is_ancestor(worktree: &Path, ancestor: &str, descendant: &str) -> Result<bool, HarnessError> {
    let out = Command::new(git_bin())
        .arg("-C")
        .arg(worktree)
        .args(["merge-base", "--is-ancestor", ancestor, descendant])
        .output()
        .map_err(|e| HarnessError::InvalidWorktree {
            message: format!(
                "could not run git merge-base in {}: {e}",
                worktree.display()
            ),
        })?;
    match out.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(HarnessError::InvalidWorktree {
            message: format!(
                "git merge-base --is-ancestor failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ),
        }),
    }
}

/// Resolve a revision to a canonical commit oid, verifying it exists and is a
/// commit (`git rev-parse --verify <rev>^{commit}`).
fn resolve_commit(worktree: &Path, rev: &str) -> Result<String, HarnessError> {
    git(
        worktree,
        &["rev-parse", "--verify", &format!("{rev}^{{commit}}")],
    )
    .map_err(|_| HarnessError::InvalidWorktree {
        message: format!("base_commit {rev:?} does not resolve to a commit in the worktree"),
    })
}

/// Per-attempt artifact directory under the system temp dir, unique by ids so
/// concurrent attempts never collide. Holds the brief file and the transcript.
fn artifact_dir(req: &ChunkRequest) -> PathBuf {
    std::env::temp_dir()
        .join("octl-harness")
        .join(sanitize(&req.run_id))
        .join(sanitize(&req.chunk_id))
        .join(sanitize(&req.attempt_id))
}

/// Reduce an id to a filesystem-safe token (ids are ulids/slugs, but defend).
fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Run one executable check via `sh -c` in the worktree, bounded by the check's
/// own [`Check::timeout`] and the chunk-wide `cancel` token. A check that exceeds
/// its deadline or is cancelled has its process group killed and is recorded as a
/// non-passing result with `exit_code: None` (killed by signal) — a wedged
/// `cargo test` can never stall the chunk (design §9).
fn run_check(worktree: &Path, check: &Check, cancel: &CancelToken) -> CheckResult {
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(&check.run).current_dir(worktree);
    match run_with_control(cmd, check.timeout, &|| cancel.is_cancelled(), OUTPUT_CAP) {
        ControlledOutcome::Exited {
            status,
            stdout,
            stderr,
        } => CheckResult {
            check_id: check.id.clone(),
            desc: check.desc.clone(),
            run: check.run.clone(),
            passed: status.success(),
            exit_code: status.code(),
            stdout: render_stream(&stdout),
            stderr: render_stream(&stderr),
        },
        ControlledOutcome::Stopped {
            reason,
            stdout,
            stderr,
        } => {
            let note = match reason {
                StopReason::Timeout => {
                    "[orchestratectl: check exceeded its timeout and was killed]"
                }
                StopReason::Cancelled => "[orchestratectl: check was cancelled and killed]",
            };
            // Delimit the note on its own line so it can't glue onto a partial
            // final stderr line.
            let mut stderr = render_stream(&stderr);
            if !stderr.is_empty() && !stderr.ends_with('\n') {
                stderr.push('\n');
            }
            stderr.push_str(note);
            stderr.push('\n');
            CheckResult {
                check_id: check.id.clone(),
                desc: check.desc.clone(),
                run: check.run.clone(),
                passed: false,
                // Killed by a signal — no clean exit code to report.
                exit_code: None,
                stdout: render_stream(&stdout),
                stderr,
            }
        }
        ControlledOutcome::SpawnErr(e) => CheckResult {
            check_id: check.id.clone(),
            desc: check.desc.clone(),
            run: check.run.clone(),
            passed: false,
            exit_code: None,
            stdout: String::new(),
            stderr: format!("could not spawn check: {e}"),
        },
    }
}

/// A non-passing [`CheckResult`] for a check that was *not run* because the chunk
/// was cancelled before we reached it. Recorded (rather than omitted) so the
/// completeness invariant — every requested check has a result — still holds when
/// cancellation truncates the check phase.
fn skipped_check(check: &Check) -> CheckResult {
    CheckResult {
        check_id: check.id.clone(),
        desc: check.desc.clone(),
        run: check.run.clone(),
        passed: false,
        exit_code: None,
        stdout: String::new(),
        stderr: "[orchestratectl: check not run — chunk cancelled]\n".to_string(),
    }
}

/// Render one captured stream to a string, appending an explicit marker when the
/// output was truncated at [`OUTPUT_CAP`] — so a capped transcript is never
/// mistaken for a complete one (or for a crash mid-output).
pub(super) fn render_stream(stream: &CappedStream) -> String {
    use std::fmt::Write as _;
    let mut s = String::from_utf8_lossy(&stream.bytes).into_owned();
    if stream.truncated {
        if !s.is_empty() && !s.ends_with('\n') {
            s.push('\n');
        }
        // Infallible: writing to a String never errors.
        let _ = writeln!(
            s,
            "[orchestratectl: output truncated at {OUTPUT_CAP} bytes]"
        );
    }
    s
}

/// The full transcript for an agent run: stdout then stderr, each truncation-
/// marked. (stdout/stderr are captured on separate threads, so their relative
/// interleaving is not recoverable; they are concatenated in a stable order.) A
/// newline is forced between the two streams when stdout does not already end in
/// one, so a final stdout JSON line (the usage summary) is never glued onto the
/// first stderr line — which would make [`parse_json_usage`]'s line scan miss it.
fn render_transcript(stdout: &CappedStream, stderr: &CappedStream) -> String {
    let mut t = render_stream(stdout);
    if !t.is_empty() && !t.ends_with('\n') {
        t.push('\n');
    }
    t.push_str(&render_stream(stderr));
    t
}

/// Persist a transcript to `path`, returning a reference iff the write succeeded
/// (a failed write is not fatal — the transcript is provenance, not a gate — but
/// it is logged so a missing `transcript_ref` is diagnosable).
fn write_transcript(path: &Path, transcript: &str) -> Option<PathBuf> {
    match std::fs::write(path, transcript) {
        Ok(()) => Some(path.to_path_buf()),
        Err(e) => {
            tracing::warn!(error = %e, path = %path.display(), "failed to persist harness transcript");
            None
        }
    }
}

/// The [`ChunkResult`] for an agent run stopped early (timeout or cancel). No
/// commit, no changed files, no check results — the run never reached those
/// phases — but the partial transcript and any salvaged `usage` are preserved.
/// Matches the conformance contract for the `Timeout`/`Cancelled` outcomes.
fn stopped_result(
    reason: StopReason,
    transcript_ref: Option<PathBuf>,
    usage: Option<Usage>,
) -> ChunkResult {
    let outcome = match reason {
        StopReason::Timeout => ChunkOutcome::Timeout,
        StopReason::Cancelled => ChunkOutcome::Cancelled,
    };
    ChunkResult {
        schema_version: HARNESS_CONTRACT_VERSION,
        outcome,
        resulting_commit: None,
        changed_files: Vec::new(),
        check_results: Vec::new(),
        transcript_ref,
        usage,
    }
}

/// Drive one chunk through `launch`, mapping the resulting *git* state to a
/// [`ChunkResult`] exactly as the aider adapter does (design §10). The full
/// per-family skeleton: cancel/credential/dirty/base preconditions, the bounded
/// agent run, git-outcome mapping, the self-check phase, and transcript capture.
///
/// The outcome is read from git, never from tool prose: HEAD unchanged with a
/// clean tree is [`ChunkOutcome::NoChange`]; a forward HEAD advance is
/// [`ChunkOutcome::Committed`]; uncommitted leftovers or a history rewrite are
/// [`ChunkOutcome::Failed`]. A timeout/cancel returns the partial transcript and
/// no commit (the worktree state is undefined; the supervisor resets before a
/// retry — see the [`CodeHarness`] trait docs).
///
/// [`CodeHarness`]: super::CodeHarness
pub(super) fn run_chunk(
    launch: &dyn AgentLaunch,
    req: &ChunkRequest,
    cancel: &CancelToken,
) -> Result<ChunkResult, HarnessError> {
    let worktree = req.worktree_path.as_path();

    // 0. Honour a cancel tripped before we do any work — a pre-cancelled
    //    request is `Cancelled`, not a credential/worktree error.
    if cancel.is_cancelled() {
        return Ok(stopped_result(StopReason::Cancelled, None, None));
    }

    // 1. Fail fast if a required credential is absent — a clear structured error
    //    beats an opaque provider failure downstream.
    launch.check_credentials()?;

    // 2. Refuse a dirty worktree: prior uncommitted edits would commingle with
    //    the chunk and defeat the base..HEAD diff.
    let status = worktree_status(worktree)?;
    if !status.is_empty() {
        return Err(HarnessError::DirtyWorktree { details: status });
    }

    // Verify the worktree is actually forked at `req.base_commit`. Silently
    // diffing against whatever HEAD happens to be would launder drift and record
    // false provenance (the plan was briefed against `base_commit` / `plan_rev`).
    let base_head = head_oid(worktree)?;
    let want_base = resolve_commit(worktree, &req.base_commit)?;
    if base_head != want_base {
        return Err(HarnessError::InvalidWorktree {
            message: format!("worktree HEAD ({base_head}) != declared base_commit ({want_base})"),
        });
    }

    // 3. Materialize the brief + transcript artifacts.
    let dir = artifact_dir(req);
    std::fs::create_dir_all(&dir).map_err(|e| HarnessError::Internal {
        message: format!("could not create artifact dir {}: {e}", dir.display()),
    })?;
    let prompt = launch.build_prompt(req);
    let brief_file = dir.join("brief.md");
    std::fs::write(&brief_file, &prompt).map_err(|e| HarnessError::Internal {
        message: format!("could not write brief {}: {e}", brief_file.display()),
    })?;
    let transcript_file = dir.join("transcript.log");

    // 4. Build + run the agent invocation, bounded by the request's optional
    //    wall-clock timeout and the cancel token. `run_with_control` re-checks
    //    the cancel before it spawns, so a cancel that arrived during the guards
    //    above still short-circuits here.
    let cmd = launch.build_command(worktree, &brief_file, &prompt, req);
    let run = run_with_control(cmd, req.timeout, &|| cancel.is_cancelled(), OUTPUT_CAP);

    // Extract the completed run's streams, or return early on an early stop /
    // spawn failure. A `Stopped` run persists its partial transcript and maps to
    // `Timeout`/`Cancelled` — never a hang, never a `HarnessError`.
    let (status, stdout, stderr) = match run {
        ControlledOutcome::Exited {
            status,
            stdout,
            stderr,
        } => (status, stdout, stderr),
        ControlledOutcome::Stopped {
            reason,
            stdout,
            stderr,
        } => {
            // Persist the partial transcript and salvage any usage the agent
            // printed before it was killed — a cost circuit-breaker (design §9)
            // wants tokens-spent-so-far even on a stopped run.
            let partial = render_transcript(&stdout, &stderr);
            let usage = launch.parse_usage(&partial);
            let transcript_ref = write_transcript(&transcript_file, &partial);
            return Ok(stopped_result(reason, transcript_ref, usage));
        }
        ControlledOutcome::SpawnErr(e) => {
            return Err(HarnessError::ProviderFailure {
                message: format!(
                    "could not run {} ({}): {e}",
                    launch.tool_label(),
                    launch.bin_display()
                ),
            })
        }
    };

    // Persist a transcript regardless of outcome (provenance). Truncation is
    // marked inline so a capped transcript is never mistaken for a complete one.
    let transcript = render_transcript(&stdout, &stderr);
    let transcript_ref = write_transcript(&transcript_file, &transcript);

    // 4b. For an adapter that does not commit in-process (aider), land whatever
    //     edits it left as a commit — but only after a clean exit, so a failed
    //     agent run is left dirty for the `Failed` mapping (we never fabricate a
    //     commit over an error). A no-op when the tree is already clean.
    if !launch.commits_in_agent() && status.success() {
        commit_leftover_changes(worktree, req, launch.tool_label());
    }

    // 5. Read the outcome from git, never from the agent's prose.
    let new_head = head_oid(worktree)?;
    let dirty_after = !worktree_status(worktree)?.is_empty();
    let tool = launch.tool_label();

    let outcome = if new_head == base_head {
        // No commit. If the agent left uncommitted edits, this is NOT a clean
        // NoChange — reporting NoChange would both lose those edits from the
        // provenance and poison the next attempt's dirty-worktree guard. The
        // adapter does not auto-reset (that would destroy evidence); the
        // supervisor owns worktree cleanup.
        if dirty_after {
            ChunkOutcome::Failed {
                reason: format!("{tool} left uncommitted changes without producing a commit"),
            }
        } else if status.success() {
            ChunkOutcome::NoChange
        } else {
            ChunkOutcome::Failed {
                reason: format!(
                    "{tool} exited {} with no commit produced",
                    status
                        .code()
                        .map_or("signal".to_string(), |c| c.to_string())
                ),
            }
        }
    } else if is_ancestor(worktree, &base_head, &new_head)? {
        // HEAD advanced forward from the base — one or more new commits. The
        // resulting commit is the branch tip; the chunk's range is
        // base_commit..tip (which `changed_files` diffs).
        ChunkOutcome::Committed {
            commit: new_head.clone(),
        }
    } else {
        // HEAD moved but the base is no longer an ancestor: the agent rewrote
        // history (reset/amend/rebase/checkout of unrelated work). Refuse to
        // call that a chunk commit — the range base..tip is meaningless.
        ChunkOutcome::Failed {
            reason: format!(
                "{tool} moved HEAD to {new_head}, which is not a descendant of \
                 base_commit {base_head} (history was rewritten)"
            ),
        }
    };

    let (resulting_commit, files) = match &outcome {
        ChunkOutcome::Committed { commit } => (
            Some(commit.clone()),
            changed_files(worktree, &base_head, &new_head)?,
        ),
        _ => (None, Vec::new()),
    };

    // 6. Run the self-check(s) — regardless of outcome, so a NoChange still
    //    reports the current check state. Each check is individually bounded by
    //    its own timeout and the cancel token (see `run_check`). Once cancel is
    //    tripped we stop *spawning* further checks — each remaining check is
    //    recorded as "skipped" without launching a shell command — but we still
    //    emit a result for *every* requested check so the completeness invariant
    //    holds. The outcome stays whatever git reported: a produced commit is not
    //    demoted to `Cancelled` (the contract forbids a commit on `Cancelled`).
    let check_results: Vec<CheckResult> = req
        .checks
        .iter()
        .map(|c| {
            if cancel.is_cancelled() {
                skipped_check(c)
            } else {
                run_check(worktree, c, cancel)
            }
        })
        .collect();

    let usage = launch.parse_usage(&transcript);

    Ok(ChunkResult {
        schema_version: HARNESS_CONTRACT_VERSION,
        outcome,
        resulting_commit,
        changed_files: files,
        check_results,
        transcript_ref,
        usage,
    })
}

#[cfg(test)]
pub(super) mod test_env {
    //! One process-wide lock for the git-inspecting adapters' env-mutating tests.
    //!
    //! aider/claude/pi tests all set the same ambient env vars (`DEEPSEEK_API_KEY`,
    //! `GIT_BIN`, `OCTL_*_BIN`). A per-module lock only serialises within one
    //! module — cross-module tests still race in the shared test binary and leak a
    //! key/binary override into another module's assertion. A single shared lock
    //! (poison-tolerant, so one test's panic doesn't cascade) serialises them all.
    use std::sync::{Mutex, MutexGuard, PoisonError};

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    pub(in crate::harness) fn lock() -> MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cap(bytes: &[u8]) -> CappedStream {
        CappedStream {
            bytes: bytes.to_vec(),
            truncated: false,
        }
    }

    #[test]
    fn credential_present_treats_empty_as_absent() {
        let _g = test_env::lock();
        std::env::set_var("OCTL_TEST_CRED", "sk-real");
        assert!(credential_present("OCTL_TEST_CRED"));
        std::env::set_var("OCTL_TEST_CRED", "");
        assert!(
            !credential_present("OCTL_TEST_CRED"),
            "empty must be absent"
        );
        std::env::remove_var("OCTL_TEST_CRED");
        assert!(!credential_present("OCTL_TEST_CRED"));
    }

    #[test]
    fn parse_json_usage_takes_last_when_no_result_tag() {
        // No `type:result` marker: the LAST usage-bearing object wins (streaming
        // cumulative totals land last), never the first partial.
        let t = "{\"usage\":{\"input_tokens\":5}}\n\
                 {\"usage\":{\"input_tokens\":900,\"output_tokens\":100}}\n";
        let u = parse_json_usage(t).unwrap();
        assert_eq!(u.input_tokens, Some(900));
        assert_eq!(u.output_tokens, Some(100));
        assert_eq!(u.total_tokens, Some(1000));
    }

    #[test]
    fn parse_json_usage_total_does_not_overflow() {
        // A hostile/absurd token count must not panic (debug) or wrap (release).
        let t = "{\"usage\":{\"input_tokens\":18446744073709551615,\"output_tokens\":1}}";
        let u = parse_json_usage(t).unwrap();
        assert_eq!(u.input_tokens, Some(u64::MAX));
        assert_eq!(u.output_tokens, Some(1));
        assert_eq!(
            u.total_tokens, None,
            "overflowing sum is dropped, not wrapped"
        );
    }

    #[test]
    fn parse_json_usage_reads_explicit_total_and_nested_cost_keys() {
        let t = "{\"usage\":{\"input\":80,\"output\":40,\"total_tokens\":120},\"cost_usd\":0.002}";
        // Flat `input`/`output` are not recognised token keys, but an explicit
        // total and a top-level cost are — so usage is still surfaced.
        let u = parse_json_usage(t).unwrap();
        assert_eq!(u.total_tokens, Some(120));
        assert_eq!(u.cost_usd, Some(0.002));
    }

    #[test]
    fn render_transcript_separates_streams_so_final_json_line_parses() {
        // stdout ends in a JSON usage line with NO trailing newline; stderr has a
        // warning. Without the forced separator they would glue into one broken
        // line and the usage scan would miss it.
        let stdout = cap(
            b"working...\n{\"type\":\"result\",\"usage\":{\"input_tokens\":7,\"output_tokens\":3}}",
        );
        let stderr = cap(b"a stderr warning\n");
        let t = render_transcript(&stdout, &stderr);
        assert!(
            t.contains("}\n"),
            "a newline must separate the JSON line from stderr: {t:?}"
        );
        let u = parse_json_usage(&t).expect("usage still parses after separation");
        assert_eq!(u.total_tokens, Some(10));
    }
}
