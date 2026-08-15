mod cli;
// User-facing configuration file (`~/.orchestratectl/config.toml`), the `file`
// layer of the flag > env > file > default precedence (AGENTS-AI-FIRST-CLI §8).
// Currently read only for the `run create --harness` default; a missing/empty
// file is the common case and yields the built-in defaults.
mod config;
mod doctor;
mod error;
mod event;
// Vendored typed git-worktree/branch wrapper (issue `workmux-extract-libs`,
// following the `multiplexer` precedent). The supervisor teardown + reconcile
// paths route their git subprocesses through `git::repo::Git`.
mod git;
// The light worker-harness launcher (0.2): harness-name registry + `--harness`
// precedence resolver (`harness::select`), the workmux-agent mapping
// (`harness::workmux_agent`), and the pi worker-prompt translation shim
// (`harness::prompt`). This is the surviving 0.2 harness surface — the heavy
// code-pipeline `CodeHarness` layer (bakeoff/conformance/adapters) was cut.
mod harness;
mod help;
mod home;
mod idempotency;
mod multiplexer;
mod node;
mod output;
mod proc;
mod run;
mod run_worker;
mod skill;
mod supervise;

use std::process::ExitCode;

fn main() -> ExitCode {
    cli::run()
}
