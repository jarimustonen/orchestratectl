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
}
