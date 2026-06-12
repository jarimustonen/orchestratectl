---
created: 2026-06-12
updated: 2026-06-12
type: chore
status: open
priority: normal
---

# Hoist text-mode warning emission into central dispatcher

## Description

Subcommands today format their own text-mode warnings (for w in warnings { eprintln!("warning: {}", w); }) at the end of every command path. This will duplicate as more subcommands land. Hoist the formatting into the central cli::run dispatcher (or a small helper) so subcommands just return (payload, warnings) and presentation lives in one place. Surfaced by review of #version-subcommand — see history/review-version-subcommand.md §6. Low priority; trivial today (one subcommand) but worth recording so it lands before fan-out across the subcommand tree. Depends on: cargo-scaffolding only.
