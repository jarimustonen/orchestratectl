## Origin

Deferred from `run-paths-typed-selector-split`'s `/llm-review` (Gemini, GPT-5.6-sol,
Opus 4.7 raised it; consensus that it is a behavior change and should NOT ride on the
type-safety refactor — the old code silently swallowed too, so parity was preserved).

## Problem

Supervisor path-resolution sites are best-effort: after the typed-selector split they
read

    parse_run_id(&cid).ok().and_then(|rid| run_paths_exact(root, &rid).ok())

which collapses THREE distinct outcomes into `None`:

1. a **malformed persisted id** (`parse_run_id` fails) — this is on-disk CORRUPTION,
   a bad `child_run_id` written into `child.spawned` or a truncated id in a manifest;
2. a symlink/tamper / corrupt-run-path detection from `run_paths_exact`;
3. genuine transient absence / I/O.

(1) and (2) deserve at least a warning log — ideally a `corrupt_state` classification
with path/event context — but are silently dropped, same as (3). The highest-value
site is `signal_children_term` at teardown: silently orphaning a child whose recorded
id doesn't parse leaves a live agent process behind.

## Proposed direction

Introduce a `parse_persisted_run_id(value, context) -> Result<RunId, CliError>` that
maps malformed durable ids to `corrupt_state` (not the CLI `invalid_id`), and a small
`try_child_paths` helper that logs/warns on parse-or-corruption failure while still
returning `None` for the best-effort callers (DRY across the ~5 sites). Keep transient
absence silent. Reconsider whether teardown should be louder than reconciliation.

Related: error-class inconsistency `invalid_run_id` (CLI selector) vs `invalid_id`
(`parse_run_id`) — worth unifying or explicitly documenting as part of this pass.
