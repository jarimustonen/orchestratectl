//! Current-executable command construction for hidden self-exec paths.
//!
//! Detached supervision, reattach/recovery, and doctor fixes must re-enter the
//! binary that is already running. Looking up a product name on `PATH` could
//! cross versions or invocation identities during the bounded rename window.

use std::process::Command;

/// Resolve this process's exact executable image.
pub(crate) fn executable() -> std::io::Result<std::path::PathBuf> {
    // Unit tests that execute a generated launcher run inside Rust's test
    // harness, not the CLI binary. Keep their absolute fixture seam out of
    // release builds; production always resolves the current image.
    #[cfg(test)]
    if let Some(path) = std::env::var_os("OCTL_TEST_SELF_EXE") {
        return Ok(path.into());
    }
    std::env::current_exe()
}

/// Start a command targeting this process's exact executable image.
pub(crate) fn command() -> std::io::Result<Command> {
    executable().map(Command::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_uses_exact_current_executable_not_path() {
        let command = command().expect("current executable");
        assert_eq!(command.get_program(), executable().unwrap());
        assert!(command.get_args().next().is_none());
    }
}
