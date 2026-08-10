---
created: 2026-08-10
updated: 2026-08-10
type: improvement
status: open
priority: normal
---

# run show: count_jsons silently returns 0 on filesystem read failure

## Description

`crates/octl-cli/src/run/show.rs::count_jsons` maps a `read_dir` failure to `0` and uses
`filter_map(Result::ok)`, so permission/IO failures on a run's `nodes/`, `discussions/`,
or `spinoffs/` dir are indistinguishable from an empty run — `data.counts` can silently
report zero while the rest of the payload looks valid. Consider returning a `Result` and
surfacing the error (or at least a warning). Pre-existing; raised by the llm-review panel
on `run-show-json-null-fields` and deferred as a separate change since it alters the
payload/error shape.
