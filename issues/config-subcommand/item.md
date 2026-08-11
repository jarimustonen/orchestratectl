---
created: 2026-08-11
updated: 2026-08-11
type: feature
status: open
priority: normal
---

# config subcommand: config path and config show --json

## Description

Follow-up to run-create-harness-flag, which introduced the first config-file layer (~/.orchestratectl/config.toml, [harness] section) but no inspection surface. AGENTS-AI-FIRST-CLI §8 wants: 'config path' (print the config file location) and 'config show --json' (print the effective resolved config with per-key source: flag|env|file|default, secrets redacted). Add the 'config' noun with these verbs. Reuse harness::select::HarnessSource for the harness key's source. Needs its own clap surface + insta snapshot suite.
