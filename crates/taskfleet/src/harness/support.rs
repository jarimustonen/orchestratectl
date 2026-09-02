//! Shared test helpers for env-mutating tests across the crate.
//!
//! The heavy git-inspecting `CodeHarness` adapter skeleton that used to live
//! here was cut in the 0.2 simplification; the one piece with crate-wide reach
//! that survives is the process-wide env lock the supervisor/watchdog tests
//! share (see [`test_env`]).

#[cfg(test)]
pub(crate) mod test_env {
    //! One process-wide lock for every env-mutating test in the crate.
    //!
    //! The watchdog tests set `TMUX_BIN`/`GIT_BIN`. `std::env::set_var`
    //! is process-global and unsafe to call concurrently with ANY other env access,
    //! so a per-module lock is not enough: two modules' locks don't mutually exclude,
    //! and their tests still race in the shared test binary — corrupting the environ
    //! block and leaking a binary override into another module's assertion
    //! (issue `idempotency-key-allowed-duplicate-run` surfaced this via the watchdog
    //! snapshot tests). A single shared lock (poison-tolerant, so one test's panic
    //! doesn't cascade into spurious `PoisonError`s) serialises them all.
    use std::sync::{Mutex, MutexGuard, PoisonError};

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    pub(crate) fn lock() -> MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
