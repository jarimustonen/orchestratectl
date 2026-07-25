//! Daemonization **readiness pipe** for confirming a detached supervisor booted.
//!
//! `run create` must learn — without an arbitrary timeout — whether the
//! supervisor it double-forks (see [`crate::run::supervisor_spawn`]) actually
//! came up. The old mechanism polled `supervisor.pid` for a bounded window
//! (`PID_FILE_WAIT`); a healthy-but-slow boot under load could exceed the
//! deadline and be false-failed into `supervisor_spawn_failed` while the
//! grandchild kept running — an orphan supervising a run the caller was told
//! failed (issue `supervisor-confirm-readiness-pipe`, review finding F4b).
//!
//! The fix is the classic UNIX readiness pipe threaded through the double-fork:
//!
//! 1. The parent ([`ReadinessPipe::new`]) creates a pipe. The **write** end is
//!    left without `FD_CLOEXEC` so it survives `exec` into the grandchild; the
//!    **read** end is `FD_CLOEXEC` (parent-only, must not leak into the child).
//!    The parent passes the write-end fd number to the child via
//!    `OCTL_READINESS_FD` in the child's environment.
//! 2. The grandchild (the real supervisor) takes ownership of that inherited fd
//!    ([`ReadinessReporter::from_env`]) and, **after** `claim_pid_atomic` +
//!    boot init, writes a one-line readiness message and closes the fd
//!    ([`ReadinessReporter::ready`] / [`ReadinessReporter::error`]).
//! 3. The parent closes its own copy of the write end
//!    ([`ReadinessPipe::close_write`]) — so it is not itself a writer — and
//!    blocks in [`ReadinessPipe::await_ready`] reading to EOF:
//!    - a `ready` line → the supervisor confirmed boot (carries its pid);
//!    - EOF with no message → every write-end copy closed without a signal, so
//!      the supervisor **died during init** (fate-sharing);
//!    - an `error` line → the supervisor articulated a real boot failure;
//!    - a truncated/garbled message → treated as a boot failure, never a hang.
//!
//! Fate-sharing is exact: `read()` returns EOF only once **all** write-end
//! copies (parent, double-fork intermediate, grandchild) are closed. The parent
//! must therefore [`close_write`](ReadinessPipe::close_write) before reading, and
//! the grandchild closes its copy as soon as it has written (or when it exits
//! without writing). There is no timeout: a slow boot simply keeps the pipe open
//! until the supervisor signals, and a dead one closes it.
//!
//! # Wire format
//!
//! One line, tag byte first, so the parser is robust to a partial write (a
//! writer killed mid-message yields a short read that still decodes or degrades
//! to `Malformed` — never a hang):
//!   - ready:  `R<pid>\n`               (`pid` decimal, the supervisor's own pid)
//!   - error:  `E<code>\t<message>\n`   (a structured boot-failure reason)

use std::io::Read as _;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

/// Environment variable carrying the inherited write-end fd number from the
/// parent ([`ReadinessPipe`]) to the grandchild supervisor
/// ([`ReadinessReporter`]). Set by `run create`'s confirmation path only; the
/// lenient spawn paths (`run reattach`, child-spawn) clear it in
/// [`crate::run::supervisor_spawn::detached_supervise_command`] so a supervisor
/// they launch never tries to write to a fd it did not inherit.
pub const ENV_READINESS_FD: &str = "OCTL_READINESS_FD";

/// Upper bound on the readiness message the parent will read, so a misbehaving
/// or corrupt writer cannot make `run create` read unboundedly. The real
/// messages are a few dozen bytes; this is generous headroom.
const MAX_MSG: usize = 4096;

const TAG_READY: u8 = b'R';
const TAG_ERROR: u8 = b'E';

/// The parent's decode of what the grandchild reported down the pipe.
#[derive(Debug, PartialEq, Eq)]
pub enum Readiness {
    /// The supervisor confirmed boot after claiming the pid file. Carries the
    /// supervisor's own pid (identical to the value it wrote into
    /// `supervisor.pid`).
    Ready { pid: u32 },
    /// EOF with no message: every write-end copy closed without a readiness
    /// signal, so the supervisor died during init (fate-sharing).
    Died,
    /// The supervisor articulated a specific boot failure before exiting.
    Error { code: String, message: String },
    /// A non-empty but undecodable message — e.g. a write truncated by a crash
    /// mid-message. Surfaced as a boot failure, carrying the raw bytes (lossily)
    /// for diagnostics.
    Malformed(String),
}

/// Parse a readiness message. Pure and total so the three named cases
/// (readiness success, init-failure EOF, partial write) are unit-testable
/// without real pipes. Tolerant by construction: it never panics and always
/// classifies.
pub fn parse_readiness(bytes: &[u8]) -> Readiness {
    if bytes.is_empty() {
        return Readiness::Died;
    }
    // Strip a single trailing newline if present; tolerate its absence (a write
    // truncated before the terminator).
    let body = bytes.strip_suffix(b"\n").unwrap_or(bytes);
    match body.first().copied() {
        Some(TAG_READY) => {
            let digits: Vec<u8> = body[1..]
                .iter()
                .copied()
                .take_while(u8::is_ascii_digit)
                .collect();
            match std::str::from_utf8(&digits)
                .ok()
                .and_then(|s| s.parse::<u32>().ok())
            {
                Some(pid) if pid != 0 => Readiness::Ready { pid },
                // Tag arrived but the pid did not (truncated write) or was 0.
                _ => Readiness::Malformed(String::from_utf8_lossy(bytes).into_owned()),
            }
        }
        Some(TAG_ERROR) => {
            let rest = &body[1..];
            let (code, message) = match rest.iter().position(|&b| b == b'\t') {
                Some(i) => (
                    String::from_utf8_lossy(&rest[..i]).into_owned(),
                    String::from_utf8_lossy(&rest[i + 1..]).into_owned(),
                ),
                None => (String::from_utf8_lossy(rest).into_owned(), String::new()),
            };
            Readiness::Error { code, message }
        }
        _ => Readiness::Malformed(String::from_utf8_lossy(bytes).into_owned()),
    }
}

/// Set or clear `FD_CLOEXEC` on `fd`.
fn set_cloexec(fd: RawFd, on: bool) -> std::io::Result<()> {
    // SAFETY: `fcntl` with `F_GETFD`/`F_SETFD` only reads/writes the fd's
    // close-on-exec flag; it does not consume or alias the fd.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let new = if on {
        flags | libc::FD_CLOEXEC
    } else {
        flags & !libc::FD_CLOEXEC
    };
    if unsafe { libc::fcntl(fd, libc::F_SETFD, new) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Parent side of the readiness pipe. Owns both ends; the read end is used to
/// [`await_ready`](Self::await_ready) and the write end is handed (by fd number)
/// to the child then dropped via [`close_write`](Self::close_write).
pub struct ReadinessPipe {
    read: OwnedFd,
    write: Option<OwnedFd>,
}

impl ReadinessPipe {
    /// Create the pipe. macOS lacks `pipe2`, so we `pipe()` and set the flags
    /// with `fcntl`: the read end is `FD_CLOEXEC` (never inherited by the child)
    /// and the write end is left without it so it survives `exec` into the
    /// grandchild.
    pub fn new() -> std::io::Result<Self> {
        let mut fds = [0 as RawFd; 2];
        // SAFETY: `pipe` writes exactly two fds into the provided array and
        // returns 0 on success; we check the return before wrapping the fds.
        if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: `pipe` just handed us two fresh, owned fds.
        let read = unsafe { OwnedFd::from_raw_fd(fds[0]) };
        let write = unsafe { OwnedFd::from_raw_fd(fds[1]) };
        set_cloexec(read.as_raw_fd(), true)?;
        // The write end intentionally keeps CLOEXEC OFF so the grandchild
        // inherits it across exec; `pipe()` fds default to CLOEXEC off.
        Ok(Self {
            read,
            write: Some(write),
        })
    }

    /// The write-end fd number to advertise to the child via
    /// [`ENV_READINESS_FD`]. Valid until [`close_write`](Self::close_write).
    pub fn write_fd(&self) -> RawFd {
        self.write
            .as_ref()
            .expect("write end still held")
            .as_raw_fd()
    }

    /// Drop the parent's copy of the write end. The parent must do this after
    /// the spawn so it is not itself a writer keeping the pipe open — otherwise
    /// [`await_ready`](Self::await_ready) would never observe EOF on the
    /// grandchild's death.
    pub fn close_write(&mut self) {
        self.write = None;
    }

    /// Block reading the read end to EOF and classify the message. Consumes the
    /// pipe (the read end closes on return). No timeout: returns as soon as the
    /// grandchild signals, or as soon as every write-end copy is closed.
    pub fn await_ready(self) -> Readiness {
        // The parent must have already closed its own write end.
        debug_assert!(self.write.is_none(), "close_write() before await_ready()");
        let mut file = std::fs::File::from(self.read);
        let mut buf = Vec::new();
        // Bounded read: a healthy writer sends a short line then closes (EOF).
        if let Err(e) = (&mut file).take(MAX_MSG as u64).read_to_end(&mut buf) {
            return Readiness::Malformed(format!("readiness read failed: {e}"));
        }
        parse_readiness(&buf)
    }
}

/// Grandchild (supervisor) side. Holds the inherited write-end fd until it
/// reports readiness or a boot error, then closes it. If dropped without
/// reporting, the fd closes anyway and the parent sees EOF (`Died`).
pub struct ReadinessReporter {
    fd: Option<OwnedFd>,
}

impl ReadinessReporter {
    /// Take ownership of the inherited write-end fd named by
    /// [`ENV_READINESS_FD`]. Returns a no-op reporter when the variable is unset
    /// (the lenient spawn paths) or unparseable.
    pub fn from_env() -> Self {
        let fd = std::env::var(ENV_READINESS_FD)
            .ok()
            .and_then(|v| v.trim().parse::<RawFd>().ok())
            .filter(|&fd| fd >= 0)
            // SAFETY: the parent passed us this write-end fd, inherited across
            // exec (CLOEXEC cleared). We take sole ownership so it is closed
            // exactly once, on report or drop.
            .map(|raw| unsafe { OwnedFd::from_raw_fd(raw) });
        Self { fd }
    }

    /// Report a confirmed boot carrying the supervisor's own pid, then close the
    /// write end so the parent's `read()` returns. Idempotent-ish: only the
    /// first `ready`/`error` writes; later calls are no-ops.
    pub fn ready(&mut self, pid: u32) {
        self.emit(format!("{}{pid}\n", TAG_READY as char));
    }

    /// Report a structured boot failure the parent will surface as the real
    /// reason, then close the write end. Newlines/tabs in `message` are
    /// flattened so the single-line wire format holds.
    pub fn error(&mut self, code: &str, message: &str) {
        let flat = message.replace(['\n', '\t'], " ");
        self.emit(format!("{}{code}\t{flat}\n", TAG_ERROR as char));
    }

    fn emit(&mut self, msg: String) {
        if let Some(fd) = self.fd.take() {
            write_all(fd.as_raw_fd(), msg.as_bytes());
            // `fd` drops here → the write end closes → the parent unblocks.
        }
    }
}

/// Write the whole buffer, retrying short writes and `EINTR`. Best-effort: if
/// the parent has already gone away (`EPIPE`) or the fd is otherwise unwritable
/// there is nothing useful to do — the parent will see EOF and treat the boot as
/// failed. Kept as a raw `write` loop (not `File::write_all`) so the partial-
/// write retry is explicit and self-contained.
fn write_all(fd: RawFd, mut buf: &[u8]) {
    while !buf.is_empty() {
        // SAFETY: `write` reads `buf.len()` bytes from `buf` and does not retain
        // the pointer; `fd` is owned by the caller for the duration.
        let n = unsafe { libc::write(fd, buf.as_ptr().cast::<libc::c_void>(), buf.len()) };
        if n < 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return;
        }
        if n == 0 {
            return;
        }
        buf = &buf[n as usize..];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ready_line() {
        assert_eq!(
            parse_readiness(b"R12345\n"),
            Readiness::Ready { pid: 12345 }
        );
    }

    #[test]
    fn parse_ready_without_trailing_newline() {
        // A write truncated before the terminator but with a full pid still
        // decodes — the pid is valid.
        assert_eq!(parse_readiness(b"R777"), Readiness::Ready { pid: 777 });
    }

    #[test]
    fn parse_empty_is_died() {
        assert_eq!(parse_readiness(b""), Readiness::Died);
    }

    #[test]
    fn parse_error_line() {
        assert_eq!(
            parse_readiness(b"Esupervisor_already_running\tpid 42 is alive\n"),
            Readiness::Error {
                code: "supervisor_already_running".into(),
                message: "pid 42 is alive".into(),
            }
        );
    }

    #[test]
    fn parse_error_without_message() {
        assert_eq!(
            parse_readiness(b"Elock_error\n"),
            Readiness::Error {
                code: "lock_error".into(),
                message: String::new(),
            }
        );
    }

    #[test]
    fn parse_partial_ready_tag_only_is_malformed() {
        // Writer crashed after the tag byte but before any pid digit.
        match parse_readiness(b"R") {
            Readiness::Malformed(_) => {}
            other => panic!("expected Malformed, got {other:?}"),
        }
    }

    #[test]
    fn parse_ready_zero_pid_is_malformed() {
        match parse_readiness(b"R0\n") {
            Readiness::Malformed(_) => {}
            other => panic!("expected Malformed, got {other:?}"),
        }
    }

    #[test]
    fn parse_garbage_is_malformed() {
        match parse_readiness(b"xyzzy") {
            Readiness::Malformed(_) => {}
            other => panic!("expected Malformed, got {other:?}"),
        }
    }

    /// Round-trip over a real pipe: reporter writes ready, parent reads it.
    #[test]
    fn pipe_roundtrip_ready() {
        let mut pipe = ReadinessPipe::new().unwrap();
        let write_fd = pipe.write_fd();
        // Simulate the grandchild taking ownership of the inherited write fd.
        // SAFETY: we hand off the raw write fd exactly once; `pipe.close_write`
        // below drops the parent's OwnedFd wrapper WITHOUT closing this fd
        // number, since ownership has moved into the reporter.
        let reporter_fd = unsafe { OwnedFd::from_raw_fd(dup(write_fd)) };
        let mut reporter = ReadinessReporter {
            fd: Some(reporter_fd),
        };
        pipe.close_write();
        reporter.ready(4242);
        assert_eq!(pipe.await_ready(), Readiness::Ready { pid: 4242 });
    }

    /// Round-trip: the reporter is dropped without reporting → the parent sees
    /// EOF and classifies it as a death during init.
    #[test]
    fn pipe_roundtrip_died_on_drop() {
        let mut pipe = ReadinessPipe::new().unwrap();
        let write_fd = pipe.write_fd();
        // SAFETY: as above — a single dup'd owned copy handed to the reporter.
        let reporter_fd = unsafe { OwnedFd::from_raw_fd(dup(write_fd)) };
        let reporter = ReadinessReporter {
            fd: Some(reporter_fd),
        };
        pipe.close_write();
        drop(reporter); // dies without signalling
        assert_eq!(pipe.await_ready(), Readiness::Died);
    }

    /// Round-trip: the reporter articulates a structured error.
    #[test]
    fn pipe_roundtrip_error() {
        let mut pipe = ReadinessPipe::new().unwrap();
        let write_fd = pipe.write_fd();
        // SAFETY: single dup'd owned copy handed to the reporter.
        let reporter_fd = unsafe { OwnedFd::from_raw_fd(dup(write_fd)) };
        let mut reporter = ReadinessReporter {
            fd: Some(reporter_fd),
        };
        pipe.close_write();
        reporter.error("supervisor_already_running", "pid 9 is alive");
        assert_eq!(
            pipe.await_ready(),
            Readiness::Error {
                code: "supervisor_already_running".into(),
                message: "pid 9 is alive".into(),
            }
        );
    }

    /// The reporter closes the fd after `ready`, so the parent both reads the
    /// message AND observes EOF — it never blocks waiting for more.
    #[test]
    fn reporter_closes_after_ready() {
        let mut pipe = ReadinessPipe::new().unwrap();
        // SAFETY: single dup'd owned copy handed to the reporter.
        let reporter_fd = unsafe { OwnedFd::from_raw_fd(dup(pipe.write_fd())) };
        let mut reporter = ReadinessReporter {
            fd: Some(reporter_fd),
        };
        pipe.close_write();
        reporter.ready(5);
        // Second call is a no-op (fd already taken); no panic, no double close.
        reporter.ready(6);
        assert_eq!(pipe.await_ready(), Readiness::Ready { pid: 5 });
    }

    #[test]
    fn from_env_absent_is_noop() {
        // With the variable unset, the reporter holds no fd and reporting does
        // nothing (the lenient spawn paths rely on this).
        std::env::remove_var(ENV_READINESS_FD);
        let mut reporter = ReadinessReporter::from_env();
        assert!(reporter.fd.is_none());
        reporter.ready(1); // must not panic
        reporter.error("x", "y"); // must not panic
    }

    /// `dup(2)` helper so a test can hand the reporter its own owned fd without
    /// disturbing the pipe's bookkeeping.
    fn dup(fd: RawFd) -> RawFd {
        let d = unsafe { libc::dup(fd) };
        assert!(d >= 0, "dup failed");
        d
    }
}
