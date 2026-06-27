# doctor-subcommand — handoff notes

## jsonl format decision (§18 vs jsonl semantics)

§18's example shows a **bundled** `{schema_version, checks:[...], summary:{...}}`
document. This binary's jsonl convention everywhere else is **one event per line**.

Decision (per the task's stated default): `--output jsonl` (the default) **streams** —
one self-describing object per line, terminated by a summary event:

```
{"event":"check","schema_version":1,"id":"config.home","status":"ok","message":"..."}
{"event":"fix","schema_version":1,"check_id":"skill.sync.foo","applied":true,...}   # only under --fix
{"event":"summary","schema_version":1,"ok":6,"warn":3,"fail":0}
```

The §18 **bundled** shape is preserved verbatim under `--output json`:
`{schema_version:1, data:{checks:[...], summary:{ok,warn,fail}}}` (plus
`data.fixes_applied[]` under `--fix`).

Every jsonl line carries `schema_version` and an `event` discriminator
(`check`/`fix`/`summary`) so a streaming consumer can version-check and route each
record independently (review finding #3). This was NOT the §18 jsonl-vs-bundled
*blocker* — the task pre-selected the streaming default and only asked that the choice
be documented; this note is that documentation.

## `--fix --dry-run` planning envelope

Uses the §11 envelope `{schema_version, dry_run:true, would:[...]}` for both json and
jsonl (a single planning document, not a per-line stream — the plan is one artifact).
The task's parenthetical called the key `dry_run_plan`; §11 (canonical) uses `would`, so
`would` is what shipped. Exit code is always 0 for dry-run (the plan itself succeeded).

## Known limitation: supervisor PID-reuse (review #11)

`data.orphan-supervisor.<id>` uses `kill(pid,0)` against the integer in `supervisor.pid`.
If the original supervisor died and the OS recycled its PID into an unrelated process,
the check reports a false OK. All four reviewers rated this low / MVP-acceptable.

A proper fix needs process **identity**, not just liveness — the codebase already has
start-time-based recycled-PID detection in `supervise::watchdog` (used for agent PIDs via
`node.json`), but `supervisor.pid` stores only the integer. Closing this would mean
writing a start-time (or nonce) into the pid-file format — a cross-cutting change to the
supervisor write path, out of scope here. **Spin-off candidate** if/when supervisor
health needs to be authoritative.

## Other deferred / spin-off candidates

- **Timeout on the `skill install` self-exec** (review #13): currently stdin is detached
  (`Stdio::null()`) so it can't block on a prompt, and the op is local-fs only (no
  network), so hang risk is low. A `wait-timeout`-style guard would need a new dep.
- **Re-run checks after `--fix`** (review #16): the report shows pre-fix check statuses
  plus a `fixes_applied[]` list. Unambiguous but two-phase; a post-fix re-run + second
  summary could be added if agents want a single converged view.

## Not bugs (context the external reviewers lacked)

- Text output "ignoring `--output PATH`": impossible — `output.rs`'s parser never yields
  `format=text` together with a file destination (a `.json/.jsonl` path forces a json
  format; any other extension is rejected).
- `install_skill` reading only stderr on failure: correct — error envelopes go to stderr
  (`error.rs::emit`), success envelopes to stdout.
