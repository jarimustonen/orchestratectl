---
created: 2026-07-25
updated: 2026-07-31
type: bug
reporter: jari
status: fixed
priority: normal
closed: 2026-07-31
---

# idempotency-key did not prevent duplicate run create

_Source: run create --idempotency-key_

## Description

OBSERVED (2026-07-25, 3dbear monorepo): two `run create --kind spinoff
--headless` calls with the SAME `--idempotency-key gitpush-triage-2026-07-25`
produced TWO distinct pending runs (`01kybtp71pscada2dsdkp4hv9j` and
`01kybtpczp1c7bpbmfkkf438hx`), not one. The skill doc says: "Use the same key on
retry and the CLI returns the original run without spawning twice."

Context: the first call was part of a batch of three `run create`s issued in one
background shell; that shell's output got truncated, so it was unclear whether the
third (git-push) call had succeeded. A second call was issued with the same
idempotency-key as a "retry" — and instead of replaying the first run, it created
a duplicate. The extra one then had to be `run cancel`led.

## Expected

The second call with a matching `--idempotency-key` returns an `idempotent_replay`
envelope describing the first run; no second run is spawned.

## Hypotheses to check

- Is the idempotency record only committed/visible after the run fully
  materializes, so two near-simultaneous calls both miss it (race)?
- Or is the key scoped per-process/per-invocation rather than persisted to shared
  run state?

## Repro

Fire two `run create` with the same `--idempotency-key` close together (or before
the first fully registers) and observe whether two runs are created.
