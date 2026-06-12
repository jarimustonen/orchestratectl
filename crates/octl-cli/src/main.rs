mod cli;
mod error;
mod output;

use std::process::ExitCode;

fn main() -> ExitCode {
    cli::run()
}
