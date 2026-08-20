---
created: 2026-08-17
updated: 2026-08-20
type: bug
reporter: jari
status: cannot-reproduce
priority: normal
labels:
- via:agent-homebase-wrapup
closed: 2026-08-17
---

# node show accepts wrong argument order silently: returns {} with exit 0

## Description

node show accepts wrong argument order silently: returns {} with exit 0

## Observed

Calling `node show` with the run-id passed as a flag instead of a positional argument
returns an empty JSON object and **exit code 0**:

```
$ orchestratectl node show n-0001 --run-id 01m05tmnyc2nkmjdqwrqhfwrv6 --output json
{}
```

The documented form is `orchestratectl node show <run-id> <node-id>`. So the call above is
malformed — `n-0001` lands in the run-id position and `--run-id` is an unrecognised flag (or
is silently accepted and ignored).

## Expected

A non-zero exit with an informative error envelope naming the problem, per the AI-first CLI
contract (strict input validation, informative errors, meaningful exit codes):

```json
{"schema_version":1,"error":{"code":"invalid_arguments","message":"..."}}
```

## Why it matters

`{}` with exit 0 is indistinguishable from a legitimate "this node has no report yet". In the
session where this was hit, the caller briefly concluded that a completed spinoff had returned
no terminal report at all, and only a direct read of the run directory
(`~/.orchestratectl/runs/<id>/events.jsonl`) established that the report existed and its
`spinoff_proposals` were genuinely empty. A wrong-argument call should never be able to
impersonate a valid empty result.

## Suggested fix

Reject unknown flags and wrong-arity positionals at parse time rather than resolving to an
empty projection. If `--run-id` is meant to be a supported alternative spelling, accept it
explicitly; otherwise fail on it.

## Environment

orchestratectl 0.2.0 (macOS arm64). Reproduced against a completed spinoff run.

## Comments

### 2026-08-17T08:14:16Z · @orchestrator

Closed cannot-reproduce 2026-08-17: verified against the RUNNING 0.2.2 binary, not the issue text. `orchestratectl node show n-0001 --run-id <id> --output json` now returns {"schema_version":1,"error":{"code":"unknown_subcommand_or_flag",...}} with exit 1, naming the usage line. The report was filed against 0.2.0. Reopen if an empty {} with exit 0 is observed on 0.2.2 or later.
