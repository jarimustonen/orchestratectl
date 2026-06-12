//! Resolution of the orchestratectl root directory.
//!
//! Order: `$ORCHESTRATECTL_HOME` overrides everything, otherwise
//! `$HOME/.orchestratectl`. Used by every read/write subcommand so the
//! test harness can point at a `tempfile::TempDir` instead of the user's
//! real `~/.orchestratectl/`.

use std::path::PathBuf;

use crate::error::CliError;

pub fn root_dir() -> Result<PathBuf, CliError> {
    if let Ok(custom) = std::env::var("ORCHESTRATECTL_HOME") {
        if custom.is_empty() {
            return Err(CliError::system(
                "home_not_set",
                "ORCHESTRATECTL_HOME is set to an empty string",
            ));
        }
        return Ok(PathBuf::from(custom));
    }
    let home = std::env::var("HOME").map_err(|_| {
        CliError::system(
            "home_not_set",
            "neither ORCHESTRATECTL_HOME nor HOME is set",
        )
    })?;
    Ok(PathBuf::from(home).join(".orchestratectl"))
}
