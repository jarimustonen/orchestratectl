---
created: 2026-07-26
updated: 2026-08-06
type: feature
status: in-progress
priority: normal
---

# run wait --timeout requires a unit suffix; bare integer rejected

## Description


## Observed

During a `/stint` round I ran (via a skill's documented pattern):

```
orchestratectl run wait <run-id> --timeout 2400
```

It failed immediately with:

```json
{"schema_version":1,"error":{"code":"invalid_arguments","message":"error: invalid value '2400' for '--timeout <TIMEOUT>': invalid duration '2400': time unit needed, for example 2400sec or 2400ms"}}
```

Because it exited instantly (exit 0 from the wrapping shell, error on the CLI),
a backgrounded wait "completed" without actually waiting — the caller thought the
run had settled when it hadn't. Re-running with `--timeout 2400sec` worked.

## Expected / suggestion

The error message itself is clear and self-correcting — good. Two low-cost improvements:

1. **Accept a bare integer as seconds** (treat `2400` as `2400sec`), which is the
   most common expectation for a `--timeout` flag; OR
2. **Fix the docs** — the `worktree-spinoff` skill (and any other bundled skill that
   shows `run wait … --timeout`) presents `--timeout` without noting the unit
   requirement. If bare integers stay rejected, the skill examples should show a
   unit (`--timeout 2400sec`) so agents don't hit this.

Preference: (1) — matches least-surprise for a duration flag and avoids the
silent-instant-exit failure mode entirely. Priority low; the workaround is trivial
once you know the unit is required.
