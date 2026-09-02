//! `dep.<bin>` — every external binary Taskfleet shells out to is on
//! `PATH` (AGENTS-AI-FIRST-CLI §18 "dependencies").
//!
//! Presence is enough for MVP: version constraints are intentionally
//! loose (the spec defers minimum-version enforcement). A missing binary
//! is a `FAIL` with an install hint, since the workflows that depend on
//! it cannot run at all.

use std::path::PathBuf;

use crate::doctor::check::CheckResult;

use super::Ctx;

/// (binary name, install hint) for each hard dependency. Hints name a
/// concrete command but stay package-manager-neutral where the tool ships
/// across ecosystems, so the advice is not wrong on Linux/CI hosts.
const DEPS: &[(&str, &str)] = &[
    (
        "tmux",
        "install tmux via your package manager (e.g. brew install tmux)",
    ),
    (
        "git",
        "install git via your package manager (e.g. xcode-select --install)",
    ),
    ("workmux", "install workmux (see its README)"),
    ("issuectl", "cargo install issuectl (see its README)"),
];

pub fn check(_ctx: &Ctx) -> Vec<CheckResult> {
    DEPS.iter()
        .map(|(bin, hint)| {
            let id = format!("dep.{bin}");
            match find_on_path(bin) {
                Some(path) => CheckResult::ok(id, format!("{bin} on PATH at {}", path.display())),
                None => CheckResult::fail(id, format!("{bin} not found on PATH"), *hint),
            }
        })
        .collect()
}

/// Locate an executable on `$PATH` without spawning it. Returns the first
/// matching path that is a regular file (and, on Unix, has an execute
/// bit set).
fn find_on_path(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(bin);
        if is_executable_file(&candidate) {
            return Some(candidate);
        }
    }
    None
}

#[cfg(unix)]
fn is_executable_file(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    match std::fs::metadata(path) {
        Ok(meta) => meta.is_file() && (meta.permissions().mode() & 0o111 != 0),
        Err(_) => false,
    }
}

#[cfg(not(unix))]
fn is_executable_file(path: &std::path::Path) -> bool {
    path.is_file()
}
