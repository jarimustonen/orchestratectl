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
    pub fn is_terminal(self) -> bool {
        !matches!(self, Liveness::Alive)
    }

    pub fn reason(self) -> &'static str {
        match self {
            Liveness::Alive => "alive",
            Liveness::Dead => "agent-died",
            Liveness::Recycled => "agent-pid-recycled",
            Liveness::TmuxGone => "agent-tmux-window-gone",
        }
    }
}

/// Read the process start_time (Unix seconds) for `pid`. Returns `None`
/// if the process does not exist or the platform sysinfo backend
/// declines to populate the value.
pub fn pid_start_time(pid: u32) -> Option<u64> {
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::Some(&[Pid::from_u32(pid)]),
        true,
        sysinfo::ProcessRefreshKind::new(),
    );
    sys.process(Pid::from_u32(pid)).map(|p| p.start_time())
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

/// Snapshot of what we know about an agent at spawn time. Compared
/// against live probes by [`check_liveness`].
#[derive(Debug, Clone)]
pub struct AgentProbe {
    pub pid: u32,
    /// Start-time captured at spawn (Unix seconds). `None` means the
    /// supervisor could not read start_time on this platform — we then
    /// fall back to PID-only liveness and accept the tiny risk of a
    /// reused PID being mistaken for the agent.
    pub start_time: Option<u64>,
    pub tmux_window: Option<String>,
    /// When `true`, skip the tmux probe. Used when tmux is not the
    /// host (e.g. fake-spawn test fixtures) or tmux unavailability is
    /// not a failure signal.
    pub skip_tmux_check: bool,
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
        if let Some(name) = probe.tmux_window.as_deref() {
            if !tmux_window_present(name) {
                return Liveness::TmuxGone;
            }
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
}
