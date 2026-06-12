mod cli;
mod error;
mod event;
mod home;
mod idempotency;
mod node;
mod output;
mod run;
mod skill;

use std::process::ExitCode;

fn main() -> ExitCode {
    cli::run()
}
