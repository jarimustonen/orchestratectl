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

/// Why a controlled run was stopped before the child exited on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// The wall-clock deadline was exceeded.
    Timeout,
    /// The caller-supplied cancel predicate returned `true` (a supervisor
    /// circuit-breaker tripped, design §9).
    Cancelled,
}

/// Outcome of a run bounded by an optional deadline **and** a cancel predicate
/// ([`run_with_control`]). Unlike [`TimedOutcome`], a `Stopped` run still returns
/// whatever output was drained before the group kill, so callers can persist a
/// partial transcript (design §10: a timed-out/cancelled chunk reports its
/// partial transcript).
pub enum ControlledOutcome {
    /// The child exited on its own (zero or non-zero); both streams captured.
    Exited {
        status: ExitStatus,
        stdout: CappedStream,
        stderr: CappedStream,
    },
    /// The run was stopped early (deadline or cancel); the process group was
    /// `SIGKILLed` and the partial drained output is returned.
    Stopped {
        reason: StopReason,
        stdout: CappedStream,
        stderr: CappedStream,
    },
    /// The child could not be spawned or an unexpected `wait` error occurred.
    SpawnErr(std::io::Error),
}

/// Spawn `cmd` and wait at most `timeout`, capturing stdout and stderr each up
/// to `cap` bytes. `stdin`/`stdout`/`stderr` and the process group are set here
/// authoritatively — the caller only supplies the program, args, env, and
/// `current_dir`.
pub fn run_with_timeout(cmd: Command, timeout: Duration, cap: usize) -> TimedOutcome {
    // A never-cancel predicate, so the only early stop is the deadline.
    match run_with_control(cmd, Some(timeout), &|| false, cap) {
        ControlledOutcome::Exited {
            status,
            stdout,
            stderr,
        } => TimedOutcome::Exited {
            status,
            stdout,
            stderr,
        },
        // With a never-true cancel predicate the only `Stopped` reason is the
        // deadline; a `Cancelled` reason is unreachable here.
        ControlledOutcome::Stopped { .. } => TimedOutcome::TimedOut,
        ControlledOutcome::SpawnErr(e) => TimedOutcome::SpawnErr(e),
    }
}

/// Spawn `cmd` and wait until it exits, the optional `timeout` deadline passes,
/// or `cancel()` returns `true` — whichever comes first. On an early stop the
/// child's process *group* is `SIGKILLed` (reaping any subprocess it forked) and
/// the partial drained output is returned in [`ControlledOutcome::Stopped`].
///
/// `timeout` of `None` means "no wall-clock ceiling" — the run is then bounded
/// only by `cancel()`. Cancellation is checked before the deadline each poll, so
/// a cancel that races a deadline is reported as [`StopReason::Cancelled`]. The
/// cancel predicate is polled every [`POLL_INTERVAL`]; latency is bounded by it.
pub fn run_with_control(
    mut cmd: Command,
    timeout: Option<Duration>,
    cancel: &dyn Fn() -> bool,
    cap: usize,
) -> ControlledOutcome {
    use std::os::unix::process::CommandExt;

    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    // Own process group so an early stop can reap the whole tree, not just the
    // direct child (which may be a shell that forked the real binary).
    cmd.process_group(0);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return ControlledOutcome::SpawnErr(e),
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

    let deadline = timeout.map(|t| Instant::now() + t);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                // Cancellation is checked before the deadline so a cancel that
                // races the timeout is reported as `Cancelled` (more actionable).
                let reason = if cancel() {
                    Some(StopReason::Cancelled)
                } else if deadline.is_some_and(|d| Instant::now() >= d) {
                    Some(StopReason::Timeout)
                } else {
                    None
                };
                if let Some(reason) = reason {
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
                    let stdout = join_reader(out_reader.take());
                    let stderr = join_reader(err_reader.take());
                    return ControlledOutcome::Stopped {
                        reason,
                        stdout,
                        stderr,
                    };
                }
                std::thread::sleep(POLL_INTERVAL);
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                join_reader(out_reader.take());
                join_reader(err_reader.take());
                return ControlledOutcome::SpawnErr(e);
            }
        }
    };

    // The child exited; join the readers to collect what they drained. The pipes
    // are closed now, so neither join blocks.
    let stdout = join_reader(out_reader.take());
    let stderr = join_reader(err_reader.take());
    ControlledOutcome::Exited {
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
    fn control_cancel_stops_promptly_with_partial_output() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        let flag = Arc::new(AtomicBool::new(false));
        let trip = Arc::clone(&flag);
        // Trip the cancel shortly after the run starts.
        let handle = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            trip.store(true, Ordering::SeqCst);
        });

        let start = Instant::now();
        // No deadline — the run is bounded only by the cancel. It emits some
        // output first so we can assert the partial drain.
        let out = run_with_control(
            sh("printf partial; sleep 30"),
            None,
            &move || flag.load(Ordering::SeqCst),
            1 << 20,
        );
        handle.join().unwrap();
        match out {
            ControlledOutcome::Stopped {
                reason,
                stdout,
                stderr,
            } => {
                assert_eq!(reason, StopReason::Cancelled);
                assert_eq!(stdout.bytes, b"partial");
                assert!(stderr.bytes.is_empty());
            }
            _ => panic!("expected Stopped(Cancelled)"),
        }
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "cancel must fire promptly, not wait for the child"
        );
    }

    #[test]
    fn control_deadline_stops_when_never_cancelled() {
        let start = Instant::now();
        let out = run_with_control(
            sh("sleep 30"),
            Some(Duration::from_millis(200)),
            &|| false,
            1 << 20,
        );
        assert!(matches!(
            out,
            ControlledOutcome::Stopped {
                reason: StopReason::Timeout,
                ..
            }
        ));
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn control_exits_normally_when_neither_fires() {
        let out = run_with_control(sh("printf ok; exit 0"), None, &|| false, 1 << 20);
        match out {
            ControlledOutcome::Exited { status, stdout, .. } => {
                assert!(status.success());
                assert_eq!(stdout.bytes, b"ok");
            }
            _ => panic!("expected Exited"),
        }
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
