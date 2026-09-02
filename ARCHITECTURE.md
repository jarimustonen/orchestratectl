# Taskfleet architecture

Taskfleet is a local, file-backed orchestration CLI. The canonical `taskfleet`
binary and bounded Cargo-only `orchestratectl` wrapper call one shared dispatcher;
there is no second engine and hidden subprocesses re-execute the current binary.

## Code map

- `crates/taskfleet-core/` owns durable run schemas, append-only events, flock
  witnesses, reduction, path containment, and atomic projection writes.
- `crates/taskfleet/` owns the CLI, run/node/event commands, harness selection,
  worker prompt generation, supervisors, merge recovery, doctor, and bundled
  skills.
- `compat/orchestratectl/` is the implementation-free compatibility binary.
- `contracts/worker-telemetry-v1/` is the runtime-neutral adapter contract. Its
  `OCTL_*` variables and `orchestratectl.worker-telemetry-adapter` id are stable
  protocol vocabulary, not product branding.
- `docs/decisions/` records architectural decisions. ADR 0002 governs the
  staged Taskfleet rename and compatibility window.

## State and process boundaries

Each run has an append-only `events.jsonl`, reduced `manifest.json` and node
projections, and a per-run supervisor. Writers append and apply under the
`LockedRun` witness; multi-file readers take a shared lock. `run merge` is an
OID-recorded recoverable transaction and is the only success truth. The
supervisor is the canonical worktree/tmux teardown actor and preserves all work
outside an explicit merge.

Fresh state uses `~/.taskfleet`. Through the bounded compatibility window, the
central resolver can adopt a sole populated `~/.orchestratectl` home in place,
accept old branded inputs with warnings, and refuse split truth. Optional state
movement is explicit, quiescent, same-filesystem, receipt-backed, and never
rewrites event bytes.

## Skills and generated prompts

Bundled templates live at `crates/taskfleet/skills/<name>/SKILL.template.md` and
are rendered with the package version at build time. Generic workflow identities
(`/worktree-*`, `/fan-out`, `/stint-*`) remain stable; Taskfleet-owned overview
and low-level skills use `taskfleet-*` names. New prompts and copyable commands
always use the canonical command and exact full run id.

Default installation writes Claude and pi layouts; Codex uses a flat prompt
layout. Canonical ownership markers and pi schema-v3 provenance record hashes.
The 0.5.1 branded skill migration moves only byte-identical recorded copies,
preserves edited/unmanaged/stale/corrupt files, and refuses partial old/new
ownership rather than choosing a winner.

## Release boundary

The ADR 0002 R6 crates.io saga, pinned Shipshape protocol, and R7 canonical
cargo-dist/Homebrew topology are prepared. Release activation remains explicitly
blocked in `release/taskfleet-release.json` through R8 integrated validation, R9
repository rename, and the R10 cut. Publishing, tagging, tap activation, and
global installation remain outside this phase.
