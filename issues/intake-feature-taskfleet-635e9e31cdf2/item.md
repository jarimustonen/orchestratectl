---
created: 2026-08-28
updated: 2026-08-28
type: feature
reporter: jari
status: untriaged
priority: normal
provenance: agent:aggountant-wrapup
source_ref: agent:aggountant-wrapup/reporter:jari/id:aggountant-2026-08-28-taskfleet-run-show-source-repo
---

# Expose source_repo in run show JSON

## Description

Expose source_repo in run show JSON

`taskfleet run create` accepts `--source-repo`, but `taskfleet run show --output json` does not expose the persisted source repository.

During an aggountant handoff there were five pending runs from multiple repositories. The preflight had to infer each run's repository from `worktree_path`, for example:

    {
      "worktree_path": "/Users/jari/Sources/3dbear-monorepo__worktrees/wt-nsbyhevnxx-media-cli-acl-rm",
      "branch": null,
      "title": "media-cli-acl-rm"
    }

Expected: the machine-readable `run show` result includes a neutral `source_repo` field matching the value supplied to `run create`, so orchestration and handoff tooling need not parse host-specific worktree paths.
