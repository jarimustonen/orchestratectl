mod cli;
mod discussion;
mod doctor;
mod error;
mod event;
// Behind-the-seam deterministic correctness floor (design.md §4). Pure gates +
// a thin capture layer; nothing in the live `run create` / supervisor / `run
// merge` path calls it yet — staged rollout (design §14) wires it into the
// supervisor's merge gate at T5. `#[allow(dead_code)]` covers the subtree until
// then. `unused_imports` is allowed alongside because the module's convenience
// re-exports (`pub use`) have no in-crate consumer until T5 either.
#[allow(dead_code, unused_imports)]
mod floor;
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
