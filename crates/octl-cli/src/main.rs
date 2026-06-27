mod cli;
mod discussion;
mod doctor;
mod error;
mod event;
mod help;
mod home;
mod idempotency;
mod node;
mod output;
mod run;
mod skill;
mod spinoff;
mod supervise;

use std::process::ExitCode;

fn main() -> ExitCode {
    cli::run()
}
