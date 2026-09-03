//! Taskfleet-local typed git operations.
//!
//! A vendored, minimal slice of git worktree/branch plumbing: only the
//! operations the supervisor's teardown + reconcile paths and the `run merge`
//! path actually use — `rev_list_count`, `is_ancestor`, `merge_tree_clean`,
//! `tip_committer_time`, `worktree_is_clean`, `main_worktree`,
//! `worktree_remove`, and `branch_delete` (with the `-d`/`-D` unmerged-safety
//! distinction). The full branch/remote/status surface is deliberately omitted;
//! native spawn invokes the explicit `workmux add` CLI through `run::spawn`.
//!
//! ## Why this is vendored (not a crate dependency)
//!
//! This mirrors the [`crate::multiplexer`] vendoring. raine (workmux's
//! maintainer) declined splitting workmux into publishable library crates — the
//! versioning / API-stability burden outweighed the reuse — and suggested
//! duplicating the needed slices instead (issues `workmux-extract-libs`,
//! `vendor-workmux-multiplexer`). workmux's counterpart is its ~50KB `src/git/`
//! module (branch + worktree + merge + remote + status); this module vendors the
//! shape of that abstraction — a typed backend threaded a resolved git binary —
//! and no more.
//!
//! ## Provenance & attribution
//!
//! Structured after workmux (raine/workmux), MIT-licensed, `src/git/`; the
//! layout and the "typed backend over a resolved binary" idiom follow that
//! module and the [`crate::multiplexer::tmux`] precedent. The git *operations*
//! themselves are a mechanical extraction of Taskfleet's own scattered
//! `Command::new(git)` call sites in [`crate::supervise::cleanup`] — same git
//! commands, args, order, and lenient error handling — so the state-integrity
//! invariants those call sites encode (the branch-preservation gates, the
//! source-relative ancestry check, the `-d`/`-D` distinction; root CLAUDE.md
//! "State integrity invariants") are preserved byte-for-byte in behavior. The
//! upstream MIT license (Copyright (c) 2025 workmux contributors) permits the
//! vendoring of the module's shape.
//!
//! ## Drift policy
//!
//! Fork-and-own — we do not track upstream drift. The vendored surface is a
//! narrow, stable slice; if a git-invocation bug is found here, fix it in place
//! and (where a workmux equivalent exists) port by hand.

pub mod repo;
