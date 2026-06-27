//! `config.home` — the orchestratectl home directory resolves to a
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
    let custom = std::env::var_os("ORCHESTRATECTL_HOME");

    // Neither ORCHESTRATECTL_HOME nor HOME resolved → ctx.root is None.
    let Some(root) = ctx.root.as_deref() else {
        return vec![CheckResult::fail(
            ID,
            "cannot resolve orchestratectl home: neither ORCHESTRATECTL_HOME nor HOME is set",
            "set HOME, or export ORCHESTRATECTL_HOME=<dir>",
        )];
    };

    let is_custom = custom.as_ref().is_some_and(|v| !v.is_empty());

    if !root.exists() {
        // Missing home is the empty state — it is created lazily on first
        // write. An *explicitly configured* home that does not exist is
        // more suspicious (likely a typo), so it warns; the default just
        // reports OK.
        return vec![if is_custom {
            CheckResult::warn(
                ID,
                format!(
                    "ORCHESTRATECTL_HOME points at non-existent path {}",
                    root.display()
                ),
                format!("create {} or unset ORCHESTRATECTL_HOME", root.display()),
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
            format!("orchestratectl home {} is not a directory", root.display()),
            format!(
                "remove {} or point ORCHESTRATECTL_HOME elsewhere",
                root.display()
            ),
        )];
    }

    if !is_writable(root) {
        return vec![CheckResult::fail(
            ID,
            format!("orchestratectl home {} is not writable", root.display()),
            format!(
                "chmod u+w {} or point ORCHESTRATECTL_HOME elsewhere",
                root.display()
            ),
        )];
    }

    vec![CheckResult::ok(
        ID,
        format!("{} is a writable directory", root.display()),
    )]
}

/// Best-effort writability probe that does not mutate the filesystem
/// (doctor's read-only default). Uses the directory's permission bits
/// rather than attempting a write.
fn is_writable(dir: &Path) -> bool {
    match std::fs::metadata(dir) {
        Ok(meta) => !meta.permissions().readonly(),
        Err(_) => false,
    }
}
