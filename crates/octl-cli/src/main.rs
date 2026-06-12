mod cli;
mod error;
mod output;
mod skill;

use std::process::ExitCode;

fn main() -> ExitCode {
    cli::run()
}
