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
//! from the resulting *git* state — the commit at `HEAD` vs `base_commit` — never
//! from aider's stdout prose (design §10). If no commit was produced, the result
//! is synthesized as [`ChunkOutcome::NoChange`].
//!
//! Credentials are never hardcoded: the model id and the credential env-var name
//! are [`AiderConfig`] inputs; the key itself is read from the environment the
//! caller set (default `DEEPSEEK_API_KEY`). Binaries honour the same override
//! convention the rest of the crate uses: `GIT_BIN` for git, `OCTL_AIDER_BIN` for
//! aider — so tests can stub either without a network call.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::{
    Check, CheckResult, ChunkOutcome, ChunkRequest, ChunkResult, CodeHarness, HarnessCapabilities,
    HarnessError, Usage, HARNESS_CONTRACT_VERSION,
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

/// `git` binary, honouring the crate-wide `GIT_BIN` override (mirrors
/// `supervise::cleanup::git_bin`).
fn git_bin() -> String {
    std::env::var("GIT_BIN").unwrap_or_else(|_| "git".to_string())
}

/// `aider` binary, honouring `OCTL_AIDER_BIN` so tests can point at a fixture
/// script that simulates an edit+commit without a network call.
fn aider_bin() -> String {
    std::env::var("OCTL_AIDER_BIN").unwrap_or_else(|_| "aider".to_string())
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

/// Files changed between two commits (`git diff --name-only base..head`).
fn changed_files(worktree: &Path, base: &str, head: &str) -> Vec<PathBuf> {
    let Ok(out) = git(
        worktree,
        &["diff", "--name-only", &format!("{base}..{head}")],
    ) else {
        return Vec::new();
    };
    out.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(PathBuf::from)
        .collect()
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

/// Run one executable check via `sh -c` in the worktree.
fn run_check(worktree: &Path, check: &Check) -> CheckResult {
    let out = Command::new("sh")
        .arg("-c")
        .arg(&check.run)
        .current_dir(worktree)
        .output();
    match out {
        Ok(o) => CheckResult {
            desc: check.desc.clone(),
            run: check.run.clone(),
            passed: o.status.success(),
            exit_code: o.status.code(),
            stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        },
        Err(e) => CheckResult {
            desc: check.desc.clone(),
            run: check.run.clone(),
            passed: false,
            exit_code: None,
            stdout: String::new(),
            stderr: format!("could not spawn check: {e}"),
        },
    }
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
    let (num_part, mult) =
        if let Some(stripped) = s.strip_suffix('k').or_else(|| s.strip_suffix('K')) {
            (stripped, 1000.0)
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
    num.parse::<f64>().ok().map(|v| (v * mult) as u64)
}

impl CodeHarness for AiderHarness {
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

    fn run_chunk(&self, req: &ChunkRequest) -> Result<ChunkResult, HarnessError> {
        let worktree = req.worktree_path.as_path();

        // 1. Fail fast if the credential the provider needs is absent — a clear
        //    structured error beats an opaque provider failure downstream.
        if std::env::var(&self.config.api_key_env).is_err() {
            return Err(HarnessError::MissingCredential {
                var: self.config.api_key_env.clone(),
            });
        }

        // 2. Refuse a dirty worktree: prior uncommitted edits would commingle
        //    with the chunk and defeat the base..HEAD diff.
        let status = worktree_status(worktree)?;
        if !status.is_empty() {
            return Err(HarnessError::DirtyWorktree { details: status });
        }

        // Record the pre-run HEAD. `base_commit` is what the chunk *should* be
        // forked from; HEAD is what it actually is. We diff against HEAD so a
        // resulting commit is detected even if the two momentarily disagree.
        let base_head = head_oid(worktree)?;

        // 3. Materialize the brief + transcript artifacts.
        let dir = artifact_dir(req);
        std::fs::create_dir_all(&dir).map_err(|e| HarnessError::Internal {
            message: format!("could not create artifact dir {}: {e}", dir.display()),
        })?;
        let brief_file = dir.join("brief.md");
        std::fs::write(&brief_file, &req.brief).map_err(|e| HarnessError::Internal {
            message: format!("could not write brief {}: {e}", brief_file.display()),
        })?;
        let transcript_file = dir.join("transcript.log");

        // 4. Build + run the aider invocation (spike-proven flags).
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
            .arg(&brief_file);
        for extra in &self.config.extra_args {
            cmd.arg(extra);
        }
        for file in &req.files {
            cmd.arg(file);
        }

        let output = cmd.output().map_err(|e| HarnessError::ProviderFailure {
            message: format!("could not run aider ({}): {e}", aider_bin()),
        })?;

        // Persist a transcript regardless of outcome (provenance).
        let mut transcript = String::from_utf8_lossy(&output.stdout).into_owned();
        transcript.push_str(&String::from_utf8_lossy(&output.stderr));
        let transcript_ref = match std::fs::write(&transcript_file, &transcript) {
            Ok(()) => Some(transcript_file),
            Err(_) => None,
        };

        // 5. Read the outcome from git, never from aider's prose.
        let new_head = head_oid(worktree)?;
        let outcome = if new_head == base_head {
            if output.status.success() {
                // Ran cleanly, changed nothing.
                ChunkOutcome::NoChange
            } else {
                // Ran, produced no commit, and signalled failure.
                ChunkOutcome::Failed {
                    reason: format!(
                        "aider exited {} with no commit produced",
                        output
                            .status
                            .code()
                            .map_or("signal".to_string(), |c| c.to_string())
                    ),
                }
            }
        } else {
            ChunkOutcome::Committed {
                commit: new_head.clone(),
            }
        };

        let (resulting_commit, files) = match &outcome {
            ChunkOutcome::Committed { commit } => (
                Some(commit.clone()),
                changed_files(worktree, &base_head, &new_head),
            ),
            _ => (None, Vec::new()),
        };

        // 6. Run the self-check(s) — regardless of outcome, so a NoChange still
        //    reports the current check state.
        let check_results: Vec<CheckResult> =
            req.checks.iter().map(|c| run_check(worktree, c)).collect();

        let usage = parse_usage(&transcript);

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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::conformance::assert_result_conforms;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Mutex;
    use tempfile::TempDir;

    // aider tests mutate process env (OCTL_AIDER_BIN, DEEPSEEK_API_KEY,
    // GIT_BIN); serialize them so parallel runners don't cross-contaminate.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Acquire the env lock, tolerating a prior test's panic (poison) so one
    /// failure does not cascade into spurious `PoisonError` failures.
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
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

    fn base_request(worktree: &Path) -> ChunkRequest {
        ChunkRequest {
            run_id: "run1".into(),
            chunk_id: "c1".into(),
            attempt_id: "a1".into(),
            worktree_path: worktree.to_path_buf(),
            base_commit: "0".repeat(40),
            plan_rev: "v1".into(),
            brief: "do the thing".into(),
            checks: vec![Check {
                desc: "always passes".into(),
                run: "true".into(),
            }],
            files: vec![PathBuf::from("out.txt")],
        }
    }

    #[test]
    fn missing_credential_fails_fast() {
        let _g = env_lock();
        let repo = init_repo();
        std::env::remove_var("DEEPSEEK_API_KEY");
        let h = AiderHarness::new(AiderConfig::new("deepseek/deepseek-chat"));
        let err = h.run_chunk(&base_request(repo.path())).unwrap_err();
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
        let err = h.run_chunk(&base_request(repo.path())).unwrap_err();
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
        let res = h.run_chunk(&req).unwrap();
        assert_result_conforms(&req, &res).unwrap();

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
        let res = h.run_chunk(&req).unwrap();
        assert_result_conforms(&req, &res).unwrap();
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
        let res = h.run_chunk(&req).unwrap();
        assert_result_conforms(&req, &res).unwrap();
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
        let err = h.run_chunk(&base_request(repo.path())).unwrap_err();
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
            desc: "always fails".into(),
            run: "exit 1".into(),
        }];
        let res = h.run_chunk(&req).unwrap();
        assert_result_conforms(&req, &res).unwrap();
        assert!(matches!(res.outcome, ChunkOutcome::Committed { .. }));
        assert_eq!(res.check_results.len(), 1);
        assert!(!res.check_results[0].passed);
        assert_eq!(res.check_results[0].exit_code, Some(1));

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
    fn parse_usage_absent_is_none() {
        assert!(parse_usage("no accounting here\n").is_none());
    }
}
