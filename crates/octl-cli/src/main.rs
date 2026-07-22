mod cli;
mod discussion;
mod doctor;
mod error;
mod event;
// Behind-the-seam code-pipeline harness contract (design.md §10). Landed as
// unused-by-default scaffolding + tests; nothing in the live `run create` /
// supervisor path constructs a `CodeHarness` yet — staged rollout (design §14)
// wires it in later. `#[allow(dead_code)]` covers the whole subtree until then.
#[allow(dead_code)]
mod harness;
mod help;
mod home;
mod idempotency;
mod node;
mod output;
mod proc;
mod run;
mod skill;
mod spinoff;
mod supervise;

use std::process::ExitCode;

fn main() -> ExitCode {
    cli::run()
}
