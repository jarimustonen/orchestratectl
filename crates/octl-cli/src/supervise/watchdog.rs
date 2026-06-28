//! Agent liveness watchdog (design.md §7.5).
//!
//! Dual polling: PID liveness via `kill(pid, 0)` + start-time identity
//! defense (so a recycled PID is not mistaken for the original agent) +
//! tmux window presence via `tmux list-windows`.
//!
//! Start-time is queried via the cross-platform `sysinfo` crate. The
//! crate exposes `Process::start_time()` as a Unix timestamp on every
//! supported platform; this avoids the macOS/Linux `sysctl` /
//! `/proc/<pid>/stat` split that the design originally specified. If
//! `sysinfo` ever drops `start_time` we'll switch to direct `libc`
//! probes; tracked in `validation.md`.

use std::process::Command;

use octl_core::schema::TmuxIdentity;
use sysinfo::{Pid, System};

use crate::supervise::pid_file;

/// Liveness verdict for an agent process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Liveness {
    Alive,
    /// PID has exited (or never existed).
    Dead,
    /// `kill(pid, 0)` succeeded but `start_time` disagrees with the
    /// captured value — PID has been recycled by an unrelated process.
    Recycled,
    /// PID looks alive but the tmux window is gone (e.g. user
    /// `tmux kill-window`). Half-state; design.md §7.5 commits to
    /// treating this as terminal after a short retry.
    TmuxGone,
}

impl Liveness {
    pub fn reason(self) -> &'static str {
        match self {
            Liveness::Alive => "alive",
            Liveness::Dead => "agent-died",
            Liveness::Recycled => "agent-pid-recycled",
            Liveness::TmuxGone => "agent-tmux-window-gone",
        }
    }
}

/// Read the process `start_time` (Unix seconds) for `pid`. Returns `None`
/// if the process does not exist or the platform sysinfo backend
/// declines to populate the value.
pub fn pid_start_time(pid: u32) -> Option<u64> {
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::Some(&[Pid::from_u32(pid)]),
        true,
        sysinfo::ProcessRefreshKind::new(),
    );
    sys.process(Pid::from_u32(pid))
        .map(sysinfo::Process::start_time)
}

/// Outcome of a tmux window probe.
///
/// The distinction between [`Absent`](TmuxProbe::Absent) and
/// [`Unknown`](TmuxProbe::Unknown) is load-bearing for liveness: only `Absent`
/// (the server answered and the window is genuinely not there) may flip a node
/// to `TmuxGone`. `Unknown` (tmux binary missing, server down on the recorded
/// socket, spawn error — we could not get a definitive answer) must NOT, or a
/// transient/operational hiccup would falsely reap a live agent. Process
/// liveness still governs in the `Unknown` case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TmuxProbe {
    /// The server answered and the target window is present.
    Present,
    /// The server answered and the target window is definitively gone.
    Absent,
    /// Could not get a definitive answer (tmux missing, server down, error).
    Unknown,
}

/// Decide present/absent from a successful `list-windows` run: an exit-0 tmux
/// means the server answered, so a missing `needle` line is a *definitive*
/// absence. Any spawn error or non-zero exit is [`TmuxProbe::Unknown`].
fn classify_list_windows(out: std::io::Result<std::process::Output>, needle: &[u8]) -> TmuxProbe {
    let out = match out {
        Ok(o) => o,
        // tmux binary missing / spawn failure — we learned nothing.
        Err(_) => return TmuxProbe::Unknown,
    };
    if !out.status.success() {
        // No server on this socket, permission error, etc. — not a verdict.
        return TmuxProbe::Unknown;
    }
    // Trim each line so a stray `\r` (anomalous terminals) does not defeat the
    // exact match.
    if out
        .stdout
        .split(|b| *b == b'\n')
        .any(|line| trim_ascii(line) == needle)
    {
        TmuxProbe::Present
    } else {
        TmuxProbe::Absent
    }
}

fn trim_ascii(b: &[u8]) -> &[u8] {
    let start = b.iter().position(|c| !c.is_ascii_whitespace());
    let Some(start) = start else { return &[] };
    let end = b.iter().rposition(|c| !c.is_ascii_whitespace()).unwrap();
    &b[start..=end]
}

/// Probe for a window by its bare *name* on the default socket
/// (`tmux list-windows -a -F '#{window_name}'`). This is the legacy,
/// ambiguous match — names are not unique across sessions — kept only for nodes
/// registered before create.sh emitted the qualified identity.
///
/// Callers may override the tmux invocation via `TMUX_BIN` (mostly for tests).
pub fn probe_window_by_name(window_name: &str) -> TmuxProbe {
    let bin = std::env::var("TMUX_BIN").unwrap_or_else(|_| "tmux".to_string());
    let out = Command::new(bin)
        .args(["list-windows", "-a", "-F", "#{window_name}"])
        .output();
    classify_list_windows(out, window_name.as_bytes())
}

/// Probe for the *exact* window named by a [`TmuxIdentity`] on its recorded
/// server: `tmux [-S <socket>] list-windows -a -F '#{window_id}'`, matching the
/// stable `window_id` line-exactly.
///
/// `window_id` (`@NNNN`) is unique within a tmux *server*, so we deliberately
/// list ALL windows on the socket (`-a`) rather than scoping to the recorded
/// session. That makes the match immune to `rename-session` and to a window
/// being linked/moved between sessions — failure modes a `-t <session>` scope
/// would turn into false absences. The recorded `session` is retained for human
/// display only; it is not part of the match.
///
/// Returns [`TmuxProbe::Unknown`] when the server cannot be reached (binary
/// missing, server down on the recorded socket, error) — see [`TmuxProbe`].
/// Honors the `TMUX_BIN` override for tests.
pub fn probe_window_qualified(identity: &TmuxIdentity) -> TmuxProbe {
    let bin = std::env::var("TMUX_BIN").unwrap_or_else(|_| "tmux".to_string());
    let mut cmd = Command::new(bin);
    if let Some(socket) = identity.socket.as_deref() {
        cmd.args(["-S", socket]);
    }
    cmd.args(["list-windows", "-a", "-F", "#{window_id}"]);
    classify_list_windows(cmd.output(), identity.window_id.as_bytes())
}

/// Snapshot of what we know about an agent at spawn time. Compared
/// against live probes by [`check_liveness`].
#[derive(Debug, Clone)]
pub struct AgentProbe {
    pub pid: u32,
    /// Start-time captured at spawn (Unix seconds). `None` means the
    /// supervisor could not read `start_time` on this platform — we then
    /// fall back to PID-only liveness and accept the tiny risk of a
    /// reused PID being mistaken for the agent.
    pub start_time: Option<u64>,
    pub tmux_window: Option<String>,
    /// Fully-qualified tmux identity captured at spawn. When `Some`, the tmux
    /// probe matches the stable `window_id` on the recorded socket (precise);
    /// when `None` (a node registered before create.sh emitted the qualified
    /// fields), it falls back to a bare-name match on [`AgentProbe::tmux_window`].
    pub tmux_identity: Option<TmuxIdentity>,
    /// When `true`, skip the tmux probe. Used when tmux is not the
    /// host (e.g. fake-spawn test fixtures) or tmux unavailability is
    /// not a failure signal.
    pub skip_tmux_check: bool,
}

/// Cap on distinct window names retained for warn-once dedup. Bounds the
/// `WARNED` set for a long-running supervisor: once the cap is hit we stop
/// tracking new names (and warn once that we're capping), accepting that a few
/// later legacy windows may re-warn rather than letting the set grow unbounded.
const MAX_LEGACY_WARN_KEYS: usize = 1024;

/// Warn — at most once per window name per process — that a node lacks a
/// qualified tmux identity and is using the ambiguous bare-name fallback. The
/// watchdog probes every tick, so an un-deduplicated `warn!` would flood the
/// log for every legacy node; this surfaces the condition once and stays quiet
/// thereafter. New spawns get the loud warning at the spawn boundary instead
/// (`run::spawn::run_create_sh`).
fn warn_legacy_bare_name_once(window_name: &str) {
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock};
    static WARNED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let mutex = WARNED.get_or_init(|| Mutex::new(HashSet::new()));
    // A poisoned mutex must not abort liveness checks — this is only log dedup,
    // so recover the guard and carry on. Decide-then-release: never hold the
    // lock across the `tracing::warn!` emit.
    let mut guard = mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let cap_hit = guard.len() >= MAX_LEGACY_WARN_KEYS;
    let is_new = !cap_hit && guard.insert(window_name.to_string());
    drop(guard);
    if is_new {
        tracing::warn!(
            tmux_window = window_name,
            "node has no qualified tmux identity; falling back to bare \
             window-name liveness matching (registered before create.sh \
             emitted the qualified fields) — this is ambiguous across sessions"
        );
    }
}

/// Probe the agent and return a [`Liveness`] verdict.
pub fn check_liveness(probe: &AgentProbe) -> Liveness {
    if !pid_file::pid_alive(probe.pid) {
        return Liveness::Dead;
    }
    if let Some(expected) = probe.start_time {
        if let Some(actual) = pid_start_time(probe.pid) {
            // Allow a 1-second tolerance: some platforms round
            // start_time to whole seconds while sysinfo's first probe
            // may report fractional ticks differently than the second.
            if expected.abs_diff(actual) > 1 {
                return Liveness::Recycled;
            }
        }
    }
    if !probe.skip_tmux_check {
        let probe_result = match probe.tmux_identity.as_ref() {
            // Precise path: match the stable window_id on the recorded socket.
            Some(identity) => Some(probe_window_qualified(identity)),
            // Legacy path: node registered before create.sh emitted the
            // qualified fields. Fall back to the ambiguous bare-name match.
            None => probe.tmux_window.as_deref().map(|name| {
                warn_legacy_bare_name_once(name);
                probe_window_by_name(name)
            }),
        };
        // Only a *definitive* absence (server answered, window not there) flips
        // the node to TmuxGone. `Unknown` (server unreachable / tmux missing)
        // and "nothing to probe" leave liveness to the PID check above — a
        // transient or operational tmux failure must not reap a live agent.
        match probe_result {
            Some(TmuxProbe::Absent) => return Liveness::TmuxGone,
            Some(TmuxProbe::Unknown) => {
                tracing::debug!(
                    "tmux liveness probe inconclusive (server unreachable or \
                     tmux unavailable); deferring to PID liveness"
                );
            }
            Some(TmuxProbe::Present) | None => {}
        }
    }
    Liveness::Alive
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_pid_has_start_time_and_is_alive() {
        let pid = std::process::id();
        let st = pid_start_time(pid).expect("self start_time");
        let probe = AgentProbe {
            pid,
            start_time: Some(st),
            tmux_window: None,
            tmux_identity: None,
            skip_tmux_check: true,
        };
        assert_eq!(check_liveness(&probe), Liveness::Alive);
    }

    #[test]
    fn dead_pid_is_dead() {
        // PID 0 is never a real process; treat as dead.
        let probe = AgentProbe {
            pid: 0,
            start_time: None,
            tmux_window: None,
            tmux_identity: None,
            skip_tmux_check: true,
        };
        assert_eq!(check_liveness(&probe), Liveness::Dead);
    }

    #[test]
    fn mismatched_start_time_detects_recycled_pid() {
        let pid = std::process::id();
        let probe = AgentProbe {
            pid,
            start_time: Some(1), // 1970-ish: cannot match a live process
            tmux_window: None,
            tmux_identity: None,
            skip_tmux_check: true,
        };
        assert_eq!(check_liveness(&probe), Liveness::Recycled);
    }

    #[test]
    fn start_time_is_stable_across_reads() {
        let pid = std::process::id();
        let a = pid_start_time(pid).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
        let b = pid_start_time(pid).unwrap();
        assert!(a.abs_diff(b) <= 1, "start_time drifted: {a} vs {b}");
    }

    // ---- Qualified-identity tmux matching --------------------------------
    // These tests mock `tmux` via TMUX_BIN. The var is process-global, so
    // serialize them behind a lock against a stale fixture from another thread.
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    static TMUX_BIN_LOCK: Mutex<()> = Mutex::new(());

    /// RAII guard for a process-global env var: restores the prior value (or
    /// unsets) on drop, so a panicking assertion cannot leak `TMUX_BIN` into
    /// another test.
    struct EnvGuard {
        key: &'static str,
        old: Option<std::ffi::OsString>,
    }
    impl EnvGuard {
        fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
            let old = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, old }
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.old {
                Some(v) => std::env::set_var(self.key, v),
                None => std::env::remove_var(self.key),
            }
        }
    }

    /// Write an executable fake `tmux` that records its argv (one arg per line)
    /// to `<dir>/args` and replays the lines in `<dir>/stdout`, exiting `code`.
    /// Passing a non-zero `code` simulates "server unreachable" for the
    /// `Unknown` path. `stdout_lines` is written to a file (not interpolated
    /// into the script) so arbitrary content — including quotes — is safe.
    fn fake_tmux(dir: &Path, stdout_lines: &[&str], code: i32) -> PathBuf {
        let out = dir.join("stdout");
        std::fs::write(&out, stdout_lines.join("\n")).unwrap();
        let p = dir.join("fake-tmux.sh");
        let body = format!(
            "#!/bin/bash\nprintf '%s\\n' \"$@\" > {args:?}\ncat {out:?}\nexit {code}\n",
            args = dir.join("args"),
            out = out,
            code = code,
        );
        std::fs::write(&p, body).unwrap();
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&p).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&p, perms).unwrap();
        p
    }

    fn args_lines(dir: &Path) -> Vec<String> {
        std::fs::read_to_string(dir.join("args"))
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect()
    }

    fn id(socket: Option<&str>, session: &str, window_id: &str) -> TmuxIdentity {
        TmuxIdentity {
            socket: socket.map(str::to_string),
            session: session.to_string(),
            window_id: window_id.to_string(),
        }
    }

    #[test]
    fn qualified_present_when_window_id_listed() {
        let _g = TMUX_BIN_LOCK.lock().unwrap();
        let dir = tempfile::TempDir::new().unwrap();
        // Server reports three windows; ours (@42) is among them.
        let _e = EnvGuard::set("TMUX_BIN", fake_tmux(dir.path(), &["@7", "@42", "@99"], 0));
        let r = probe_window_qualified(&id(Some("/tmp/sock"), "octl", "@42"));
        assert_eq!(r, TmuxProbe::Present);
        // The probe targeted the recorded socket and listed ALL windows (-a),
        // NOT scoped to a session (so a session rename can't break the match).
        let args = args_lines(dir.path());
        assert!(
            args.windows(2)
                .any(|w| w == ["-S".to_string(), "/tmp/sock".to_string()]),
            "socket not passed: {args:?}"
        );
        assert!(args.iter().any(|a| a == "-a"), "expected -a: {args:?}");
        assert!(
            !args.iter().any(|a| a == "-t"),
            "must not session-scope: {args:?}"
        );
    }

    #[test]
    fn qualified_absent_when_window_id_missing() {
        let _g = TMUX_BIN_LOCK.lock().unwrap();
        let dir = tempfile::TempDir::new().unwrap();
        // Same-named-but-different windows exist on the server; ours (@42) does
        // not — exactly the cross-session false-positive the bare-name match
        // made. Server answered (exit 0), so this is a *definitive* absence.
        let _e = EnvGuard::set("TMUX_BIN", fake_tmux(dir.path(), &["@7", "@99"], 0));
        let r = probe_window_qualified(&id(None, "octl", "@42"));
        assert_eq!(r, TmuxProbe::Absent);
        // No socket recorded → no `-S` flag (default socket).
        assert!(
            !args_lines(dir.path()).iter().any(|a| a == "-S"),
            "unexpected -S with null socket"
        );
    }

    #[test]
    fn qualified_unknown_when_server_unreachable() {
        let _g = TMUX_BIN_LOCK.lock().unwrap();
        let dir = tempfile::TempDir::new().unwrap();
        // Non-zero exit == "no server running on <socket>" / error. This must
        // NOT be read as absence.
        let _e = EnvGuard::set("TMUX_BIN", fake_tmux(dir.path(), &["no server running"], 1));
        let r = probe_window_qualified(&id(Some("/tmp/dead-sock"), "octl", "@42"));
        assert_eq!(r, TmuxProbe::Unknown);
    }

    #[test]
    fn check_liveness_uses_qualified_identity_for_tmux_gone() {
        let _g = TMUX_BIN_LOCK.lock().unwrap();
        let dir = tempfile::TempDir::new().unwrap();
        // Live PID (self) + start_time skipped, so the verdict turns purely on
        // the tmux probe. The fake server lists @99, not our @42 → TmuxGone.
        let _e = EnvGuard::set("TMUX_BIN", fake_tmux(dir.path(), &["@99"], 0));
        let probe = AgentProbe {
            pid: std::process::id(),
            start_time: None,
            tmux_window: Some("🚀 wt/x".to_string()),
            tmux_identity: Some(id(Some("/tmp/sock"), "octl", "@42")),
            skip_tmux_check: false,
        };
        assert_eq!(check_liveness(&probe), Liveness::TmuxGone);
    }

    #[test]
    fn check_liveness_alive_when_qualified_window_present() {
        let _g = TMUX_BIN_LOCK.lock().unwrap();
        let dir = tempfile::TempDir::new().unwrap();
        let _e = EnvGuard::set("TMUX_BIN", fake_tmux(dir.path(), &["@42"], 0));
        let probe = AgentProbe {
            pid: std::process::id(),
            start_time: None,
            tmux_window: Some("🚀 wt/x".to_string()),
            tmux_identity: Some(id(None, "octl", "@42")),
            skip_tmux_check: false,
        };
        assert_eq!(check_liveness(&probe), Liveness::Alive);
    }

    #[test]
    fn check_liveness_stays_alive_when_probe_unknown() {
        let _g = TMUX_BIN_LOCK.lock().unwrap();
        let dir = tempfile::TempDir::new().unwrap();
        // Recorded socket's server is down (non-zero exit). The agent PID (self)
        // is alive, so an inconclusive tmux probe must NOT reap it — this is the
        // regression the tri-state fixes (wrong/dead socket → false TmuxGone).
        let _e = EnvGuard::set("TMUX_BIN", fake_tmux(dir.path(), &["no server"], 1));
        let probe = AgentProbe {
            pid: std::process::id(),
            start_time: None,
            tmux_window: Some("🚀 wt/x".to_string()),
            tmux_identity: Some(id(Some("/tmp/dead-sock"), "octl", "@42")),
            skip_tmux_check: false,
        };
        assert_eq!(check_liveness(&probe), Liveness::Alive);
    }

    #[test]
    fn check_liveness_falls_back_to_bare_name_without_identity() {
        let _g = TMUX_BIN_LOCK.lock().unwrap();
        let dir = tempfile::TempDir::new().unwrap();
        // Legacy node: no identity. The fallback queries by window NAME, so the
        // fake returns the bare name we expect.
        let _e = EnvGuard::set("TMUX_BIN", fake_tmux(dir.path(), &["legacy-win"], 0));
        let probe = AgentProbe {
            pid: std::process::id(),
            start_time: None,
            tmux_window: Some("legacy-win".to_string()),
            tmux_identity: None,
            skip_tmux_check: false,
        };
        assert_eq!(check_liveness(&probe), Liveness::Alive);
    }
}
