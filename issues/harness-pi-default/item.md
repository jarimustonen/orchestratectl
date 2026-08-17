---
created: 2026-08-17
updated: 2026-08-17
type: improvement
reporter: jari
status: done
priority: normal
lane: surface
lane_seq: 15
closed: 2026-08-17
---

# Make pi the built-in default harness per ADR 0001 D4

_Source: crates/octl-cli/src/harness_

## Description

ADR docs/decisions/0001-thin-supervisor-vs-harden.md D4 decides: pi.dev is the universal default harness (autonomous and interactive), claude is a non-default opt-in. The code still has DEFAULT_HARNESS = "claude" in crates/octl-cli/src/harness/mod.rs, and select.rs's module docs describe claude as the intended default. On Jari's machine this is masked because ~/.orchestratectl/config.toml sets [harness] default = "pi", so observed behavior already matches the ADR — but a fresh install with no config file gets claude, contradicting D4. Fix: flip the built-in default to pi, update the harness module docs and crates/octl-cli/AGENTS.md (which also documents claude as default), and re-run the insta snapshot loop (config show / help surfaces may bake the default in). Precedence order itself (flag > env > file > built-in) is unchanged. Sequenced before add-configurable-agent, which builds on the same resolver.
