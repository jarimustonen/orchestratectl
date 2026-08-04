---
created: 2026-08-04
updated: 2026-08-04
type: chore
reporter: jari
status: done
priority: normal
closed: 2026-08-04
commits:
- hash: 9ef7287
  summary: document GIT_CONFIG_GLOBAL runner gotcha in dist-workspace.toml
---

# Release mac build fails: GIT_CONFIG_GLOBAL on hauis runner breaks actions/checkout

_Source: .github/workflows/release.yml_

## Description

The v0.1.0 release mac build (aarch64-apple-darwin on the self-hosted hauis runner) failed 3x at actions/checkout@v4 with 'Unable to replace auth placeholder in .../.gitconfig', cascading to skip host + publish-homebrew-formula. Root cause: the runner's ~/actions-runner/.env set GIT_CONFIG_GLOBAL=/Users/jari/actions-runner/.gitconfig-ci. actions/checkout overrides HOME and reads $HOME/.gitconfig, but git honors GIT_CONFIG_GLOBAL and wrote the auth token to .gitconfig-ci, so checkout could not find the placeholder to substitute. Fix: removed the GIT_CONFIG_GLOBAL line from the runner .env (backup at .env.bak-*) and restarted the runner service; documented the gotcha in dist-workspace.toml. A dotfiles symlink at ~/.gitconfig is fine (existed during the successful July release).
