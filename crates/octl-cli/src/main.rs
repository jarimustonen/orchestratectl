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
// Vendored typed git-worktree/branch wrapper (issue `workmux-extract-libs`,
// following the `multiplexer` precedent). The supervisor teardown + reconcile
// paths route their git subprocesses through `git::repo::Git`.
mod git;
// Behind-the-seam code-pipeline harness contract (design.md §10). Nothing in the
// live `run create` / supervisor path constructs a `CodeHarness` yet — staged
// rollout (design §14) wires it in later. The one live surface is the standalone
// `harness bakeoff` subcommand (`harness::bakeoff`), which drives the real agent
// adapters to compare agent loops; it is explicitly run, never part of the
// supervisor path. `#[allow(dead_code)]` still covers the parts of the subtree
// (protocol variants, helpers) not reached until the supervisor wiring lands.
#[allow(dead_code)]
mod harness;
mod help;
mod home;
mod idempotency;
mod multiplexer;
mod node;
mod output;
// Behind-the-seam inverted control loop (design.md §2 + §0.2, breakdown T4). The
// tiered orchestrator + typed action primitives + decision envelopes + loop
// skeleton, as a pure in-memory state machine with deterministic stubs. Nothing
// in the live `run create` / supervisor path constructs an `Orchestrator` or
// calls `drive` yet — staged rollout (design §14) wires it into the real
// supervisor + event log at T5. `#[allow(dead_code)]` covers the subtree until
// then; `unused_imports` is allowed alongside because the module's convenience
// re-exports (`pub use`) have no in-crate consumer until T5 either (mirrors the
// `floor` module).
#[allow(dead_code, unused_imports)]
mod pipeline;
mod proc;
mod run;
mod skill;
mod spinoff;
mod supervise;

use std::process::ExitCode;

fn main() -> ExitCode {
    cli::run()
}
