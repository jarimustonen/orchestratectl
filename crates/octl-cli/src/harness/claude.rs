//! Claude-Code-family adapters — [`ClaudeHarness`] and [`ClaudeDeepseekHarness`]
//! (design.md §10: `claude-deepseek` is the primary "option A" binding; a plain
//! `claude` is the ambient-login sibling).
//!
//! Both drive **Claude Code** headless (`-p`/print mode) over one chunk, let it
//! edit files + `git commit`, and map the result from the resulting *git* state —
//! exactly like [`super::aider`] (never from Claude's prose). They differ *only*
//! by the binary and its leading args (captured in [`ClaudeVariant`]); the whole
//! git-inspecting skeleton, the implement/self-check/commit prompt framing
//! ([`support::commit_framed_prompt`]), and the usage parse are shared:
//!
//! ```text
//! claude          -p --output-format json --dangerously-skip-permissions [--model M] <prompt>
//! claude-deepseek --model flash -p --output-format json <prompt>
//! ```
//!
//! `--output-format json` makes Claude emit a single machine-readable result
//! object carrying `total_cost_usd` + `usage.{input,output}_tokens`, which
//! [`parse_claude_usage`] lifts into [`Usage`] (best-effort, never a gate).
//!
//! Credentials are never hardcoded and never checked here: plain `claude` uses
//! the ambient Claude Code login, and `claude-deepseek` sources its own `DeepSeek`
//! key (from SOPS) inside the wrapper. The binaries honour `OCTL_CLAUDE_BIN` /
//! `OCTL_CLAUDE_DEEPSEEK_BIN` overrides so tests can point at a fixture script
//! that fakes an edit+commit without a network call.
//!
//! **`claude-deepseek` already appends `--dangerously-skip-permissions` itself**
//! (it `exec`s `claude --dangerously-skip-permissions "$@"`), so the deepseek
//! variant must NOT add that flag a second time — only the plain `claude` variant
//! passes it.

use std::path::Path;
use std::process::Command;

use super::support::{self, AgentLaunch};
use super::{
    CancelToken, ChunkRequest, ChunkResult, CodeHarness, HarnessCapabilities, HarnessError, Usage,
};

/// `claude` binary, honouring `OCTL_CLAUDE_BIN` so tests can stub it.
fn claude_bin() -> String {
    std::env::var("OCTL_CLAUDE_BIN").unwrap_or_else(|_| "claude".to_string())
}

/// `claude-deepseek` wrapper binary, honouring `OCTL_CLAUDE_DEEPSEEK_BIN`.
fn claude_deepseek_bin() -> String {
    std::env::var("OCTL_CLAUDE_DEEPSEEK_BIN").unwrap_or_else(|_| "claude-deepseek".to_string())
}

/// Which Claude-Code launch a [`ClaudeHarness`] performs — the *only* thing that
/// differs between the plain and deepseek-backed adapters.
#[derive(Debug, Clone)]
enum ClaudeVariant {
    /// Plain `claude`, ambient login. Optional `--model` alias.
    Claude { model: Option<String> },
    /// `claude-deepseek --model <pro|flash>`, `DeepSeek` backend (key sourced by
    /// the wrapper). The wrapper adds `--dangerously-skip-permissions` itself.
    Deepseek { model: String },
}

/// A Claude-Code [`CodeHarness`] adapter. Construct via [`ClaudeHarness::claude`]
/// or [`ClaudeHarness::deepseek`]; both share [`run_chunk`](support::run_chunk)
/// and differ only in [`ClaudeVariant`].
#[derive(Debug, Clone)]
pub struct ClaudeHarness {
    variant: ClaudeVariant,
    /// Extra args inserted before the prompt (empty by default) — lets a caller
    /// pass e.g. `--fallback-model` without a new field.
    extra_args: Vec<String>,
}

impl ClaudeHarness {
    /// Plain Claude Code over the ambient login. `model` is an optional `--model`
    /// alias (e.g. `"sonnet"`); `None` uses Claude Code's configured default.
    pub fn claude(model: Option<String>) -> Self {
        Self {
            variant: ClaudeVariant::Claude { model },
            extra_args: Vec::new(),
        }
    }

    /// Claude Code pointed at a `DeepSeek` backend via the `claude-deepseek`
    /// wrapper. `model` is the wrapper's `--model` (`"flash"` or `"pro"`).
    pub fn deepseek(model: impl Into<String>) -> Self {
        Self {
            variant: ClaudeVariant::Deepseek {
                model: model.into(),
            },
            extra_args: Vec::new(),
        }
    }

    /// Append extra args (inserted before the prompt positional).
    #[must_use]
    pub fn with_extra_args(mut self, args: Vec<String>) -> Self {
        self.extra_args = args;
        self
    }
}

impl AgentLaunch for ClaudeHarness {
    fn capabilities(&self) -> HarnessCapabilities {
        HarnessCapabilities {
            // Claude Code edits any file, tests included.
            can_author_tests: true,
            // Usage is read from the `--output-format json` result object.
            reports_usage: true,
            // The prompt asks Claude to stay in scope, but the adapter does not
            // *enforce* it — the deterministic floor (design §4) does.
            honors_file_scope: false,
            // The adapter runs the request's checks as the code-node self-check.
            runs_checks: true,
        }
    }

    fn check_credentials(&self) -> Result<(), HarnessError> {
        // Plain `claude` uses the ambient login; `claude-deepseek` sources its
        // own key inside the wrapper. Neither reads a credential env var here, so
        // there is nothing to fail fast on — a genuine auth failure surfaces as a
        // provider/agent error captured in the transcript.
        Ok(())
    }

    fn build_prompt(&self, req: &ChunkRequest) -> String {
        support::commit_framed_prompt(req)
    }

    fn build_command(
        &self,
        worktree: &Path,
        _brief_file: &Path,
        prompt: &str,
        _req: &ChunkRequest,
    ) -> Command {
        let mut cmd = match &self.variant {
            ClaudeVariant::Claude { model } => {
                let mut c = Command::new(claude_bin());
                c.arg("-p")
                    .arg("--output-format")
                    .arg("json")
                    // Non-interactive: bypass every permission gate (throwaway
                    // worktree; the agent must edit + commit without prompts).
                    .arg("--dangerously-skip-permissions");
                if let Some(m) = model {
                    c.arg("--model").arg(m);
                }
                c
            }
            ClaudeVariant::Deepseek { model } => {
                let mut c = Command::new(claude_deepseek_bin());
                // The wrapper consumes `--model <pro|flash>`, then `exec`s
                // `claude --dangerously-skip-permissions <rest>` — so we pass the
                // print/json flags but NOT a second skip-permissions flag.
                c.arg("--model")
                    .arg(model)
                    .arg("-p")
                    .arg("--output-format")
                    .arg("json");
                c
            }
        };
        cmd.current_dir(worktree);
        for extra in &self.extra_args {
            cmd.arg(extra);
        }
        // `--` terminates option parsing so a prompt that begins with a dash
        // cannot be mistaken for a Claude flag; the prompt is the sole positional.
        cmd.arg("--").arg(prompt);
        cmd
    }

    fn parse_usage(&self, transcript: &str) -> Option<Usage> {
        parse_claude_usage(transcript)
    }

    fn tool_label(&self) -> &'static str {
        match &self.variant {
            ClaudeVariant::Claude { .. } => "claude",
            ClaudeVariant::Deepseek { .. } => "claude-deepseek",
        }
    }

    fn bin_display(&self) -> String {
        match &self.variant {
            ClaudeVariant::Claude { .. } => claude_bin(),
            ClaudeVariant::Deepseek { .. } => claude_deepseek_bin(),
        }
    }
}

impl CodeHarness for ClaudeHarness {
    fn capabilities(&self) -> HarnessCapabilities {
        <Self as AgentLaunch>::capabilities(self)
    }

    fn run_chunk(
        &self,
        req: &ChunkRequest,
        cancel: &CancelToken,
    ) -> Result<ChunkResult, HarnessError> {
        support::run_chunk(self, req, cancel)
    }
}

/// Deprecated-name alias kept intentionally minimal: some call sites prefer a
/// dedicated deepseek type. `claude-deepseek` and `claude` are *one* adapter
/// (they differ only by [`ClaudeVariant`]); [`ClaudeHarness::deepseek`] is the
/// canonical constructor.
pub type ClaudeDeepseekHarness = ClaudeHarness;

/// Best-effort extraction of Claude Code's `--output-format json` usage summary.
///
/// In print/json mode Claude emits a single result object on stdout, e.g.
/// `{"type":"result","total_cost_usd":0.0012,"usage":{"input_tokens":1200,
/// "output_tokens":300,...},...}`. Delegates to [`support::parse_json_usage`],
/// the shared JSON-usage scanner (also used by pi); never fails.
fn parse_claude_usage(transcript: &str) -> Option<Usage> {
    support::parse_json_usage(transcript)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::conformance::{run_and_check, run_and_check_with_cancel};
    use crate::harness::{Check, ChunkOutcome};
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use tempfile::TempDir;

    // These tests mutate process env (OCTL_CLAUDE_BIN, OCTL_CLAUDE_DEEPSEEK_BIN,
    // GIT_BIN); serialize them so parallel runners don't cross-contaminate.

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        crate::harness::support::test_env::lock()
    }

    /// Write a fixture script into `dir` (kept OUT of the git worktree so it does
    /// not show up as an untracked change and trip the dirty-worktree guard).
    fn write_script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        let mut perms = std::fs::metadata(&p).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&p, perms).unwrap();
        p
    }

    /// A real git repo with one commit, so the adapter's base..HEAD diffing runs
    /// against genuine git state (the agent is the only stubbed boundary).
    fn init_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        let run = |args: &[&str]| {
            let ok = Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .output()
                .unwrap();
            assert!(ok.status.success(), "git {args:?}: {ok:?}");
        };
        run(&["init", "-q", "-b", "main"]);
        run(&["config", "user.email", "t@t"]);
        run(&["config", "user.name", "t"]);
        std::fs::write(dir.path().join("seed.txt"), "seed\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "seed"]);
        dir
    }

    fn head_of(worktree: &Path) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(worktree)
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn base_request(worktree: &Path) -> ChunkRequest {
        ChunkRequest {
            run_id: "run1".into(),
            chunk_id: "c1".into(),
            attempt_id: "a1".into(),
            worktree_path: worktree.to_path_buf(),
            base_commit: head_of(worktree),
            plan_rev: "v1".into(),
            brief: "do the thing".into(),
            checks: vec![Check {
                id: "chk1".into(),
                desc: "always passes".into(),
                run: "true".into(),
                timeout: None,
            }],
            files: vec![PathBuf::from("out.txt")],
            timeout: None,
        }
    }

    #[test]
    fn claude_commit_produced_maps_to_committed_with_json_usage() {
        let _g = env_lock();
        let repo = init_repo();
        // Fake claude: writes a file, commits it, prints a --output-format json
        // result object with usage + cost.
        let sdir = TempDir::new().unwrap();
        let bin = write_script(
            sdir.path(),
            "fake-claude.sh",
            "#!/bin/bash\n\
             printf 'edited\\n' > out.txt\n\
             git add out.txt\n\
             git commit -q -m 'chunk edit'\n\
             printf '%s\\n' '{\"type\":\"result\",\"total_cost_usd\":0.0012,\"usage\":{\"input_tokens\":1200,\"output_tokens\":300}}'\n",
        );
        std::env::set_var("OCTL_CLAUDE_BIN", &bin);

        let h = ClaudeHarness::claude(None);
        let req = base_request(repo.path());
        let res = run_and_check(&h, &req).unwrap();

        assert!(matches!(res.outcome, ChunkOutcome::Committed { .. }));
        assert_eq!(res.changed_files, vec![PathBuf::from("out.txt")]);
        assert_eq!(res.check_results.len(), 1);
        assert!(res.check_results[0].passed);
        let usage = res.usage.expect("json usage parsed");
        assert_eq!(usage.cost_usd, Some(0.0012));
        assert_eq!(usage.input_tokens, Some(1200));
        assert_eq!(usage.output_tokens, Some(300));
        assert_eq!(usage.total_tokens, Some(1500));

        std::env::remove_var("OCTL_CLAUDE_BIN");
    }

    #[test]
    fn deepseek_variant_uses_its_own_binary() {
        let _g = env_lock();
        let repo = init_repo();
        // The deepseek fixture writes a distinct file so we can prove the
        // deepseek binary (not the plain-claude one) was the one that ran.
        let sdir = TempDir::new().unwrap();
        let ds = write_script(
            sdir.path(),
            "fake-ds.sh",
            "#!/bin/bash\nprintf 'ds\\n' > ds.txt\ngit add ds.txt\ngit commit -q -m ds\n",
        );
        // Point plain-claude at a binary that would fail the test if invoked.
        std::env::set_var("OCTL_CLAUDE_BIN", "/nonexistent/should-not-run");
        std::env::set_var("OCTL_CLAUDE_DEEPSEEK_BIN", &ds);

        let h = ClaudeHarness::deepseek("flash");
        let req = base_request(repo.path());
        let res = run_and_check(&h, &req).unwrap();
        assert!(matches!(res.outcome, ChunkOutcome::Committed { .. }));
        assert_eq!(res.changed_files, vec![PathBuf::from("ds.txt")]);

        std::env::remove_var("OCTL_CLAUDE_BIN");
        std::env::remove_var("OCTL_CLAUDE_DEEPSEEK_BIN");
    }

    #[test]
    fn no_credential_check_runs_without_env() {
        let _g = env_lock();
        let repo = init_repo();
        // No credential env set at all — a clean no-op agent still runs (unlike
        // aider, the Claude adapters never fail fast on a missing key).
        let sdir = TempDir::new().unwrap();
        let bin = write_script(
            sdir.path(),
            "fake-claude.sh",
            "#!/bin/bash\necho '{\"type\":\"result\"}'\nexit 0\n",
        );
        std::env::set_var("OCTL_CLAUDE_BIN", &bin);
        let h = ClaudeHarness::claude(None);
        let res = run_and_check(&h, &base_request(repo.path())).unwrap();
        assert_eq!(res.outcome, ChunkOutcome::NoChange);
        std::env::remove_var("OCTL_CLAUDE_BIN");
    }

    #[test]
    fn no_commit_nonzero_exit_maps_to_failed() {
        let _g = env_lock();
        let repo = init_repo();
        let sdir = TempDir::new().unwrap();
        let bin = write_script(
            sdir.path(),
            "fake-claude.sh",
            "#!/bin/bash\necho 'boom' >&2\nexit 2\n",
        );
        std::env::set_var("OCTL_CLAUDE_BIN", &bin);
        let h = ClaudeHarness::claude(None);
        let res = run_and_check(&h, &base_request(repo.path())).unwrap();
        assert!(matches!(res.outcome, ChunkOutcome::Failed { .. }));
        std::env::remove_var("OCTL_CLAUDE_BIN");
    }

    #[test]
    fn timeout_kills_hung_claude() {
        let _g = env_lock();
        let repo = init_repo();
        let sdir = TempDir::new().unwrap();
        let bin = write_script(
            sdir.path(),
            "fake-claude.sh",
            "#!/bin/bash\nsleep 30 & sleep 30\n",
        );
        std::env::set_var("OCTL_CLAUDE_BIN", &bin);
        let h = ClaudeHarness::claude(None);
        let mut req = base_request(repo.path());
        req.timeout = Some(std::time::Duration::from_millis(200));
        let start = std::time::Instant::now();
        let res = run_and_check(&h, &req).unwrap();
        assert_eq!(res.outcome, ChunkOutcome::Timeout);
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
        std::env::remove_var("OCTL_CLAUDE_BIN");
    }

    #[test]
    fn cancel_in_flight_aborts_claude() {
        let _g = env_lock();
        let repo = init_repo();
        let sdir = TempDir::new().unwrap();
        let bin = write_script(sdir.path(), "fake-claude.sh", "#!/bin/bash\nsleep 30\n");
        std::env::set_var("OCTL_CLAUDE_BIN", &bin);
        let h = ClaudeHarness::claude(None);
        let req = base_request(repo.path());
        let cancel = CancelToken::new();
        let trip = cancel.clone();
        let handle = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(150));
            trip.cancel();
        });
        let start = std::time::Instant::now();
        let res = run_and_check_with_cancel(&h, &req, &cancel).unwrap();
        handle.join().unwrap();
        assert_eq!(res.outcome, ChunkOutcome::Cancelled);
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
        std::env::remove_var("OCTL_CLAUDE_BIN");
    }

    #[test]
    fn spawn_failure_is_structured_error() {
        let _g = env_lock();
        let repo = init_repo();
        std::env::set_var("OCTL_CLAUDE_BIN", "/nonexistent/claude-xyz");
        let h = ClaudeHarness::claude(None);
        let err = h
            .run_chunk(&base_request(repo.path()), &CancelToken::new())
            .unwrap_err();
        assert!(matches!(err, HarnessError::ProviderFailure { .. }));
        std::env::remove_var("OCTL_CLAUDE_BIN");
    }

    #[test]
    fn parse_claude_usage_from_json_line() {
        let t = "some log line\n{\"type\":\"result\",\"total_cost_usd\":0.5,\"usage\":{\"input_tokens\":10,\"output_tokens\":20}}\n";
        let u = parse_claude_usage(t).unwrap();
        assert_eq!(u.input_tokens, Some(10));
        assert_eq!(u.output_tokens, Some(20));
        assert_eq!(u.total_tokens, Some(30));
        assert_eq!(u.cost_usd, Some(0.5));
    }

    #[test]
    fn parse_claude_usage_absent_is_none() {
        assert!(parse_claude_usage("no json here\njust prose\n").is_none());
        // A JSON object with neither usage nor cost yields None.
        assert!(parse_claude_usage("{\"type\":\"result\"}").is_none());
    }

    #[test]
    fn deepseek_type_alias_is_claude_harness() {
        // Compile-time proof the alias resolves; deepseek() builds the variant.
        let _h: ClaudeDeepseekHarness = ClaudeHarness::deepseek("pro");
    }
}
