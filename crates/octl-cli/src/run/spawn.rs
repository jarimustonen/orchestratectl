//! Shell-out to `~/.claude/skills/worktree/scripts/create.sh` per
//! design.md §8. Single source of truth for the worktree + tmux + agent
//! materialization step. Higher-level callers (`run create` top-level
//! and child-spawn) compose this with event-log writes and supervisor
//! spawn.
//!
//! Contract:
//!
//! - stdout on exit 0 is exactly one JSON object matching
//!   [`SpawnOutcome`]. Extra trailing whitespace is tolerated.
//! - stderr on failure is the standard error envelope (`{schema_version,
//!   error: {...}}`). We propagate `error.code` 1:1 into our own
//!   envelope as `create_sh_error_<code>` so AI callers can still parse
//!   the inner diagnosis without colliding with our own code namespace.
//! - exit 1 from create.sh maps to our exit 1 (user-actionable; clean
//!   state). exit 2 maps to our exit 2 (system; possibly partial state).
//!   exit ≥ 3 (shouldn't happen, but defend) → our exit 2 with stderr
//!   captured as the message.

use std::path::{Path, PathBuf};
use std::process::Command;

use octl_core::schema::TmuxIdentity;
use serde::Deserialize;
use serde_json::Value;

use crate::error::{CliError, ExitKind};

/// The structured-stdout envelope create.sh emits on exit 0
/// (design.md §8.1).
#[derive(Debug, Clone, Deserialize)]
pub struct SpawnOutcome {
    #[serde(default)]
    #[allow(dead_code)]
    pub schema_version: u32,
    #[serde(default)]
    #[allow(dead_code)]
    pub r#type: String,
    pub branch: String,
    pub worktree_path: String,
    pub tmux_window: String,
    /// create.sh's best-effort agent PID, already verified by it via
    /// `tmux list-panes -F '#{pane_pid}'` walk. Callers should treat
    /// this as authoritative on success, but may verify liveness with
    /// `kill(pid, 0)` before recording it on the node.
    pub agent_pid_hint: i64,
    #[serde(default)]
    #[allow(dead_code)]
    pub workmux_session: Option<String>,
    /// Server socket path of the tmux window (`#{socket_path}`). Part of the
    /// qualified tmux identity. create.sh emits the resolved path even for the
    /// default socket, so `None` means either the deployed create.sh predates
    /// the field or its socket query failed. See [`SpawnOutcome::tmux_identity`].
    #[serde(default)]
    pub tmux_socket: Option<String>,
    /// Session that owns the tmux window (`#{session_name}`). `None` if the
    /// deployed create.sh predates the qualified-identity fields.
    #[serde(default)]
    pub tmux_session: Option<String>,
    /// Stable `@NNNN` window id (`#{window_id}`). `None` if the deployed
    /// create.sh predates the qualified-identity fields.
    #[serde(default)]
    pub tmux_window_id: Option<String>,
}

impl SpawnOutcome {
    /// The fully-qualified tmux identity, when create.sh supplied it. Returns
    /// `Some` only when both `tmux_session` and `tmux_window_id` are present and
    /// non-empty — the minimum to match a window. An empty `tmux_socket` is
    /// normalized to `None` (never `tmux -S ""`). A create.sh that predates
    /// these fields — or that emits a partial/empty identity — yields `None`,
    /// and the caller falls back to bare-name matching on `tmux_window` (warned
    /// at the spawn boundary by [`run_create_sh`]).
    pub fn tmux_identity(&self) -> Option<TmuxIdentity> {
        let nonempty = |v: &Option<String>| {
            v.as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        let session = nonempty(&self.tmux_session)?;
        let window_id = nonempty(&self.tmux_window_id)?;
        Some(TmuxIdentity {
            socket: nonempty(&self.tmux_socket),
            session,
            window_id,
        })
    }
}

/// Inputs for one create.sh invocation.
pub struct SpawnRequest<'a> {
    /// kebab-case kind string passed as `--type`.
    pub kind: &'a str,
    /// Branch name (positional 1). Caller is responsible for any
    /// canonicalization; create.sh validates the charset.
    pub branch: &'a str,
    /// Prompt-file path (positional 2). Must exist and be readable.
    pub prompt_file: &'a Path,
    /// Optional pass-through layout name (`-l`).
    pub layout: Option<&'a str>,
    /// Pass `--no-hooks` to create.sh.
    pub no_hooks: bool,
    /// Pass `--keep-tmux-on-error` so a debugging human can inspect the
    /// half-finished tmux window. Only set by tests / `--debug`.
    pub keep_tmux_on_error: bool,
    /// Target tmux session for the worker's window, forwarded to create.sh
    /// as `--parent-session <name>`. `Some` only for a headless spawn
    /// (`--headless` / `--tmux-session`); `None` keeps the default
    /// foreground placement in the caller's own session.
    pub parent_session: Option<&'a str>,
    /// Seconds create.sh waits for the freshly launched agent to become
    /// discoverable before failing with `agent-pid-undiscoverable`,
    /// forwarded as `--agent-startup-timeout <seconds>`. Callers validate
    /// the [1, 600] range (clap) before building the request; octl's
    /// default (90) is higher than create.sh's own 30s because octl
    /// spawns are frequently part of high-fan-out batches that self-load
    /// the host.
    pub agent_startup_timeout: u32,
    /// Base ref to fork the worktree's branch from, forwarded to create.sh
    /// as `--base <ref>` (and on to `workmux add --base`). `Some` for any
    /// run carrying `--source-branch` — critically for `--kind orchestrated`
    /// children whose integration branch is NOT `main`. `None` keeps
    /// workmux's default (the current branch / configured `base_branch`).
    pub source_branch: Option<&'a str>,
}

/// Locate the create.sh binary. Tests override via `OCTL_CREATE_SH`
/// which lets us point at a fixture script that emits canned JSON
/// without needing tmux/workmux available.
pub fn create_sh_path() -> PathBuf {
    if let Ok(v) = std::env::var("OCTL_CREATE_SH") {
        return PathBuf::from(v);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
    PathBuf::from(home).join(".claude/skills/worktree/scripts/create.sh")
}

/// Write `task` content to `<run-dir>/prompt.md` and return its path.
/// Idempotent: a re-run with the same content is a no-op rewrite.
pub fn write_prompt_file(run_dir: &Path, task: &str) -> Result<PathBuf, CliError> {
    let path = run_dir.join("prompt.md");
    std::fs::create_dir_all(run_dir)
        .map_err(|e| CliError::system("io_error", format!("mkdir {}: {}", run_dir.display(), e)))?;
    std::fs::write(&path, task)
        .map_err(|e| CliError::system("io_error", format!("write {}: {}", path.display(), e)))?;
    Ok(path)
}

/// Invoke create.sh and parse its structured stdout.
///
/// On non-zero exit, builds a `CliError` whose payload includes whatever
/// the script wrote on stderr: if stderr parses as a standard error
/// envelope we surface its `code`/`message`/`invalid_value`/`expected`
/// fields; otherwise the raw stderr is included verbatim so debugging
/// has the full context.
pub fn run_create_sh(req: &SpawnRequest<'_>) -> Result<SpawnOutcome, CliError> {
    let script = create_sh_path();
    if !script.exists() {
        return Err(CliError::system(
            "create_sh_missing",
            format!(
                "create.sh not found at {}; install the worktree skill or set OCTL_CREATE_SH",
                script.display()
            ),
        ));
    }
    let mut cmd = Command::new(&script);
    cmd.arg("--type").arg(req.kind);
    if let Some(layout) = req.layout {
        cmd.arg("--layout").arg(layout);
    }
    if req.no_hooks {
        cmd.arg("--no-hooks");
    }
    if req.keep_tmux_on_error {
        cmd.arg("--keep-tmux-on-error");
    }
    if let Some(session) = req.parent_session {
        cmd.arg("--parent-session").arg(session);
    }
    // Always forward octl's agent-startup window (default 90s, higher than
    // create.sh's 30s) so a loaded host doesn't fail the spawn with
    // `agent-pid-undiscoverable`. Validated to [1, 600] at the CLI boundary.
    cmd.arg("--agent-startup-timeout")
        .arg(req.agent_startup_timeout.to_string());
    if let Some(base) = req.source_branch {
        cmd.arg("--base").arg(base);
    }
    cmd.arg(req.branch).arg(req.prompt_file);

    let output = cmd.output().map_err(|e| {
        CliError::system(
            "spawn_failed",
            format!("invoke create.sh ({}): {}", script.display(), e),
        )
    })?;

    if !output.status.success() {
        // The exit-code mapping is documented per-arm; `Some(2)` and the `_`
        // fallthrough deliberately both map to System.
        #[allow(clippy::match_same_arms)]
        let exit_kind = match output.status.code() {
            // create.sh exit 2 = refused-but-actionable (precondition).
            // create.sh exit 1 = mid-flow failure with cleanup done.
            // Both surface to AI as ExitKind::System (we already
            // exhausted user input validation before getting here, so
            // either way the orchestrator is the actor that retries).
            // create.sh's own 2 maps to our 2; its 1 maps to our 1.
            Some(2) => ExitKind::System,
            Some(1) => ExitKind::User,
            _ => ExitKind::System,
        };
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let (code, message, invalid_value, expected) = parse_error_envelope(&stderr)
            .unwrap_or_else(|| {
                (
                    "create_sh_unparseable".to_string(),
                    format!(
                        "create.sh exited {} with non-envelope stderr: {}",
                        output.status.code().unwrap_or(-1),
                        stderr.trim()
                    ),
                    None,
                    None,
                )
            });
        return Err(CliError {
            kind: exit_kind,
            code: format!("create_sh_error_{code}"),
            message,
            invalid_value,
            expected,
        });
    }

    let stdout = String::from_utf8(output.stdout).map_err(|e| {
        CliError::system(
            "create_sh_invalid_stdout",
            format!("create.sh stdout was not UTF-8: {e}"),
        )
    })?;
    let trimmed = stdout.trim();
    let outcome = serde_json::from_str::<SpawnOutcome>(trimmed).map_err(|e| {
        CliError::system(
            "create_sh_unparseable_stdout",
            format!("could not parse create.sh stdout as SpawnOutcome ({e}): {trimmed}"),
        )
    })?;
    // Back-compat / contract check: a create.sh that predates the
    // qualified-identity fields — or that emits a partial/empty identity —
    // yields no usable identity, so the node falls back to bare-name liveness
    // matching (ambiguous across sessions / blind to non-default sockets). Warn
    // here, at the spawn boundary, rather than per watchdog tick. Fires once per
    // spawn whose outcome lacks a usable identity.
    if outcome.tmux_identity().is_none() {
        tracing::warn!(
            tmux_window = %outcome.tmux_window,
            branch = %outcome.branch,
            "create.sh did not emit a usable qualified tmux identity \
             (tmux_session + tmux_window_id, non-empty); falling back to bare \
             window-name liveness matching — update the worktree skill's create.sh"
        );
    }
    Ok(outcome)
}

/// The `code` [`run_create_sh`] reports when create.sh materialised the
/// worktree + tmux window, announced success, but its own immediately-following
/// `tmux list-windows` lookup did not find the window it just created — a
/// tmux settle/timing race under a concurrently-loaded session. create.sh
/// prefixes propagated codes with `create_sh_error_`.
const TMUX_WINDOW_NOT_FOUND_CODE: &str = "create_sh_error_tmux-window-not-found";

/// How many EXTRA create.sh attempts to make after a transient
/// `tmux-window-not-found` (so total executions = 1 initial + `TMUX_MAX_RETRIES`),
/// and how long to pause between them. create.sh cleanly rolls back the partial
/// worktree + branch before exiting on this error, so each retry starts from a
/// clean slate; the reported real-world workaround succeeded on the second try.
const TMUX_MAX_RETRIES: u32 = 3;
const TMUX_RETRY_BACKOFF: std::time::Duration = std::time::Duration::from_millis(1500);

/// Invoke create.sh, retrying on the transient, self-cleaning
/// `tmux-window-not-found` race (issue `headless-spawn-tmux-window-race`).
///
/// create.sh occasionally reports it "Successfully created … tmux window" and
/// then fails its own post-create window lookup because the window has not yet
/// settled in a busy session — after which it cleanly rolls back the worktree
/// and branch and exits non-zero. Because the rollback leaves no partial state,
/// a retry a moment later reliably succeeds. We bound the retries and back off
/// briefly between them; every OTHER create.sh error (including a genuine,
/// non-transient failure) is returned on the first occurrence with no retry, so
/// this never masks a real problem or loops on a deterministic failure.
///
/// The retry cadence is overridable to zero-latency in tests via
/// `OCTL_TMUX_RETRY_BACKOFF_MS=0`.
pub fn run_create_sh_with_tmux_retry(req: &SpawnRequest<'_>) -> Result<SpawnOutcome, CliError> {
    let backoff = match std::env::var("OCTL_TMUX_RETRY_BACKOFF_MS") {
        Ok(v) => match v.trim().parse::<u64>() {
            Ok(ms) => std::time::Duration::from_millis(ms),
            Err(_) => TMUX_RETRY_BACKOFF,
        },
        Err(_) => TMUX_RETRY_BACKOFF,
    };
    let mut attempt: u32 = 0;
    loop {
        match run_create_sh(req) {
            Ok(o) => return Ok(o),
            Err(e) if e.code == TMUX_WINDOW_NOT_FOUND_CODE && attempt < TMUX_MAX_RETRIES => {
                attempt += 1;
                tracing::warn!(
                    target: "orchestratectl::run",
                    branch = %req.branch,
                    attempt,
                    max = TMUX_MAX_RETRIES,
                    "create.sh hit the transient tmux-window-not-found race \
                     (window created then not found); rolled back cleanly, retrying"
                );
                if !backoff.is_zero() {
                    std::thread::sleep(backoff);
                }
            }
            Err(e) => return Err(e),
        }
    }
}

fn parse_error_envelope(stderr: &str) -> Option<(String, String, Option<String>, Option<Value>)> {
    // create.sh writes the error envelope as the *last* JSON object on
    // stderr; preceding lines may carry human progress notes. Scan from
    // the bottom to find a line that parses.
    for line in stderr.lines().rev() {
        let line = line.trim();
        if !line.starts_with('{') {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<Value>(line) {
            let err = v.get("error")?;
            let code = err.get("code").and_then(Value::as_str)?.to_string();
            let message = err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let invalid_value = err
                .get("invalid_value")
                .and_then(Value::as_str)
                .map(str::to_string);
            let expected = err.get("expected").cloned();
            return Some((code, message, invalid_value, expected));
        }
    }
    None
}

/// Verify the agent PID create.sh handed back is still alive. If the
/// process died between create.sh's last check and our call (rare but
/// possible under load), treat it as discovery failure so the caller
/// can emit `node.failed` cleanly instead of recording a dead PID.
pub fn verify_agent_pid(pid: i64) -> Result<(), CliError> {
    if pid <= 0 {
        return Err(CliError::system(
            "agent_pid_invalid",
            format!("create.sh returned non-positive agent_pid_hint: {pid}"),
        ));
    }
    // `agent_pid_hint` is external input from create.sh. Validate the upper
    // bound explicitly rather than letting `as u32` truncate silently — a
    // hint above u32::MAX would otherwise check liveness of the wrong process.
    let pid = u32::try_from(pid).map_err(|_| {
        CliError::system(
            "agent_pid_invalid",
            format!("create.sh returned out-of-range agent_pid_hint: {pid}"),
        )
    })?;
    if !crate::supervise::pid_file::pid_alive(pid) {
        return Err(CliError::system(
            "agent_pid_discovery_failed",
            format!("agent_pid {pid} was not alive after create.sh returned"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tempfile::TempDir;

    // Tests mutate the OCTL_CREATE_SH env var; serialize them so a
    // parallel test runner doesn't see a stale fixture from another
    // thread.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn fixture_script(dir: &Path, body: &str) -> PathBuf {
        let p = dir.join("fake-create.sh");
        std::fs::write(&p, body).unwrap();
        let mut perms = std::fs::metadata(&p).unwrap().permissions();
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o755);
        std::fs::set_permissions(&p, perms).unwrap();
        p
    }

    #[test]
    fn parses_success_stdout() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = TempDir::new().unwrap();
        let me = std::process::id();
        let script = fixture_script(
            dir.path(),
            &format!(
                r#"#!/bin/bash
cat <<EOF
{{"schema_version":1,"type":"spinoff","branch":"wt/x","worktree_path":"/tmp/x","tmux_window":"🚀 wt/x","agent_pid_hint":{me},"workmux_session":"orchestratectl"}}
EOF
"#
            ),
        );
        std::env::set_var("OCTL_CREATE_SH", &script);
        let prompt = dir.path().join("p.md");
        std::fs::write(&prompt, "do thing").unwrap();
        let out = run_create_sh(&SpawnRequest {
            kind: "spinoff",
            branch: "wt/x",
            prompt_file: &prompt,
            layout: None,
            no_hooks: false,
            keep_tmux_on_error: false,
            agent_startup_timeout: 90,
            parent_session: None,
            source_branch: None,
        })
        .unwrap();
        assert_eq!(out.branch, "wt/x");
        assert_eq!(out.tmux_window, "🚀 wt/x");
        assert_eq!(out.agent_pid_hint, i64::from(me));
        std::env::remove_var("OCTL_CREATE_SH");
    }

    #[test]
    fn propagates_error_envelope_from_stderr() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = TempDir::new().unwrap();
        let script = fixture_script(
            dir.path(),
            r#"#!/bin/bash
echo "some human progress" >&2
echo '{"schema_version":1,"error":{"code":"branch-exists","message":"branch already exists","invalid_value":"wt/x"}}' >&2
exit 2
"#,
        );
        std::env::set_var("OCTL_CREATE_SH", &script);
        let prompt = dir.path().join("p.md");
        std::fs::write(&prompt, "do thing").unwrap();
        let err = run_create_sh(&SpawnRequest {
            kind: "spinoff",
            branch: "wt/x",
            prompt_file: &prompt,
            layout: None,
            no_hooks: false,
            keep_tmux_on_error: false,
            agent_startup_timeout: 90,
            parent_session: None,
            source_branch: None,
        })
        .unwrap_err();
        assert_eq!(err.code, "create_sh_error_branch-exists");
        assert_eq!(err.invalid_value.as_deref(), Some("wt/x"));
        assert!(matches!(err.kind, ExitKind::System));
        std::env::remove_var("OCTL_CREATE_SH");
    }

    #[test]
    fn maps_exit_1_to_user() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = TempDir::new().unwrap();
        let script = fixture_script(
            dir.path(),
            r#"#!/bin/bash
echo '{"schema_version":1,"error":{"code":"workmux-failed","message":"workmux add returned non-zero"}}' >&2
exit 1
"#,
        );
        std::env::set_var("OCTL_CREATE_SH", &script);
        let prompt = dir.path().join("p.md");
        std::fs::write(&prompt, "x").unwrap();
        let err = run_create_sh(&SpawnRequest {
            kind: "spinoff",
            branch: "wt/x",
            prompt_file: &prompt,
            layout: None,
            no_hooks: false,
            keep_tmux_on_error: false,
            agent_startup_timeout: 90,
            parent_session: None,
            source_branch: None,
        })
        .unwrap_err();
        assert!(matches!(err.kind, ExitKind::User));
        assert_eq!(err.code, "create_sh_error_workmux-failed");
        std::env::remove_var("OCTL_CREATE_SH");
    }

    #[test]
    fn write_prompt_file_roundtrip() {
        let dir = TempDir::new().unwrap();
        let p = write_prompt_file(dir.path(), "hello").unwrap();
        assert_eq!(std::fs::read_to_string(p).unwrap(), "hello");
    }

    /// A `MakeWriter` that appends to a shared buffer so a test can assert on
    /// what tracing emitted under `with_default`.
    #[derive(Clone)]
    struct BufWriter(std::sync::Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for BufWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for BufWriter {
        type Writer = BufWriter;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    fn run_fixture(dir: &Path, body: &str) -> Result<SpawnOutcome, CliError> {
        let script = fixture_script(dir, body);
        std::env::set_var("OCTL_CREATE_SH", &script);
        let prompt = dir.join("p.md");
        std::fs::write(&prompt, "x").unwrap();
        let out = run_create_sh(&SpawnRequest {
            kind: "spinoff",
            branch: "wt/x",
            prompt_file: &prompt,
            layout: None,
            no_hooks: false,
            keep_tmux_on_error: false,
            agent_startup_timeout: 90,
            parent_session: None,
            source_branch: None,
        });
        std::env::remove_var("OCTL_CREATE_SH");
        out
    }

    #[test]
    fn parses_qualified_tmux_identity() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = TempDir::new().unwrap();
        let me = std::process::id();
        let out = run_fixture(
            dir.path(),
            &format!(
                r#"#!/bin/bash
cat <<EOF
{{"schema_version":1,"type":"spinoff","branch":"wt/x","worktree_path":"/tmp/x","tmux_window":"🚀 wt/x","agent_pid_hint":{me},"workmux_session":"octl","tmux_socket":"/private/tmp/tmux-501/default","tmux_session":"octl","tmux_window_id":"@42"}}
EOF
"#
            ),
        )
        .unwrap();
        let id = out.tmux_identity().expect("qualified identity present");
        assert_eq!(id.socket.as_deref(), Some("/private/tmp/tmux-501/default"));
        assert_eq!(id.session, "octl");
        assert_eq!(id.window_id, "@42");
    }

    #[test]
    fn qualified_identity_tolerates_null_socket() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = TempDir::new().unwrap();
        let me = std::process::id();
        let out = run_fixture(
            dir.path(),
            &format!(
                r#"#!/bin/bash
cat <<EOF
{{"schema_version":1,"type":"spinoff","branch":"wt/x","worktree_path":"/tmp/x","tmux_window":"🚀 wt/x","agent_pid_hint":{me},"workmux_session":"octl","tmux_socket":null,"tmux_session":"octl","tmux_window_id":"@7"}}
EOF
"#
            ),
        )
        .unwrap();
        let id = out
            .tmux_identity()
            .expect("identity present even without socket");
        assert_eq!(id.socket, None);
        assert_eq!(id.window_id, "@7");
    }

    #[test]
    fn forwards_parent_session_flag() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = TempDir::new().unwrap();
        let me = std::process::id();
        // Fixture echoes back whichever `--parent-session <name>` it was given
        // (default `none` if absent) as the emitted `tmux_session`, so the test
        // can assert the flag actually reached the script's argv.
        let script = fixture_script(
            dir.path(),
            &format!(
                r#"#!/bin/bash
sess="none"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --parent-session) sess="$2"; shift 2 ;;
    *) shift ;;
  esac
done
cat <<EOF
{{"schema_version":1,"type":"spinoff","branch":"wt/x","worktree_path":"/tmp/x","tmux_window":"🚀 wt/x","agent_pid_hint":{me},"tmux_session":"$sess","tmux_window_id":"@9"}}
EOF
"#
            ),
        );
        std::env::set_var("OCTL_CREATE_SH", &script);
        let prompt = dir.path().join("p.md");
        std::fs::write(&prompt, "x").unwrap();

        let with = run_create_sh(&SpawnRequest {
            kind: "spinoff",
            branch: "wt/x",
            prompt_file: &prompt,
            layout: None,
            no_hooks: false,
            keep_tmux_on_error: false,
            agent_startup_timeout: 90,
            parent_session: Some("headless"),
            source_branch: None,
        })
        .unwrap();
        assert_eq!(with.tmux_session.as_deref(), Some("headless"));

        let without = run_create_sh(&SpawnRequest {
            kind: "spinoff",
            branch: "wt/x",
            prompt_file: &prompt,
            layout: None,
            no_hooks: false,
            keep_tmux_on_error: false,
            agent_startup_timeout: 90,
            parent_session: None,
            source_branch: None,
        })
        .unwrap();
        assert_eq!(without.tmux_session.as_deref(), Some("none"));
        std::env::remove_var("OCTL_CREATE_SH");
    }

    #[test]
    fn forwards_base_flag_from_source_branch() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = TempDir::new().unwrap();
        let me = std::process::id();
        // Fixture echoes back whichever `--base <ref>` it was given (default
        // `none` if absent) as the emitted `tmux_session`, so the test can
        // assert the base ref actually reached the script's argv.
        let script = fixture_script(
            dir.path(),
            &format!(
                r#"#!/bin/bash
base="none"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --base) base="$2"; shift 2 ;;
    *) shift ;;
  esac
done
cat <<EOF
{{"schema_version":1,"type":"orchestrated","branch":"wt/x","worktree_path":"/tmp/x","tmux_window":"🎼 wt/x","agent_pid_hint":{me},"tmux_session":"$base","tmux_window_id":"@9"}}
EOF
"#
            ),
        );
        std::env::set_var("OCTL_CREATE_SH", &script);
        let prompt = dir.path().join("p.md");
        std::fs::write(&prompt, "x").unwrap();

        let with = run_create_sh(&SpawnRequest {
            kind: "orchestrated",
            branch: "wt/x",
            prompt_file: &prompt,
            layout: None,
            no_hooks: false,
            keep_tmux_on_error: false,
            agent_startup_timeout: 90,
            parent_session: None,
            source_branch: Some("orchestrate/integration"),
        })
        .unwrap();
        assert_eq!(
            with.tmux_session.as_deref(),
            Some("orchestrate/integration")
        );

        let without = run_create_sh(&SpawnRequest {
            kind: "orchestrated",
            branch: "wt/x",
            prompt_file: &prompt,
            layout: None,
            no_hooks: false,
            keep_tmux_on_error: false,
            agent_startup_timeout: 90,
            parent_session: None,
            source_branch: None,
        })
        .unwrap();
        assert_eq!(without.tmux_session.as_deref(), Some("none"));
        std::env::remove_var("OCTL_CREATE_SH");
    }

    #[test]
    fn forwards_agent_startup_timeout_flag() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = TempDir::new().unwrap();
        let me = std::process::id();
        // Fixture echoes back whichever `--agent-startup-timeout <s>` it was
        // given (default `none` if absent) as the emitted `tmux_session`, so the
        // test can assert the value actually reached the script's argv.
        let script = fixture_script(
            dir.path(),
            &format!(
                r#"#!/bin/bash
to="none"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --agent-startup-timeout) to="$2"; shift 2 ;;
    *) shift ;;
  esac
done
cat <<EOF
{{"schema_version":1,"type":"spinoff","branch":"wt/x","worktree_path":"/tmp/x","tmux_window":"🚀 wt/x","agent_pid_hint":{me},"tmux_session":"$to","tmux_window_id":"@9"}}
EOF
"#
            ),
        );
        std::env::set_var("OCTL_CREATE_SH", &script);
        let prompt = dir.path().join("p.md");
        std::fs::write(&prompt, "x").unwrap();

        // Explicit non-default value is forwarded verbatim.
        let with = run_create_sh(&SpawnRequest {
            kind: "spinoff",
            branch: "wt/x",
            prompt_file: &prompt,
            layout: None,
            no_hooks: false,
            keep_tmux_on_error: false,
            agent_startup_timeout: 180,
            parent_session: None,
            source_branch: None,
        })
        .unwrap();
        assert_eq!(with.tmux_session.as_deref(), Some("180"));

        // The octl default (90) is always forwarded — never create.sh's 30s.
        let defaulted = run_create_sh(&SpawnRequest {
            kind: "spinoff",
            branch: "wt/x",
            prompt_file: &prompt,
            layout: None,
            no_hooks: false,
            keep_tmux_on_error: false,
            agent_startup_timeout: 90,
            parent_session: None,
            source_branch: None,
        })
        .unwrap();
        assert_eq!(defaulted.tmux_session.as_deref(), Some("90"));
        std::env::remove_var("OCTL_CREATE_SH");
    }

    #[test]
    fn back_compat_missing_identity_is_none_and_warns() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = TempDir::new().unwrap();
        let me = std::process::id();
        // A create.sh that predates the qualified-identity fields: stdout has
        // no tmux_socket/tmux_session/tmux_window_id.
        let body = format!(
            r#"#!/bin/bash
cat <<EOF
{{"schema_version":1,"type":"spinoff","branch":"wt/x","worktree_path":"/tmp/x","tmux_window":"🚀 wt/x","agent_pid_hint":{me},"workmux_session":"octl"}}
EOF
"#
        );
        let buf = std::sync::Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::fmt()
            .with_writer(BufWriter(buf.clone()))
            .with_max_level(tracing::Level::WARN)
            .finish();
        let out = tracing::subscriber::with_default(subscriber, || {
            run_fixture(dir.path(), &body).unwrap()
        });
        assert!(out.tmux_identity().is_none());
        assert_eq!(out.tmux_session, None);
        assert_eq!(out.tmux_window_id, None);
        let logged = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        assert!(
            logged.contains("qualified tmux identity"),
            "expected back-compat warning, got: {logged:?}"
        );
    }

    /// Build a `SpawnRequest` pointing at `prompt`, run it through the
    /// tmux-retry wrapper with zero backoff, and return the result.
    fn run_retry_fixture(dir: &Path, body: &str) -> Result<SpawnOutcome, CliError> {
        let script = fixture_script(dir, body);
        std::env::set_var("OCTL_CREATE_SH", &script);
        std::env::set_var("OCTL_TMUX_RETRY_BACKOFF_MS", "0");
        let prompt = dir.join("p.md");
        std::fs::write(&prompt, "x").unwrap();
        let out = run_create_sh_with_tmux_retry(&SpawnRequest {
            kind: "spinoff",
            branch: "wt/x",
            prompt_file: &prompt,
            layout: None,
            no_hooks: false,
            keep_tmux_on_error: false,
            agent_startup_timeout: 90,
            parent_session: None,
            source_branch: None,
        });
        std::env::remove_var("OCTL_TMUX_RETRY_BACKOFF_MS");
        std::env::remove_var("OCTL_CREATE_SH");
        out
    }

    #[test]
    fn tmux_retry_recovers_after_transient_window_not_found() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = TempDir::new().unwrap();
        let me = std::process::id();
        let counter = dir.path().join("attempts");
        // Fails with the transient tmux-window-not-found error on the first
        // attempt, then succeeds — mirroring the create-then-observe race that
        // clears on a retry a moment later.
        let body = format!(
            r#"#!/bin/bash
n=0
if [[ -f "{c}" ]]; then n=$(cat "{c}"); fi
n=$((n+1))
echo "$n" > "{c}"
if [[ "$n" -lt 2 ]]; then
  echo '{{"schema_version":1,"error":{{"code":"tmux-window-not-found","message":"No tmux window"}}}}' >&2
  exit 1
fi
cat <<EOF
{{"schema_version":1,"type":"spinoff","branch":"wt/x","worktree_path":"/tmp/x","tmux_window":"🚀 wt/x","agent_pid_hint":{me},"tmux_session":"headless","tmux_window_id":"@9"}}
EOF
"#,
            c = counter.display()
        );
        let out = run_retry_fixture(dir.path(), &body).expect("retry should recover");
        assert_eq!(out.branch, "wt/x");
        // Exactly two attempts: one failure + one success.
        assert_eq!(
            std::fs::read_to_string(&counter).unwrap().trim(),
            "2",
            "expected exactly one retry after the transient failure"
        );
    }

    #[test]
    fn tmux_retry_gives_up_after_bound_and_surfaces_error() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = TempDir::new().unwrap();
        let counter = dir.path().join("attempts");
        // Always fails with the transient code: the wrapper must stop after the
        // bounded attempts and surface the error rather than loop forever.
        let body = format!(
            r#"#!/bin/bash
n=0
if [[ -f "{c}" ]]; then n=$(cat "{c}"); fi
echo "$((n+1))" > "{c}"
echo '{{"schema_version":1,"error":{{"code":"tmux-window-not-found","message":"No tmux window"}}}}' >&2
exit 1
"#,
            c = counter.display()
        );
        let err = run_retry_fixture(dir.path(), &body).unwrap_err();
        assert_eq!(err.code, "create_sh_error_tmux-window-not-found");
        // 1 initial + TMUX_MAX_RETRIES retries.
        assert_eq!(
            std::fs::read_to_string(&counter).unwrap().trim(),
            (TMUX_MAX_RETRIES + 1).to_string(),
            "expected initial attempt plus the full retry budget"
        );
    }

    #[test]
    fn tmux_retry_surfaces_a_different_error_from_a_later_attempt() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = TempDir::new().unwrap();
        let counter = dir.path().join("attempts");
        // Attempt 1 hits the transient race; attempt 2 fails with a DIFFERENT,
        // non-transient error (e.g. a partial rollback left the branch behind).
        // The wrapper must stop retrying and surface that error loudly — never
        // loop on it or mask it.
        let body = format!(
            r#"#!/bin/bash
n=0
if [[ -f "{c}" ]]; then n=$(cat "{c}"); fi
n=$((n+1))
echo "$n" > "{c}"
if [[ "$n" -lt 2 ]]; then
  echo '{{"schema_version":1,"error":{{"code":"tmux-window-not-found","message":"No tmux window"}}}}' >&2
  exit 1
fi
echo '{{"schema_version":1,"error":{{"code":"branch-exists","message":"branch already exists"}}}}' >&2
exit 2
"#,
            c = counter.display()
        );
        let err = run_retry_fixture(dir.path(), &body).unwrap_err();
        assert_eq!(err.code, "create_sh_error_branch-exists");
        assert_eq!(
            std::fs::read_to_string(&counter).unwrap().trim(),
            "2",
            "the non-transient error on attempt 2 must be surfaced immediately, no further retries"
        );
    }

    #[test]
    fn tmux_retry_does_not_retry_other_errors() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = TempDir::new().unwrap();
        let counter = dir.path().join("attempts");
        // A different, non-transient error must be returned on the first try
        // with no retry — the wrapper must never mask or loop on a real failure.
        let body = format!(
            r#"#!/bin/bash
n=0
if [[ -f "{c}" ]]; then n=$(cat "{c}"); fi
echo "$((n+1))" > "{c}"
echo '{{"schema_version":1,"error":{{"code":"branch-exists","message":"branch already exists"}}}}' >&2
exit 2
"#,
            c = counter.display()
        );
        let err = run_retry_fixture(dir.path(), &body).unwrap_err();
        assert_eq!(err.code, "create_sh_error_branch-exists");
        assert_eq!(
            std::fs::read_to_string(&counter).unwrap().trim(),
            "1",
            "a non-transient error must not be retried"
        );
    }
}
