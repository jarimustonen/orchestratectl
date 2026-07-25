//! `aider` adapter — the first conforming [`CodeHarness`] (design.md §10:
//! "First adapter: aider").
//!
//! Drives `aider` non-interactively with the invocation proven by the
//! feasibility spike (`issues/code-pipeline`, task-0 spike):
//!
//! ```text
//! aider --model <resolved> --yes-always --no-check-update --no-analytics \
//!       --map-tokens 0 --message-file <brief> <files...>
//! ```
//!
//! It **commits but does not merge** (design §3 code role). The outcome is read
//! from the resulting *git* state — never from aider's stdout prose (design §10):
//! the worktree must be forked at `base_commit`; a forward HEAD advance is
//! [`ChunkOutcome::Committed`]; a clean no-op is [`ChunkOutcome::NoChange`];
//! uncommitted leftovers or a history rewrite are [`ChunkOutcome::Failed`].
//!
//! Credentials are never hardcoded: the model id and the credential env-var name
//! are [`AiderConfig`] inputs; the key itself is read from the environment the
//! caller set (default `DEEPSEEK_API_KEY`). Binaries honour the same override
//! convention the rest of the crate uses: `GIT_BIN` for git, `OCTL_AIDER_BIN` for
//! aider — so tests can stub either without a network call.
//!
//! The whole git-inspecting skeleton (preconditions → bounded run →
//! git-outcome mapping → self-checks → transcript capture) lives in
//! [`super::support`]; this adapter only supplies the aider-specific launch via
//! [`support::AgentLaunch`].

use std::path::Path;
use std::process::Command;

use super::support::{self, AgentLaunch};
use super::{
    CancelToken, ChunkRequest, ChunkResult, CodeHarness, HarnessCapabilities, HarnessError, Usage,
};

/// How to invoke aider for a chunk. The credential itself is never stored here —
/// only the *name* of the env var to read it from (design §10: routing config
/// stays out of the plan; the binding is recorded in execution events).
#[derive(Debug, Clone)]
pub struct AiderConfig {
    /// Resolved model id passed to `--model` (e.g. `deepseek/deepseek-chat`).
    pub model: String,
    /// Environment variable the provider key is read from. aider itself reads the
    /// key from the process environment; this adapter only *checks* it is present
    /// so a missing key fails fast with [`HarnessError::MissingCredential`]
    /// instead of an opaque provider error. Default `DEEPSEEK_API_KEY`.
    pub api_key_env: String,
    /// Extra args appended after the fixed flags (before the file list). Empty by
    /// default; lets a caller pass e.g. `--reasoning-effort` without a new field.
    pub extra_args: Vec<String>,
}

impl AiderConfig {
    /// Config for a given model, reading the key from `DEEPSEEK_API_KEY`.
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            api_key_env: "DEEPSEEK_API_KEY".to_string(),
            extra_args: Vec::new(),
        }
    }
}

/// The aider [`CodeHarness`] adapter.
#[derive(Debug, Clone)]
pub struct AiderHarness {
    config: AiderConfig,
}

impl AiderHarness {
    /// Build an adapter from config.
    pub fn new(config: AiderConfig) -> Self {
        Self { config }
    }
}

/// `aider` binary, honouring `OCTL_AIDER_BIN` so tests can point at a fixture
/// script that simulates an edit+commit without a network call.
fn aider_bin() -> String {
    std::env::var("OCTL_AIDER_BIN").unwrap_or_else(|_| "aider".to_string())
}

/// Best-effort parse of aider's usage summary from its transcript. aider prints
/// a line like `Tokens: 1.2k sent, 345 received. Cost: $0.0012 message, ...`.
/// Never fails: any field it cannot read stays `None`. Usage is provenance, not a
/// gate — so a fragile parse here can never affect correctness (design §10).
fn parse_usage(transcript: &str) -> Option<Usage> {
    let mut usage = Usage::default();
    let mut found = false;
    for line in transcript.lines() {
        if let Some(cost) = parse_cost(line) {
            usage.cost_usd = Some(cost);
            found = true;
        }
        if let Some((sent, received)) = parse_tokens(line) {
            usage.input_tokens = sent;
            usage.output_tokens = received;
            if let (Some(s), Some(r)) = (sent, received) {
                usage.total_tokens = Some(s + r);
            }
            found = true;
        }
    }
    found.then_some(usage)
}

/// Parse the first `$<float>` following a `Cost:` marker on a line.
fn parse_cost(line: &str) -> Option<f64> {
    let after = line.split("Cost:").nth(1)?;
    let dollar = after.split('$').nth(1)?;
    let num: String = dollar
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    num.parse::<f64>().ok()
}

/// Parse `Tokens: <n> sent, <n> received` (supporting a `k` suffix).
fn parse_tokens(line: &str) -> Option<(Option<u64>, Option<u64>)> {
    let after = line.split("Tokens:").nth(1)?;
    let sent = after
        .split("sent")
        .next()
        .and_then(|s| parse_token_count(s.trim()));
    let received = after
        .split("sent,")
        .nth(1)
        .and_then(|s| s.split("received").next())
        .and_then(|s| parse_token_count(s.trim()));
    if sent.is_none() && received.is_none() {
        return None;
    }
    Some((sent, received))
}

/// Parse a token count token like `1.2k`, `345`, or `2.0k`.
fn parse_token_count(s: &str) -> Option<u64> {
    let s = s.trim();
    let (num_part, mult) = if let Some(stripped) = s.strip_suffix(['k', 'K']) {
        (stripped, 1_000.0)
    } else if let Some(stripped) = s.strip_suffix(['m', 'M']) {
        (stripped, 1_000_000.0)
    } else {
        (s, 1.0)
    };
    let num: String = num_part
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    if num.is_empty() {
        return None;
    }
    // Guard the f64→u64 cast: reject negatives/NaN so usage never records a
    // saturated-garbage token count.
    num.parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && *v >= 0.0)
        .map(|v| (v * mult) as u64)
}

impl AgentLaunch for AiderHarness {
    fn capabilities(&self) -> HarnessCapabilities {
        HarnessCapabilities {
            // aider edits whatever files it is given, tests included.
            can_author_tests: true,
            // Best-effort usage is parsed from the transcript.
            reports_usage: true,
            // aider is scoped to the files passed on its argv, but the adapter
            // does not *enforce* it — the deterministic floor (design §4) does.
            honors_file_scope: false,
            // The adapter runs the request's checks as the code-node self-check.
            runs_checks: true,
        }
    }

    fn commits_in_agent(&self) -> bool {
        // aider is *expected* to auto-commit its edits, but a live run can finish
        // with the tree dirty and no commit (auto-commit disabled by aider's own
        // heuristics/config). Returning false makes the shared skeleton commit
        // whatever aider left after a clean exit, so a dirty tree becomes a
        // `Committed` result — matching the contract the Claude family + pi satisfy
        // by committing in-agent, instead of a spurious `Failed`.
        false
    }

    fn check_credentials(&self) -> Result<(), HarnessError> {
        // aider itself reads the key from the process environment; this adapter
        // only *checks* it is present so a missing key fails fast with a
        // structured error instead of an opaque provider failure downstream.
        if std::env::var(&self.config.api_key_env).is_err() {
            return Err(HarnessError::MissingCredential {
                var: self.config.api_key_env.clone(),
            });
        }
        Ok(())
    }

    fn build_prompt(&self, req: &ChunkRequest) -> String {
        // aider takes the brief verbatim via `--message-file`; it auto-commits
        // its edits, so no extra implement/commit framing is added here.
        req.brief.clone()
    }

    fn build_command(
        &self,
        worktree: &Path,
        brief_file: &Path,
        _prompt: &str,
        req: &ChunkRequest,
    ) -> Command {
        // Spike-proven flags (issues/code-pipeline task-0 spike).
        let mut cmd = Command::new(aider_bin());
        cmd.current_dir(worktree)
            .arg("--model")
            .arg(&self.config.model)
            .arg("--yes-always")
            .arg("--no-check-update")
            .arg("--no-analytics")
            .arg("--map-tokens")
            .arg("0")
            .arg("--message-file")
            .arg(brief_file);
        for extra in &self.config.extra_args {
            cmd.arg(extra);
        }
        // `--` terminates option parsing so a file named `--model` (or any
        // leading-dash path) can't inject an aider flag.
        cmd.arg("--");
        for file in &req.files {
            cmd.arg(file);
        }
        cmd
    }

    fn parse_usage(&self, transcript: &str) -> Option<Usage> {
        parse_usage(transcript)
    }

    fn tool_label(&self) -> &'static str {
        "aider"
    }

    fn bin_display(&self) -> String {
        aider_bin()
    }
}

impl CodeHarness for AiderHarness {
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
    use crate::harness::conformance::{run_and_check, run_and_check_with_cancel};
    use crate::harness::{Check, ChunkOutcome};
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use tempfile::TempDir;

    // aider tests mutate process env (OCTL_AIDER_BIN, DEEPSEEK_API_KEY,
    // GIT_BIN); serialize them so parallel runners don't cross-contaminate.

    /// Acquire the env lock, tolerating a prior test's panic (poison) so one
    /// failure does not cascade into spurious `PoisonError` failures.
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
    /// against genuine git state (aider is the only stubbed boundary).
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

    /// Current HEAD oid of a repo (real git).
    fn head_of(worktree: &Path) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(worktree)
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// Subject line of the repo's HEAD commit (real git).
    fn head_message(worktree: &Path) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(worktree)
            .args(["log", "-1", "--format=%s"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// A request whose `base_commit` is the repo's real HEAD (the adapter now
    /// verifies the two agree). Uses a bogus base only where the test returns
    /// before the base check (credential / dirty guards).
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
        let h = AiderHarness::new(AiderConfig::new("deepseek/deepseek-chat"));
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
    fn dirty_worktree_is_rejected() {
        let _g = env_lock();
        let repo = init_repo();
        std::fs::write(repo.path().join("dirt.txt"), "x").unwrap();
        std::env::set_var("DEEPSEEK_API_KEY", "test-key");
        let h = AiderHarness::new(AiderConfig::new("m"));
        let err = h
            .run_chunk(&base_request(repo.path()), &CancelToken::new())
            .unwrap_err();
        assert!(matches!(err, HarnessError::DirtyWorktree { .. }));
        std::env::remove_var("DEEPSEEK_API_KEY");
    }

    #[test]
    fn commit_produced_maps_to_committed() {
        let _g = env_lock();
        let repo = init_repo();
        // Fake aider: writes a file, commits it, prints a usage line.
        let sdir = TempDir::new().unwrap();
        let bin = write_script(
            sdir.path(),
            "fake-aider.sh",
            "#!/bin/bash\n\
             printf 'edited\\n' > out.txt\n\
             git add out.txt\n\
             git commit -q -m 'chunk edit'\n\
             echo 'Tokens: 1.2k sent, 300 received. Cost: $0.0004 message, $0.0004 session.'\n",
        );
        std::env::set_var("OCTL_AIDER_BIN", &bin);
        std::env::set_var("DEEPSEEK_API_KEY", "test-key");

        let h = AiderHarness::new(AiderConfig::new("m"));
        let req = base_request(repo.path());
        let res = run_and_check(&h, &req).unwrap();

        assert!(matches!(res.outcome, ChunkOutcome::Committed { .. }));
        assert!(res.resulting_commit.is_some());
        assert_eq!(res.changed_files, vec![PathBuf::from("out.txt")]);
        assert_eq!(res.check_results.len(), 1);
        assert!(res.check_results[0].passed);
        assert!(res.transcript_ref.is_some());
        // Usage was parsed best-effort from the transcript.
        let usage = res.usage.expect("usage parsed");
        assert_eq!(usage.cost_usd, Some(0.0004));
        assert_eq!(usage.input_tokens, Some(1200));
        assert_eq!(usage.output_tokens, Some(300));

        std::env::remove_var("OCTL_AIDER_BIN");
        std::env::remove_var("DEEPSEEK_API_KEY");
    }

    #[test]
    fn no_commit_clean_exit_maps_to_no_change() {
        let _g = env_lock();
        let repo = init_repo();
        let sdir = TempDir::new().unwrap();
        let bin = write_script(
            sdir.path(),
            "fake-aider.sh",
            "#!/bin/bash\necho 'nothing to do'\nexit 0\n",
        );
        std::env::set_var("OCTL_AIDER_BIN", &bin);
        std::env::set_var("DEEPSEEK_API_KEY", "test-key");

        let h = AiderHarness::new(AiderConfig::new("m"));
        let req = base_request(repo.path());
        let res = run_and_check(&h, &req).unwrap();
        assert_eq!(res.outcome, ChunkOutcome::NoChange);
        assert!(res.resulting_commit.is_none());
        assert!(res.changed_files.is_empty());

        std::env::remove_var("OCTL_AIDER_BIN");
        std::env::remove_var("DEEPSEEK_API_KEY");
    }

    #[test]
    fn no_commit_nonzero_exit_maps_to_failed() {
        let _g = env_lock();
        let repo = init_repo();
        let sdir = TempDir::new().unwrap();
        let bin = write_script(
            sdir.path(),
            "fake-aider.sh",
            "#!/bin/bash\necho 'provider blew up' >&2\nexit 3\n",
        );
        std::env::set_var("OCTL_AIDER_BIN", &bin);
        std::env::set_var("DEEPSEEK_API_KEY", "test-key");

        let h = AiderHarness::new(AiderConfig::new("m"));
        let req = base_request(repo.path());
        let res = run_and_check(&h, &req).unwrap();
        assert!(matches!(res.outcome, ChunkOutcome::Failed { .. }));

        std::env::remove_var("OCTL_AIDER_BIN");
        std::env::remove_var("DEEPSEEK_API_KEY");
    }

    #[test]
    fn provider_spawn_failure_is_structured_error() {
        let _g = env_lock();
        let repo = init_repo();
        std::env::set_var("OCTL_AIDER_BIN", "/nonexistent/aider-xyz");
        std::env::set_var("DEEPSEEK_API_KEY", "test-key");

        let h = AiderHarness::new(AiderConfig::new("m"));
        let err = h
            .run_chunk(&base_request(repo.path()), &CancelToken::new())
            .unwrap_err();
        assert!(matches!(err, HarnessError::ProviderFailure { .. }));

        std::env::remove_var("OCTL_AIDER_BIN");
        std::env::remove_var("DEEPSEEK_API_KEY");
    }

    #[test]
    fn check_failure_still_committed() {
        let _g = env_lock();
        let repo = init_repo();
        let sdir = TempDir::new().unwrap();
        let bin = write_script(
            sdir.path(),
            "fake-aider.sh",
            "#!/bin/bash\nprintf 'x\\n' > out.txt\ngit add out.txt\ngit commit -q -m e\n",
        );
        std::env::set_var("OCTL_AIDER_BIN", &bin);
        std::env::set_var("DEEPSEEK_API_KEY", "test-key");

        let h = AiderHarness::new(AiderConfig::new("m"));
        let mut req = base_request(repo.path());
        req.checks = vec![Check {
            id: "chk1".into(),
            desc: "always fails".into(),
            run: "exit 1".into(),
            timeout: None,
        }];
        let res = run_and_check(&h, &req).unwrap();
        assert!(matches!(res.outcome, ChunkOutcome::Committed { .. }));
        assert_eq!(res.check_results.len(), 1);
        assert!(!res.check_results[0].passed);
        assert_eq!(res.check_results[0].exit_code, Some(1));
        assert_eq!(res.check_results[0].check_id, "chk1");

        std::env::remove_var("OCTL_AIDER_BIN");
        std::env::remove_var("DEEPSEEK_API_KEY");
    }

    #[test]
    fn base_commit_mismatch_is_rejected() {
        let _g = env_lock();
        let repo = init_repo();
        // Add a second commit so HEAD advances past the request's base_commit.
        std::fs::write(repo.path().join("seed.txt"), "seed2\n").unwrap();
        Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .args(["commit", "-qam", "advance"])
            .output()
            .unwrap();
        // base_request reads the (new) HEAD, so rewind the request's base to the
        // parent — the worktree HEAD now disagrees with the declared base.
        std::env::set_var("DEEPSEEK_API_KEY", "test-key");
        let mut req = base_request(repo.path());
        req.base_commit = head_of(repo.path()) + "~1";
        let h = AiderHarness::new(AiderConfig::new("m"));
        // Resolve the parent to a real oid so it's a *mismatch*, not an unknown ref.
        let parent = String::from_utf8_lossy(
            &Command::new("git")
                .arg("-C")
                .arg(repo.path())
                .args(["rev-parse", "HEAD~1"])
                .output()
                .unwrap()
                .stdout,
        )
        .trim()
        .to_string();
        req.base_commit = parent;
        let err = h.run_chunk(&req, &CancelToken::new()).unwrap_err();
        assert!(matches!(err, HarnessError::InvalidWorktree { .. }));
        std::env::remove_var("DEEPSEEK_API_KEY");
    }

    #[test]
    fn uncommitted_edits_are_committed_by_the_adapter() {
        let _g = env_lock();
        let repo = init_repo();
        // Fake aider that mimics the live bug: it edits a tracked file AND creates
        // a new untracked file, but (auto-commit no-op) never commits. The adapter
        // must land those edits as a commit itself — the whole point of
        // `commits_in_agent() == false` — so the result is `Committed`, not the old
        // `Failed`.
        let sdir = TempDir::new().unwrap();
        let bin = write_script(
            sdir.path(),
            "fake-aider.sh",
            "#!/bin/bash\nprintf 'dirty\\n' >> seed.txt\nprintf 'new\\n' > out.txt\nexit 0\n",
        );
        std::env::set_var("OCTL_AIDER_BIN", &bin);
        std::env::set_var("DEEPSEEK_API_KEY", "test-key");

        let h = AiderHarness::new(AiderConfig::new("m"));
        let req = base_request(repo.path());
        let res = run_and_check(&h, &req).unwrap();
        // The adapter committed aider's leftover edits (tracked + untracked).
        assert!(matches!(res.outcome, ChunkOutcome::Committed { .. }));
        assert!(res.resulting_commit.is_some());
        let mut files = res.changed_files.clone();
        files.sort();
        assert_eq!(
            files,
            vec![PathBuf::from("out.txt"), PathBuf::from("seed.txt")]
        );
        // The self-check still runs against the committed state.
        assert_eq!(res.check_results.len(), 1);
        assert!(res.check_results[0].passed);
        // The adapter commit is auditable: its message names the tool + chunk and
        // marks it adapter-committed (so downstream never confuses it with an
        // aider-authored auto-commit).
        assert_eq!(
            head_message(repo.path()),
            "aider: chunk c1 attempt a1 (adapter-committed)"
        );

        std::env::remove_var("OCTL_AIDER_BIN");
        std::env::remove_var("DEEPSEEK_API_KEY");
    }

    #[test]
    fn aider_scratch_droppings_are_not_committed() {
        let _g = env_lock();
        let repo = init_repo();
        // Fake aider edits a real deliverable AND drops its own history/cache files
        // in the worktree, then exits 0 without committing. The adapter must commit
        // the deliverable but NOT aider's `.aider.*` scratch files — regardless of
        // whether the repo's .gitignore covers them (this repo's does not).
        let sdir = TempDir::new().unwrap();
        let bin = write_script(
            sdir.path(),
            "fake-aider.sh",
            "#!/bin/bash\n\
             printf 'real\\n' > out.txt\n\
             printf 'chat\\n' > .aider.chat.history.md\n\
             mkdir -p .aider.tags.cache.v4 && printf 'x\\n' > .aider.tags.cache.v4/cache.db\n\
             exit 0\n",
        );
        std::env::set_var("OCTL_AIDER_BIN", &bin);
        std::env::set_var("DEEPSEEK_API_KEY", "test-key");

        let h = AiderHarness::new(AiderConfig::new("m"));
        let res = run_and_check(&h, &base_request(repo.path())).unwrap();
        assert!(matches!(res.outcome, ChunkOutcome::Committed { .. }));
        // Only the deliverable was committed; no `.aider*` path leaked into the diff.
        assert_eq!(res.changed_files, vec![PathBuf::from("out.txt")]);
        assert!(
            !res.changed_files
                .iter()
                .any(|p| p.to_string_lossy().contains(".aider")),
            "aider scratch files must not be committed: {:?}",
            res.changed_files
        );
        // The scratch files still exist on disk (untracked) — we excluded, not deleted.
        assert!(repo.path().join(".aider.chat.history.md").exists());

        std::env::remove_var("OCTL_AIDER_BIN");
        std::env::remove_var("DEEPSEEK_API_KEY");
    }

    #[test]
    fn history_rewrite_left_dirty_is_not_a_commit() {
        let _g = env_lock();
        let repo = init_repo();
        // Fake aider rewrites history onto an orphan root and leaves the tree dirty
        // WITHOUT committing (exit 0). The adapter's leftover-commit lands on the
        // rewritten history, but base_commit is no longer an ancestor of the new
        // HEAD — so the git-outcome mapping must still refuse it as `Failed`, never
        // launder a rewrite into a `Committed`.
        let sdir = TempDir::new().unwrap();
        let bin = write_script(
            sdir.path(),
            "fake-aider.sh",
            "#!/bin/bash\n\
             git checkout -q --orphan rogue\n\
             git rm -q -rf . >/dev/null 2>&1\n\
             printf 'x\\n' > other.txt\n\
             exit 0\n",
        );
        std::env::set_var("OCTL_AIDER_BIN", &bin);
        std::env::set_var("DEEPSEEK_API_KEY", "test-key");

        let h = AiderHarness::new(AiderConfig::new("m"));
        let res = run_and_check(&h, &base_request(repo.path())).unwrap();
        assert!(matches!(res.outcome, ChunkOutcome::Failed { .. }));
        assert!(res.resulting_commit.is_none());

        std::env::remove_var("OCTL_AIDER_BIN");
        std::env::remove_var("DEEPSEEK_API_KEY");
    }

    #[test]
    fn nonzero_exit_leaves_dirty_tree_uncommitted() {
        let _g = env_lock();
        let repo = init_repo();
        // aider exits non-zero AND leaves edits behind. The adapter must NOT
        // fabricate a commit over a failed run — the dirty tree maps to `Failed`,
        // and the leftover edits survive for the supervisor to reset.
        let sdir = TempDir::new().unwrap();
        let bin = write_script(
            sdir.path(),
            "fake-aider.sh",
            "#!/bin/bash\nprintf 'dirty\\n' >> seed.txt\necho boom >&2\nexit 3\n",
        );
        std::env::set_var("OCTL_AIDER_BIN", &bin);
        std::env::set_var("DEEPSEEK_API_KEY", "test-key");

        let h = AiderHarness::new(AiderConfig::new("m"));
        let req = base_request(repo.path());
        let res = run_and_check(&h, &req).unwrap();
        assert!(matches!(res.outcome, ChunkOutcome::Failed { .. }));
        assert!(res.resulting_commit.is_none());

        std::env::remove_var("OCTL_AIDER_BIN");
        std::env::remove_var("DEEPSEEK_API_KEY");
    }

    #[test]
    fn history_rewrite_is_not_a_commit() {
        let _g = env_lock();
        let repo = init_repo();
        // Fake aider resets HEAD backwards to an unrelated new root — HEAD moves
        // but base_commit is no longer an ancestor.
        let sdir = TempDir::new().unwrap();
        let bin = write_script(
            sdir.path(),
            "fake-aider.sh",
            "#!/bin/bash\n\
             git checkout -q --orphan rogue\n\
             git rm -q -rf . >/dev/null 2>&1\n\
             printf 'x\\n' > other.txt\n\
             git add other.txt\n\
             git commit -q -m rogue\n",
        );
        std::env::set_var("OCTL_AIDER_BIN", &bin);
        std::env::set_var("DEEPSEEK_API_KEY", "test-key");

        let h = AiderHarness::new(AiderConfig::new("m"));
        let req = base_request(repo.path());
        let res = run_and_check(&h, &req).unwrap();
        assert!(matches!(res.outcome, ChunkOutcome::Failed { .. }));
        assert!(res.resulting_commit.is_none());

        std::env::remove_var("OCTL_AIDER_BIN");
        std::env::remove_var("DEEPSEEK_API_KEY");
    }

    #[test]
    fn multiple_commits_reported_as_committed_tip_with_full_diff() {
        let _g = env_lock();
        let repo = init_repo();
        // Fake aider makes TWO commits forward; the range base..tip spans both.
        let sdir = TempDir::new().unwrap();
        let bin = write_script(
            sdir.path(),
            "fake-aider.sh",
            "#!/bin/bash\n\
             printf 'a\\n' > a.txt && git add a.txt && git commit -q -m a\n\
             printf 'b\\n' > out.txt && git add out.txt && git commit -q -m b\n",
        );
        std::env::set_var("OCTL_AIDER_BIN", &bin);
        std::env::set_var("DEEPSEEK_API_KEY", "test-key");

        let h = AiderHarness::new(AiderConfig::new("m"));
        let req = base_request(repo.path());
        let res = run_and_check(&h, &req).unwrap();
        assert!(matches!(res.outcome, ChunkOutcome::Committed { .. }));
        let mut files = res.changed_files.clone();
        files.sort();
        assert_eq!(
            files,
            vec![PathBuf::from("a.txt"), PathBuf::from("out.txt")]
        );

        std::env::remove_var("OCTL_AIDER_BIN");
        std::env::remove_var("DEEPSEEK_API_KEY");
    }

    // ---- Execution control: timeout + cancellation (real subprocess). ----

    #[test]
    fn timeout_kills_hung_aider() {
        let _g = env_lock();
        let repo = init_repo();
        // A fake aider that hangs far past the deadline (and forks a grandchild,
        // so the process-group kill is exercised).
        let sdir = TempDir::new().unwrap();
        let bin = write_script(
            sdir.path(),
            "fake-aider.sh",
            "#!/bin/bash\nsleep 30 & sleep 30\n",
        );
        std::env::set_var("OCTL_AIDER_BIN", &bin);
        std::env::set_var("DEEPSEEK_API_KEY", "test-key");

        let h = AiderHarness::new(AiderConfig::new("m"));
        let mut req = base_request(repo.path());
        req.timeout = Some(std::time::Duration::from_millis(200));

        let start = std::time::Instant::now();
        let res = run_and_check(&h, &req).unwrap();
        assert_eq!(res.outcome, ChunkOutcome::Timeout);
        assert!(res.resulting_commit.is_none());
        assert!(res.changed_files.is_empty());
        assert!(res.check_results.is_empty());
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "timeout must fire promptly, not wait for the hung child"
        );

        std::env::remove_var("OCTL_AIDER_BIN");
        std::env::remove_var("DEEPSEEK_API_KEY");
    }

    #[test]
    fn cancel_before_run_returns_cancelled_without_spawning() {
        let _g = env_lock();
        let repo = init_repo();
        // If aider *were* spawned it would commit; a pre-tripped cancel must
        // short-circuit before that, so no commit is produced.
        let sdir = TempDir::new().unwrap();
        let bin = write_script(
            sdir.path(),
            "fake-aider.sh",
            "#!/bin/bash\nprintf 'x\\n' > out.txt\ngit add out.txt\ngit commit -q -m e\n",
        );
        std::env::set_var("OCTL_AIDER_BIN", &bin);
        std::env::set_var("DEEPSEEK_API_KEY", "test-key");

        let h = AiderHarness::new(AiderConfig::new("m"));
        let req = base_request(repo.path());
        let before = head_of(repo.path());
        let cancel = CancelToken::new();
        cancel.cancel();
        let res = run_and_check_with_cancel(&h, &req, &cancel).unwrap();
        assert_eq!(res.outcome, ChunkOutcome::Cancelled);
        assert!(res.resulting_commit.is_none());
        // HEAD did not move — aider never ran.
        assert_eq!(head_of(repo.path()), before);

        std::env::remove_var("OCTL_AIDER_BIN");
        std::env::remove_var("DEEPSEEK_API_KEY");
    }

    #[test]
    fn cancel_in_flight_aborts_aider() {
        let _g = env_lock();
        let repo = init_repo();
        let sdir = TempDir::new().unwrap();
        let bin = write_script(sdir.path(), "fake-aider.sh", "#!/bin/bash\nsleep 30\n");
        std::env::set_var("OCTL_AIDER_BIN", &bin);
        std::env::set_var("DEEPSEEK_API_KEY", "test-key");

        let h = AiderHarness::new(AiderConfig::new("m"));
        let req = base_request(repo.path());
        // No timeout — the only way out is the cancel tripped mid-run.
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
        assert!(res.resulting_commit.is_none());
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "cancel must abort promptly, not wait for the hung child"
        );

        std::env::remove_var("OCTL_AIDER_BIN");
        std::env::remove_var("DEEPSEEK_API_KEY");
    }

    #[test]
    fn per_check_timeout_kills_wedged_check() {
        let _g = env_lock();
        let repo = init_repo();
        // aider commits fine; the self-check then hangs and must be killed by its
        // own per-check timeout without stalling the chunk.
        let sdir = TempDir::new().unwrap();
        let bin = write_script(
            sdir.path(),
            "fake-aider.sh",
            "#!/bin/bash\nprintf 'x\\n' > out.txt\ngit add out.txt\ngit commit -q -m e\n",
        );
        std::env::set_var("OCTL_AIDER_BIN", &bin);
        std::env::set_var("DEEPSEEK_API_KEY", "test-key");

        let h = AiderHarness::new(AiderConfig::new("m"));
        let mut req = base_request(repo.path());
        req.checks = vec![Check {
            id: "slow".into(),
            desc: "wedged".into(),
            run: "sleep 30".into(),
            timeout: Some(std::time::Duration::from_millis(200)),
        }];

        let start = std::time::Instant::now();
        let res = run_and_check(&h, &req).unwrap();
        // The chunk still committed; only the check was killed.
        assert!(matches!(res.outcome, ChunkOutcome::Committed { .. }));
        assert_eq!(res.check_results.len(), 1);
        assert!(!res.check_results[0].passed);
        // Killed by a signal — no clean exit code.
        assert_eq!(res.check_results[0].exit_code, None);
        assert!(res.check_results[0].stderr.contains("exceeded its timeout"));
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "a wedged check must be killed by its timeout, not hang the chunk"
        );

        std::env::remove_var("OCTL_AIDER_BIN");
        std::env::remove_var("DEEPSEEK_API_KEY");
    }

    #[test]
    fn cancel_during_checks_keeps_commit_and_completes_check_results() {
        let _g = env_lock();
        let repo = init_repo();
        // aider commits, then touches a sentinel so the test can trip the cancel
        // *deterministically* only after the commit exists (no timing race). The
        // first check blocks (`sleep 30`), so the trip lands during the check
        // phase: the running check is killed and every later check is recorded as
        // "skipped" without being spawned. Completeness (a result per requested
        // check) holds, and the produced commit is NOT demoted to Cancelled.
        let sdir = TempDir::new().unwrap();
        let sentinel = sdir.path().join("committed");
        let bin = write_script(
            sdir.path(),
            "fake-aider.sh",
            &format!(
                "#!/bin/bash\n\
                 printf 'x\\n' > out.txt\n\
                 git add out.txt\n\
                 git commit -q -m e\n\
                 touch {}\n",
                sentinel.display()
            ),
        );
        std::env::set_var("OCTL_AIDER_BIN", &bin);
        std::env::set_var("DEEPSEEK_API_KEY", "test-key");

        let h = AiderHarness::new(AiderConfig::new("m"));
        let mut req = base_request(repo.path());
        req.checks = vec![
            Check {
                id: "c1".into(),
                desc: "blocks".into(),
                run: "sleep 30".into(),
                timeout: None,
            },
            Check {
                id: "c2".into(),
                desc: "would pass".into(),
                run: "true".into(),
                timeout: None,
            },
        ];
        let cancel = CancelToken::new();
        let trip = cancel.clone();
        let sentinel_seen = sentinel.clone();
        let handle = std::thread::spawn(move || {
            // Wait until aider has committed, then trip — guarantees the commit
            // exists and we are (about to be) in the check phase.
            while !sentinel_seen.exists() {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            trip.cancel();
        });

        let start = std::time::Instant::now();
        let res = run_and_check_with_cancel(&h, &req, &cancel).unwrap();
        handle.join().unwrap();
        // The commit survives — cancellation during the tail does not erase it.
        assert!(matches!(res.outcome, ChunkOutcome::Committed { .. }));
        // Completeness: a result for BOTH requested checks.
        assert_eq!(res.check_results.len(), 2);
        assert_eq!(res.check_results[0].check_id, "c1");
        assert_eq!(res.check_results[1].check_id, "c2");
        assert!(res.check_results.iter().all(|c| !c.passed));
        // c2 was skipped, never spawned.
        assert!(res.check_results[1].stderr.contains("not run"));
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "the blocked check must be killed by cancel, not run to completion"
        );

        std::env::remove_var("OCTL_AIDER_BIN");
        std::env::remove_var("DEEPSEEK_API_KEY");
    }

    #[test]
    fn parse_usage_reads_cost_and_tokens() {
        let u = parse_usage("Tokens: 2.0k sent, 500 received. Cost: $0.01 message.").unwrap();
        assert_eq!(u.input_tokens, Some(2000));
        assert_eq!(u.output_tokens, Some(500));
        assert_eq!(u.total_tokens, Some(2500));
        assert_eq!(u.cost_usd, Some(0.01));
    }

    #[test]
    fn parse_usage_handles_million_suffix() {
        let u = parse_usage("Tokens: 1.5M sent, 2k received. Cost: $1.20 message.").unwrap();
        assert_eq!(u.input_tokens, Some(1_500_000));
        assert_eq!(u.output_tokens, Some(2_000));
        assert_eq!(u.cost_usd, Some(1.20));
    }

    #[test]
    fn parse_usage_absent_is_none() {
        assert!(parse_usage("no accounting here\n").is_none());
    }
}
