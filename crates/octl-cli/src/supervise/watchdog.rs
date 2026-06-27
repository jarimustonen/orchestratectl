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

/// Check whether `tmux list-windows` includes a window whose name
/// matches `window_name`. A tmux server that is not running counts as
/// "window not present" (returns `false`).
///
/// Network-namespace / non-default socket considerations: callers may
/// override the tmux invocation via `TMUX_BIN` env var (mostly for
/// tests that mock tmux out).
pub fn tmux_window_present(window_name: &str) -> bool {
    let bin = std::env::var("TMUX_BIN").unwrap_or_else(|_| "tmux".to_string());
    let out = match Command::new(bin)
        .args(["list-windows", "-a", "-F", "#{window_name}"])
        .output()
    {
        Ok(o) => o,
        Err(_) => return false,
    };
    if !out.status.success() {
        return false;
    }
    out.stdout
        .split(|b| *b == b'\n')
        .any(|line| line == window_name.as_bytes())
}

/// Check whether the *exact* window named by a [`TmuxIdentity`] is present,
/// querying the server on its recorded socket and scoping to its session:
/// `tmux [-S <socket>] list-windows -t <session> -F '#{window_id}'`, then an
/// exact (not substring) match against `window_id`. This is the precise form:
/// unlike [`tmux_window_present`] it cannot false-positive on a same-named
/// window in another session, and it can see a window on a non-default socket.
///
/// A tmux server that is not running, an unknown session, or any non-zero exit
/// all count as "window not present" (`false`). Honors the `TMUX_BIN` override
/// for tests.
pub fn tmux_window_present_qualified(identity: &TmuxIdentity) -> bool {
    let bin = std::env::var("TMUX_BIN").unwrap_or_else(|_| "tmux".to_string());
    let mut cmd = Command::new(bin);
    if let Some(socket) = identity.socket.as_deref() {
        cmd.args(["-S", socket]);
    }
    cmd.args([
        "list-windows",
        "-t",
        &identity.session,
        "-F",
        "#{window_id}",
    ]);
    let out = match cmd.output() {
        Ok(o) => o,
        Err(_) => return false,
    };
    if !out.status.success() {
        return false;
    }
    out.stdout
        .split(|b| *b == b'\n')
        .any(|line| line == identity.window_id.as_bytes())
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
    /// probe matches on `session:window_id` + socket (precise); when `None`
    /// (a node registered before create.sh emitted the qualified fields), it
    /// falls back to a bare-name match on [`AgentProbe::tmux_window`].
    pub tmux_identity: Option<TmuxIdentity>,
    /// When `true`, skip the tmux probe. Used when tmux is not the
    /// host (e.g. fake-spawn test fixtures) or tmux unavailability is
    /// not a failure signal.
    pub skip_tmux_check: bool,
}

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
    let mut seen = WARNED
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .unwrap();
    if seen.insert(window_name.to_string()) {
        tracing::warn!(
            tmux_window = window_name,
            "node has no qualified tmux identity; falling back to bare \
             window-name liveness matching (registered before create.sh \
             emitted session/window_id) — this is ambiguous across sessions"
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
        let present = match probe.tmux_identity.as_ref() {
            // Precise path: match the exact window on its own socket+session.
            Some(identity) => Some(tmux_window_present_qualified(identity)),
            // Legacy path: node registered before create.sh emitted the
            // qualified fields. Fall back to the ambiguous bare-name match.
            None => probe.tmux_window.as_deref().map(|name| {
                warn_legacy_bare_name_once(name);
                tmux_window_present(name)
            }),
        };
        // `None` means we have nothing to probe with — don't fail liveness on
        // that absence (matches the prior bare-name behavior).
        if present == Some(false) {
            return Liveness::TmuxGone;
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

    /// Write an executable fake `tmux` that records its args to `<dir>/args`
    /// and prints `stdout_body` verbatim, exiting 0. `stdout_body` is the set
    /// of `#{window_id}` lines the fake "server" reports.
    fn fake_tmux(dir: &Path, stdout_body: &str) -> PathBuf {
        let p = dir.join("fake-tmux.sh");
        let body = format!(
            "#!/bin/bash\nprintf '%s' \"$*\" > \"{}/args\"\nprintf '%s\\n' '{}'\n",
            dir.display(),
            stdout_body
        );
        std::fs::write(&p, body).unwrap();
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&p).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&p, perms).unwrap();
        p
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
        // Server reports two windows; ours (@42) is among them.
        let bin = fake_tmux(dir.path(), "@7\n@42\n@99");
        std::env::set_var("TMUX_BIN", &bin);
        let present = tmux_window_present_qualified(&id(Some("/tmp/sock"), "octl", "@42"));
        // The probe scoped to the recorded socket + session.
        let args = std::fs::read_to_string(dir.path().join("args")).unwrap();
        std::env::remove_var("TMUX_BIN");
        assert!(present, "window @42 should be reported present");
        assert!(args.contains("-S /tmp/sock"), "socket not passed: {args}");
        assert!(args.contains("-t octl"), "session not scoped: {args}");
    }

    #[test]
    fn qualified_absent_when_window_id_missing() {
        let _g = TMUX_BIN_LOCK.lock().unwrap();
        let dir = tempfile::TempDir::new().unwrap();
        // Same-named-but-different windows exist; ours (@42) does not — this is
        // exactly the cross-session false-positive the bare-name match made.
        let bin = fake_tmux(dir.path(), "@7\n@99");
        std::env::set_var("TMUX_BIN", &bin);
        let present = tmux_window_present_qualified(&id(None, "octl", "@42"));
        let args = std::fs::read_to_string(dir.path().join("args")).unwrap();
        std::env::remove_var("TMUX_BIN");
        assert!(!present, "window @42 should be reported absent");
        // No socket recorded → no `-S` flag (default socket).
        assert!(
            !args.contains("-S"),
            "unexpected -S with null socket: {args}"
        );
    }

    #[test]
    fn check_liveness_uses_qualified_identity_for_tmux_gone() {
        let _g = TMUX_BIN_LOCK.lock().unwrap();
        let dir = tempfile::TempDir::new().unwrap();
        // Live PID (self) + start_time skipped, so the verdict turns purely on
        // the tmux probe. The fake server lists @99, not our @42 → TmuxGone.
        let bin = fake_tmux(dir.path(), "@99");
        std::env::set_var("TMUX_BIN", &bin);
        let probe = AgentProbe {
            pid: std::process::id(),
            start_time: None,
            tmux_window: Some("🚀 wt/x".to_string()),
            tmux_identity: Some(id(Some("/tmp/sock"), "octl", "@42")),
            skip_tmux_check: false,
        };
        let v = check_liveness(&probe);
        std::env::remove_var("TMUX_BIN");
        assert_eq!(v, Liveness::TmuxGone);
    }

    #[test]
    fn check_liveness_alive_when_qualified_window_present() {
        let _g = TMUX_BIN_LOCK.lock().unwrap();
        let dir = tempfile::TempDir::new().unwrap();
        let bin = fake_tmux(dir.path(), "@42");
        std::env::set_var("TMUX_BIN", &bin);
        let probe = AgentProbe {
            pid: std::process::id(),
            start_time: None,
            tmux_window: Some("🚀 wt/x".to_string()),
            tmux_identity: Some(id(None, "octl", "@42")),
            skip_tmux_check: false,
        };
        let v = check_liveness(&probe);
        std::env::remove_var("TMUX_BIN");
        assert_eq!(v, Liveness::Alive);
    }

    #[test]
    fn check_liveness_falls_back_to_bare_name_without_identity() {
        let _g = TMUX_BIN_LOCK.lock().unwrap();
        let dir = tempfile::TempDir::new().unwrap();
        // Legacy node: no identity. The fallback queries by window NAME, so the
        // fake returns the bare name we expect.
        let bin = fake_tmux(dir.path(), "legacy-win");
        std::env::set_var("TMUX_BIN", &bin);
        let probe = AgentProbe {
            pid: std::process::id(),
            start_time: None,
            tmux_window: Some("legacy-win".to_string()),
            tmux_identity: None,
            skip_tmux_check: false,
        };
        let v = check_liveness(&probe);
        std::env::remove_var("TMUX_BIN");
        assert_eq!(v, Liveness::Alive);
    }
}
