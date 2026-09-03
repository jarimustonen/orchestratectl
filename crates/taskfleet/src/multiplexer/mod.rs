//! Taskfleet-local terminal-multiplexer operations.
//!
//! A vendored, minimal slice of workmux's multiplexer abstraction: only the
//! tmux operations the supervisor needs — `kill_window`, `kill_session`,
//! `new_session` (detached / "headless"), and window lookup. The
//! kitty/wezterm/zellij backends and the full `Multiplexer` trait are
//! deliberately omitted; the supervisor talks tmux only, and the git side of
//! native spawn identity capture lives in `run::spawn`.
//!
//! ## Why this is vendored (not a crate dependency)
//!
//! raine (workmux's maintainer) declined splitting workmux into publishable
//! library crates — the versioning / API-stability burden outweighed the reuse
//! — and suggested duplicating the tmux slice instead (issues
//! `workmux-extract-libs`, `vendor-workmux-multiplexer`). This module is that
//! duplicate: a fork-and-own copy of the tmux backend's kill/lookup/create
//! surface, adapted to the supervisor's needs (optional server socket, lenient
//! best-effort teardown, exact-cwd window lookup). See [`tmux`] for provenance.

pub mod tmux;
