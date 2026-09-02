# Taskfleet state migration

Taskfleet 0.6–0.7 can adopt a sole populated legacy `~/.orchestratectl` home in place. Physical movement to `~/.taskfleet` is optional and explicit:

```sh
taskfleet state migrate \
  --source /absolute/normalized/.orchestratectl \
  --destination /absolute/normalized/.taskfleet \
  --dry-run --json

taskfleet state migrate \
  --source /absolute/normalized/.orchestratectl \
  --destination /absolute/normalized/.taskfleet --json
```

The command is noninteractive. It rejects relative, non-normalized, symlinked or inaccessible roots, an existing destination, different filesystems, held run locks, nonterminal runs/nodes, live identity-bearing supervisors/workers, pending merge transactions, corrupt events/projections, and an `applied_seq` that does not equal the final event sequence. It hashes every regular state file before the move and verifies the same inventory afterward. It never rewrites event or projection bytes.

## Required operator exclusion

Before dry-run and apply, stop every old `orchestratectl` 0.5.1 process and prevent new old-version commands from starting. Current binaries share an external lock at `$HOME/.taskfleet-migrations/state.lock`, but this cannot fence:

- an already-running 0.5.1 process;
- a future lock or write by an unmodified 0.5.1 binary;
- a process that holds an open descriptor into the old directory.

An open descriptor remains valid across a Unix directory rename. Taskfleet does not pretend this can be discovered or revoked portably. The operator-enforced exclusion is therefore part of the migration precondition.

## Receipt and crash recovery

A pair-keyed JSON receipt lives at `$HOME/.taskfleet-migrations/` (falling back to the Unix account home only when `HOME` is unset), outside both roots. Its durable states are:

1. `prepared` — source bytes and filesystem identity were verified and fsynced before rename;
2. `renamed` — the whole-root same-filesystem rename and parent-directory fsync completed;
3. `verified` — destination bytes match the pre-rename hash;
4. `rollback_prepared` — rename-back intent was fsynced before rollback;
5. `canonical_write_started` — rollback is permanently closed;
6. `rolled_back` — the verified, still-unwritten destination was renamed back.

Re-running the identical `state migrate` command recovers a crash in `prepared` or `renamed`; re-running `state rollback` completes a `rollback_prepared` receipt. If the source vanished and destination appeared while the receipt still says `prepared`, recovery verifies the recorded hash and advances forward. Corrupt, unsupported or contradictory receipts fail closed. Both roots existing always fails; Taskfleet never merges or chooses by timestamp.

## Rollback boundary

Before any ordinary command attempts the first canonical event append, projection/config/skill/supervisor metadata write, or canonical log creation, it durably changes the outside receipt to `canonical_write_started`. Marker-before-write ordering is conservative on an I/O failure, but fail-closed: canonical bytes can never be written while the receipt still authorizes rename-back.

Only a `verified` receipt may be rolled back:

```sh
taskfleet state rollback \
  --source /absolute/normalized/.orchestratectl \
  --destination /absolute/normalized/.taskfleet \
  --dry-run --json
```

Rollback repeats the complete quiescence and hash checks, then uses an atomic same-filesystem rename. Once `canonical_write_started`, rollback is permanently refused; repair or fix forward in the canonical root. No symlink or writable alias is created at either stage.
