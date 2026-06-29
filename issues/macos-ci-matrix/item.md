---
created: 2026-06-27
updated: 2026-06-29
type: improvement
status: fixed
priority: normal
related: ['@ci-and-lints']
closed: 2026-06-29
---

# macOS CI matrix for platform-sensitive paths

_Source: .github/workflows/ci.yml_

## Description

Surfaced by the ci-and-lints multi-model review (history/review-ci-and-lints.md, F15). CI runs Linux-only (ubuntu-latest), but orchestratectl is developed and primarily run on macOS and its supervisor/pid-liveness/tmux/unix-fd paths are platform-sensitive. A regression in a macOS-specific path passes Linux CI and only breaks on the dev machine. Add macos-latest to a build/test matrix (at least the test job). CI-scope expansion with cost/runner tradeoffs, hence its own issue.
