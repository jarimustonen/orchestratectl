//! Bounded subprocess execution shared across the CLI.
//!
//! One timeout + process-group-kill + capped-capture implementation, lifted
//! from the original tmux-specific watchdog `run_timed` (the
//! `spinoff-issuectl-subprocess-bounds` work). Both the liveness watchdog
//! ([`crate::supervise::watchdog`]) and spin-off materialization
//! ([`crate::spinoff::approve`]) drive untrusted/foreign binaries (`tmux`,
//! `issuectl`) that can wedge, stream unbounded output, or fork children — so
//! every such spawn goes through here rather than `Command::output()`:
//!
//! - **Timeout.** The std library has no built-in process timeout, so this
//!   polls [`std::process::Child::try_wait`] against a deadline and SIGKILLs the
//!   child's process *group* on overrun (the child is placed in its own group
//!   via `process_group(0)`, so the group kill reaps any subprocess it forked
//!   and closes the pipes, releasing the reader threads).
//! - **Output cap.** stdout and stderr are each drained on a helper thread (so a
//!   child that fills one pipe cannot dead-lock waiting for us to read the
//!   other) but only the first `cap` bytes are retained — a runaway producer
//!   bounds our memory instead of `OOMing`. Draining continues past the cap so the
//!   writer never blocks; the deadline still bounds total wall-clock.

use std::io::Read;
use std::process::{Command, ExitStatus, Stdio};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::supervise::pid_file;

/// How often the wait loop polls the child while counting down to the deadline.
const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// One captured output stream, retained up to a cap.
#[derive(Debug, Clone, Default)]
pub struct CappedStream {
    /// The captured bytes — at most `cap` of them.
    pub bytes: Vec<u8>,
    /// `true` if the child produced more than `cap` bytes and the tail was
    /// dropped. Callers should warn and treat the bytes as a prefix only.
    pub truncated: bool,
}

impl CappedStream {
    fn empty() -> Self {
        Self::default()
    }
}

/// Outcome of a bounded subprocess run.
pub enum TimedOutcome {
    /// The child exited on its own (zero or non-zero); both streams captured
    /// (each capped).
    Exited {
        status: ExitStatus,
        stdout: CappedStream,
        stderr: CappedStream,
    },
    /// The deadline was exceeded; the child's process group was `SIGKILLed`.
    TimedOut,
    /// The child could not be spawned (e.g. binary not on `PATH`) or an
    /// unexpected `wait` error occurred.
    SpawnErr(std::io::Error),
}

/// Spawn `cmd` and wait at most `timeout`, capturing stdout and stderr each up
/// to `cap` bytes. `stdin`/`stdout`/`stderr` and the process group are set here
/// authoritatively — the caller only supplies the program, args, env, and
/// `current_dir`.
pub fn run_with_timeout(mut cmd: Command, timeout: Duration, cap: usize) -> TimedOutcome {
    use std::os::unix::process::CommandExt;

    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    // Own process group so a timeout can reap the whole tree, not just the
    // direct child (which may be a shell that forked the real binary).
    cmd.process_group(0);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return TimedOutcome::SpawnErr(e),
    };
    let pid = child.id();

    // Drain each pipe on its own thread so a child that fills one pipe cannot
    // dead-lock waiting for us to read the other.
    let mut out_reader: Option<JoinHandle<CappedStream>> = child
        .stdout
        .take()
        .map(|s| std::thread::spawn(move || read_capped(s, cap)));
    let mut err_reader: Option<JoinHandle<CappedStream>> = child
        .stderr
        .take()
        .map(|s| std::thread::spawn(move || read_capped(s, cap)));

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if Instant::now() >= deadline {
                    if let Some(pgid) = pid_file::to_pid_t(pid) {
                        // SAFETY: SIGKILL to `-pgid` signals our own freshly
                        // spawned process group (pgid == child pid via
                        // process_group(0)); `pgid` is range-checked by
                        // `to_pid_t`, so the negation can never be `-1`
                        // (broadcast). Killing the group releases the pipes so
                        // the reader threads below unblock.
                        unsafe { libc::kill(-pgid, libc::SIGKILL) };
                    }
                    let _ = child.wait();
                    join_reader(out_reader.take());
                    join_reader(err_reader.take());
                    return TimedOutcome::TimedOut;
                }
                std::thread::sleep(POLL_INTERVAL);
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                join_reader(out_reader.take());
                join_reader(err_reader.take());
                return TimedOutcome::SpawnErr(e);
            }
        }
    };

    // The child exited; join the readers to collect what they drained. The pipes
    // are closed now, so neither join blocks.
    let stdout = join_reader(out_reader.take());
    let stderr = join_reader(err_reader.take());
    TimedOutcome::Exited {
        status,
        stdout,
        stderr,
    }
}

/// Read `r` to EOF but retain only the first `cap` bytes, flagging truncation.
/// Reading continues past the cap (discarding) so the writer never blocks on a
/// full pipe — the caller's deadline bounds a truly endless producer.
fn read_capped(mut r: impl Read, cap: usize) -> CappedStream {
    let mut bytes = Vec::new();
    let mut truncated = false;
    let mut chunk = [0u8; 8192];
    loop {
        match r.read(&mut chunk) {
            Ok(n) if n > 0 => {
                if bytes.len() < cap {
                    let take = (cap - bytes.len()).min(n);
                    bytes.extend_from_slice(&chunk[..take]);
                    if take < n {
                        truncated = true;
                    }
                } else {
                    truncated = true;
                }
            }
            // EOF (`Ok(0)`) or a read error mid-stream (e.g. the pipe torn down
            // by a group kill) ends capture with whatever we have so far.
            Ok(_) | Err(_) => break,
        }
    }
    CappedStream { bytes, truncated }
}

/// Join a reader thread, defaulting to an empty stream if it was absent or the
/// thread panicked (a panicked drain is only a lost capture, never fatal).
fn join_reader(reader: Option<JoinHandle<CappedStream>>) -> CappedStream {
    reader
        .and_then(|h| h.join().ok())
        .unwrap_or_else(CappedStream::empty)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `/bin/sh -c <script>` as a `Command`, ready for [`run_with_timeout`].
    fn sh(script: &str) -> Command {
        let mut c = Command::new("/bin/sh");
        c.arg("-c").arg(script);
        c
    }

    #[test]
    fn captures_stdout_and_exit_status() {
        let out = run_with_timeout(sh("printf hello; exit 0"), Duration::from_secs(5), 1 << 20);
        match out {
            TimedOutcome::Exited {
                status,
                stdout,
                stderr,
            } => {
                assert!(status.success());
                assert_eq!(stdout.bytes, b"hello");
                assert!(!stdout.truncated);
                assert!(stderr.bytes.is_empty());
            }
            _ => panic!("expected Exited"),
        }
    }

    #[test]
    fn captures_stderr_and_nonzero_status() {
        let out = run_with_timeout(
            sh("printf oops 1>&2; exit 7"),
            Duration::from_secs(5),
            1 << 20,
        );
        match out {
            TimedOutcome::Exited { status, stderr, .. } => {
                assert_eq!(status.code(), Some(7));
                assert_eq!(stderr.bytes, b"oops");
            }
            _ => panic!("expected Exited"),
        }
    }

    #[test]
    fn caps_oversized_output_and_flags_truncation() {
        // Produce ~64 KiB but cap at 1 KiB.
        let out = run_with_timeout(
            sh("yes AAAAAAAA | head -c 65536"),
            Duration::from_secs(10),
            1024,
        );
        match out {
            TimedOutcome::Exited { stdout, .. } => {
                assert_eq!(stdout.bytes.len(), 1024, "retained exactly the cap");
                assert!(stdout.truncated, "overflow must flag truncation");
            }
            _ => panic!("expected Exited"),
        }
    }

    #[test]
    fn kills_group_on_timeout() {
        let start = Instant::now();
        // A child that would sleep far past the deadline; also forks a grandchild
        // so the group-kill path is exercised.
        let out = run_with_timeout(
            sh("sleep 30 & sleep 30"),
            Duration::from_millis(200),
            1 << 20,
        );
        assert!(matches!(out, TimedOutcome::TimedOut));
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "timeout must fire promptly, not wait for the child"
        );
    }

    #[test]
    fn missing_binary_is_spawn_err() {
        let out = run_with_timeout(
            Command::new("/nonexistent/orchestratectl-no-such-binary"),
            Duration::from_secs(5),
            1 << 20,
        );
        match out {
            TimedOutcome::SpawnErr(e) => {
                assert_eq!(e.kind(), std::io::ErrorKind::NotFound);
            }
            _ => panic!("expected SpawnErr"),
        }
    }
}
