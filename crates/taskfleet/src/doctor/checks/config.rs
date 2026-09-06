//! `config.home` — the Taskfleet home directory resolves to a
//! writable location (AGENTS-AI-FIRST-CLI §18 "configuration integrity").
//!
//! Read-only: doctor never *creates* the home directory (that is the
//! empty-state the real subcommands handle on first write). A missing
//! default home is therefore OK, not a failure; only a home that exists
//! but cannot be used (a file where a directory belongs, or a read-only
//! directory) is a problem.

use std::path::Path;

use crate::doctor::check::CheckResult;

use super::Ctx;

const ID: &str = "config.home";

pub fn check(ctx: &Ctx) -> Vec<CheckResult> {
    let source = crate::home::home_source().ok();

    // No resolved Taskfleet home → ctx.root is None.
    let Some(root) = ctx.root.as_deref() else {
        return vec![CheckResult::fail(
            ID,
            "cannot resolve Taskfleet home: no home input is available",
            "set HOME, or export TASKFLEET_HOME=<dir>",
        )];
    };

    let is_custom = matches!(
        source,
        Some(crate::home::HomeSource::CanonicalExplicit | crate::home::HomeSource::InternalWorker)
    );

    if !root.exists() {
        // Missing home is the empty state — it is created lazily on first
        // write. An *explicitly configured* home that does not exist is
        // more suspicious (likely a typo), so it warns; the default just
        // reports OK.
        return vec![if is_custom {
            CheckResult::warn(
                ID,
                format!(
                    "explicit Taskfleet home points at non-existent path {}",
                    root.display()
                ),
                format!(
                    "create {} or unset the explicit home variable",
                    root.display()
                ),
            )
        } else {
            CheckResult::ok(
                ID,
                format!(
                    "{} absent (empty state; created on first use)",
                    root.display()
                ),
            )
        }];
    }

    if !root.is_dir() {
        return vec![CheckResult::fail(
            ID,
            format!("Taskfleet home {} is not a directory", root.display()),
            format!(
                "remove {} or point TASKFLEET_HOME elsewhere",
                root.display()
            ),
        )];
    }

    if !is_writable(root) {
        return vec![CheckResult::fail(
            ID,
            format!("Taskfleet home {} is not writable", root.display()),
            format!(
                "chmod u+w {} or point TASKFLEET_HOME elsewhere",
                root.display()
            ),
        )];
    }

    vec![CheckResult::ok(
        ID,
        format!("{} is a writable directory", root.display()),
    )]
}

/// Writability probe that does not mutate the filesystem (doctor's
/// read-only default). On Unix uses `access(2)` with `W_OK`, which honours
/// the *effective* uid/gid, ownership, and read-only mounts — unlike
/// `Permissions::readonly()`, which only inspects mode bits and returns
/// "writable" for, e.g., a root-owned 0755 dir the current user cannot
/// write. Off Unix, falls back to the mode-bit heuristic.
#[cfg(unix)]
fn is_writable(dir: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c_path) = std::ffi::CString::new(dir.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: `access(2)` is read-only and side-effect-free; it only
    // probes the path against the process's real uid/gid for W_OK.
    unsafe { libc::access(c_path.as_ptr(), libc::W_OK) == 0 }
}

#[cfg(not(unix))]
fn is_writable(dir: &Path) -> bool {
    match std::fs::metadata(dir) {
        Ok(meta) => !meta.permissions().readonly(),
        Err(_) => false,
    }
}
