//! `pi` adapter — the [earendil-works/pi] coding agent behind [`CodeHarness`]
//! (design.md §10: pi.dev candidate, "customizable, multi-provider").
//!
//! Installed via `npm i -g @earendil-works/pi-coding-agent` (ships the `pi`
//! binary; the `@mariozechner/*` names are dead ends). Driven headless exactly
//! like the Claude family — one-shot `-p`/print mode, edits files in the worktree,
//! and (because pi does NOT auto-commit) told by the prompt to `git commit`. The
//! outcome is read from the resulting *git* state, never from pi's prose:
//!
//! ```text
//! pi -p --mode json --provider <provider> --model <model> <prompt>
//! ```
//!
//! `--provider`/`--model` select the backend; the key is read from the provider's
//! env var (default `DEEPSEEK_API_KEY`, checked fast so a missing key is a clean
//! [`HarnessError::MissingCredential`] rather than an opaque agent failure). pi's
//! own auth resolution (`--api-key`, `~/.pi/agent/auth.json`) still applies, but a
//! present env var is the common path and the only precondition this adapter
//! guarantees. `--mode json` yields a machine-readable event stream from which
//! [`support::parse_json_usage`] lifts token/cost best-effort.
//!
//! The binary honours `OCTL_PI_BIN` so tests can point at a fixture script that
//! fakes an edit+commit without a network call.
//!
//! [earendil-works/pi]: https://github.com/earendil-works/pi

use std::path::Path;
use std::process::Command;

use super::support::{self, AgentLaunch};
use super::{
    CancelToken, ChunkRequest, ChunkResult, CodeHarness, HarnessCapabilities, HarnessError, Usage,
};

/// `pi` binary, honouring `OCTL_PI_BIN` so tests can stub it.
fn pi_bin() -> String {
    std::env::var("OCTL_PI_BIN").unwrap_or_else(|_| "pi".to_string())
}

/// How to invoke pi for a chunk. The credential is never stored here — only the
/// provider/model selection and the *name* of the env var to read the key from.
#[derive(Debug, Clone)]
pub struct PiConfig {
    /// pi `--provider` (e.g. `"deepseek"`, `"anthropic"`).
    pub provider: String,
    /// pi `--model` (e.g. `"deepseek-v4-flash"`).
    pub model: String,
    /// Environment variable the provider key is read from. pi resolves its own
    /// auth, but the adapter checks this is present so a missing key fails fast.
    /// Default `DEEPSEEK_API_KEY`.
    pub api_key_env: String,
    /// Extra args inserted before the prompt positional (empty by default).
    pub extra_args: Vec<String>,
}

impl PiConfig {
    /// Config for a `DeepSeek`-backed pi run at `model` (key from `DEEPSEEK_API_KEY`).
    pub fn deepseek(model: impl Into<String>) -> Self {
        Self {
            provider: "deepseek".to_string(),
            model: model.into(),
            api_key_env: "DEEPSEEK_API_KEY".to_string(),
            extra_args: Vec::new(),
        }
    }
}

/// The pi [`CodeHarness`] adapter.
#[derive(Debug, Clone)]
pub struct PiHarness {
    config: PiConfig,
}

impl PiHarness {
    /// Build an adapter from config.
    pub fn new(config: PiConfig) -> Self {
        Self { config }
    }
}

impl AgentLaunch for PiHarness {
    fn capabilities(&self) -> HarnessCapabilities {
        HarnessCapabilities {
            // pi edits any file via its write/edit tools, tests included.
            can_author_tests: true,
            // Usage is lifted best-effort from pi's `--mode json` stream.
            reports_usage: true,
            // The prompt asks pi to stay in scope, but the adapter does not
            // *enforce* it — the deterministic floor (design §4) does.
            honors_file_scope: false,
            // The adapter runs the request's checks as the code-node self-check.
            runs_checks: true,
        }
    }

    fn check_credentials(&self) -> Result<(), HarnessError> {
        if !support::credential_present(&self.config.api_key_env) {
            return Err(HarnessError::MissingCredential {
                var: self.config.api_key_env.clone(),
            });
        }
        Ok(())
    }

    fn build_prompt(&self, req: &ChunkRequest) -> String {
        // pi does not auto-commit; the commit-framed prompt tells it to.
        support::commit_framed_prompt(req)
    }

    fn build_command(
        &self,
        worktree: &Path,
        _brief_file: &Path,
        prompt: &str,
        _req: &ChunkRequest,
    ) -> Command {
        let mut cmd = Command::new(pi_bin());
        cmd.current_dir(worktree)
            .arg("-p")
            .arg("--mode")
            .arg("json")
            .arg("--provider")
            .arg(&self.config.provider)
            .arg("--model")
            .arg(&self.config.model);
        for extra in &self.config.extra_args {
            cmd.arg(extra);
        }
        // pi's arg parser does NOT support a `--` end-of-options terminator — it
        // rejects it with `Error: Unknown option: --` and exits non-zero (the live
        // bake-off's exit-1). The prompt is passed as the sole positional message
        // (`pi [options] [messages...]`). pi also treats a leading-dash token as an
        // unknown option and offers no argv escape, so if the (commit-framed) prompt
        // begins with `-` we prepend a single space: invisible to the model, but it
        // keeps pi's parser from mistaking the prompt for a flag. Verified against
        // pi 0.82 — a space-prefixed dash arg parses as an ordinary message.
        let positional = if prompt.starts_with('-') {
            format!(" {prompt}")
        } else {
            prompt.to_string()
        };
        cmd.arg(positional);
        cmd
    }

    fn parse_usage(&self, transcript: &str) -> Option<Usage> {
        support::parse_json_usage(transcript)
    }

    fn tool_label(&self) -> &'static str {
        "pi"
    }

    fn bin_display(&self) -> String {
        pi_bin()
    }
}

impl CodeHarness for PiHarness {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::conformance::run_and_check;
    use crate::harness::{Check, ChunkOutcome};
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        crate::harness::support::test_env::lock()
    }

    fn write_script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        let mut perms = std::fs::metadata(&p).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&p, perms).unwrap();
        p
    }

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
    fn missing_credential_fails_fast() {
        let _g = env_lock();
        let repo = init_repo();
        std::env::remove_var("DEEPSEEK_API_KEY");
        let h = PiHarness::new(PiConfig::deepseek("deepseek-v4-flash"));
        let err = h
            .run_chunk(&base_request(repo.path()), &CancelToken::new())
            .unwrap_err();
        assert_eq!(
            err,
            HarnessError::MissingCredential {
                var: "DEEPSEEK_API_KEY".into()
            }
        );
    }

    #[test]
    fn commit_produced_maps_to_committed_with_json_usage() {
        let _g = env_lock();
        let repo = init_repo();
        let sdir = TempDir::new().unwrap();
        let bin = write_script(
            sdir.path(),
            "fake-pi.sh",
            "#!/bin/bash\n\
             printf 'edited\\n' > out.txt\n\
             git add out.txt\n\
             git commit -q -m 'chunk edit'\n\
             printf '%s\\n' '{\"type\":\"result\",\"usage\":{\"input_tokens\":80,\"output_tokens\":40},\"cost_usd\":0.002}'\n",
        );
        std::env::set_var("OCTL_PI_BIN", &bin);
        std::env::set_var("DEEPSEEK_API_KEY", "test-key");

        let h = PiHarness::new(PiConfig::deepseek("deepseek-v4-flash"));
        let res = run_and_check(&h, &base_request(repo.path())).unwrap();
        assert!(matches!(res.outcome, ChunkOutcome::Committed { .. }));
        assert_eq!(res.changed_files, vec![PathBuf::from("out.txt")]);
        let usage = res.usage.expect("json usage parsed");
        assert_eq!(usage.input_tokens, Some(80));
        assert_eq!(usage.output_tokens, Some(40));
        assert_eq!(usage.total_tokens, Some(120));
        assert_eq!(usage.cost_usd, Some(0.002));

        std::env::remove_var("OCTL_PI_BIN");
        std::env::remove_var("DEEPSEEK_API_KEY");
    }

    #[test]
    fn no_change_clean_exit() {
        let _g = env_lock();
        let repo = init_repo();
        let sdir = TempDir::new().unwrap();
        let bin = write_script(
            sdir.path(),
            "fake-pi.sh",
            "#!/bin/bash\necho '{\"type\":\"result\"}'\nexit 0\n",
        );
        std::env::set_var("OCTL_PI_BIN", &bin);
        std::env::set_var("DEEPSEEK_API_KEY", "test-key");
        let h = PiHarness::new(PiConfig::deepseek("deepseek-v4-flash"));
        let res = run_and_check(&h, &base_request(repo.path())).unwrap();
        assert_eq!(res.outcome, ChunkOutcome::NoChange);
        std::env::remove_var("OCTL_PI_BIN");
        std::env::remove_var("DEEPSEEK_API_KEY");
    }

    /// A fixture that dumps its argv (NUL-delimited) to `$OCTL_TEST_ARGV_OUT`,
    /// then exits 0 without editing — the outcome is irrelevant to an argv check.
    const ARGV_DUMP: &str =
        "#!/bin/bash\nprintf '%s\\0' \"$@\" > \"$OCTL_TEST_ARGV_OUT\"\nexit 0\n";

    fn captured_argv(argv_out: &Path) -> Vec<String> {
        let raw = std::fs::read(argv_out).unwrap();
        raw.split(|b| *b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| String::from_utf8_lossy(s).into_owned())
            .collect()
    }

    #[test]
    fn pi_argv_omits_option_terminator_and_ends_with_prompt() {
        let _g = env_lock();
        let repo = init_repo();
        let sdir = TempDir::new().unwrap();
        let argv_out = sdir.path().join("argv");
        let bin = write_script(sdir.path(), "fake-pi.sh", ARGV_DUMP);
        std::env::set_var("OCTL_PI_BIN", &bin);
        std::env::set_var("OCTL_TEST_ARGV_OUT", &argv_out);
        std::env::set_var("DEEPSEEK_API_KEY", "test-key");

        let h = PiHarness::new(PiConfig::deepseek("deepseek-v4-flash"));
        let res = run_and_check(&h, &base_request(repo.path())).unwrap();
        assert_eq!(res.outcome, ChunkOutcome::NoChange);

        let args = captured_argv(&argv_out);
        // pi rejects `--` (`Error: Unknown option: --`) — it must NOT appear, or a
        // live run exits 1 (the bug this fixes).
        assert!(
            !args.iter().any(|a| a == "--"),
            "pi must not receive a `--` terminator: {args:?}"
        );
        // The selection flags survive and the prompt is the sole trailing positional.
        assert!(args.iter().any(|a| a == "-p"));
        assert!(args
            .windows(2)
            .any(|w| w[0] == "--provider" && w[1] == "deepseek"));
        assert!(args
            .windows(2)
            .any(|w| w[0] == "--model" && w[1] == "deepseek-v4-flash"));
        // The last arg is the commit-framed prompt (begins with the brief text).
        assert!(
            args.last().unwrap().starts_with("do the thing"),
            "prompt is the final positional: {:?}",
            args.last()
        );

        std::env::remove_var("OCTL_PI_BIN");
        std::env::remove_var("OCTL_TEST_ARGV_OUT");
        std::env::remove_var("DEEPSEEK_API_KEY");
    }

    #[test]
    fn pi_dash_leading_prompt_is_space_guarded() {
        let _g = env_lock();
        let repo = init_repo();
        let sdir = TempDir::new().unwrap();
        let argv_out = sdir.path().join("argv");
        let bin = write_script(sdir.path(), "fake-pi.sh", ARGV_DUMP);
        std::env::set_var("OCTL_PI_BIN", &bin);
        std::env::set_var("OCTL_TEST_ARGV_OUT", &argv_out);
        std::env::set_var("DEEPSEEK_API_KEY", "test-key");

        // A brief that begins with a dash. pi has no `--` escape and rejects a
        // leading-dash positional as an unknown option, so the adapter must guard
        // it with a leading space — otherwise a valid brief exits 1 (regression).
        let h = PiHarness::new(PiConfig::deepseek("deepseek-v4-flash"));
        let mut req = base_request(repo.path());
        req.brief = "--delete the old parser".into();
        let _ = run_and_check(&h, &req).unwrap();

        let args = captured_argv(&argv_out);
        let last = args.last().unwrap();
        assert!(
            last.starts_with(' '),
            "a dash-leading prompt must be space-guarded so pi accepts it: {last:?}"
        );
        // The brief content survives verbatim after the guard space.
        assert!(last.trim_start().starts_with("--delete the old parser"));

        std::env::remove_var("OCTL_PI_BIN");
        std::env::remove_var("OCTL_TEST_ARGV_OUT");
        std::env::remove_var("DEEPSEEK_API_KEY");
    }

    #[test]
    fn spawn_failure_is_structured_error() {
        let _g = env_lock();
        let repo = init_repo();
        std::env::set_var("OCTL_PI_BIN", "/nonexistent/pi-xyz");
        std::env::set_var("DEEPSEEK_API_KEY", "test-key");
        let h = PiHarness::new(PiConfig::deepseek("deepseek-v4-flash"));
        let err = h
            .run_chunk(&base_request(repo.path()), &CancelToken::new())
            .unwrap_err();
        assert!(matches!(err, HarnessError::ProviderFailure { .. }));
        std::env::remove_var("OCTL_PI_BIN");
        std::env::remove_var("DEEPSEEK_API_KEY");
    }
}
