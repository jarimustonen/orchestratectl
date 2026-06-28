---
created: 2026-06-28
updated: 2026-06-28
type: bug
status: open
priority: high
---

# run create --headless crashes create.sh with unknown-flag --parent-session

_Source: src/run/create + create.sh_

## Description

orchestratectl run create --headless (and presumably --tmux-session) forwards `--parent-session <name>` to create.sh, but create.sh rejects it:

```
{"code":"unknown-flag","message":"Unknown flag: --parent-session","expected":"--type, --agent, --layout, --no-hooks, --keep-tmux-on-error, --agent-startup-timeout"}
```

orchestratectl then wraps this as `create_sh_error_create_sh_unparseable` and exits 2, so the spawn fails entirely. Repro: `orchestratectl run create --kind orchestrated --headless ...` during the /orchestrate smoke test (2026-06-28). The --headless/--tmux-session feature (commit 1418fa6) added the flag on the Rust side but create.sh does not accept `--parent-session`. Either teach create.sh the flag or rename to what create.sh expects (`--parent-session` vs the workmux session arg). Found during /orchestrate end-to-end smoke test.
