# Contributing to taskfleet

Issues and pull requests are welcome. The repo is small and the
contribution loop is short; please
read this file before opening either.

## Before you file an issue

- Run `taskfleet version` and `taskfleet doctor` and include
  the output. `doctor` reports `63 ok / 0 fail` on a clean install — any
  non-zero `fail` count is worth surfacing.
- Check `TODO.md` — the active pre-publication campaign is tracked there
  and many known gaps already have an issue under `issues/<slug>/`.
- For workflow questions ("how do I do X with `/worktree-*`"), open a
  GitHub Discussion instead of an issue.

## Before you open a pull request

- The repo uses [`issuectl`](https://github.com/jarimustonen/issuectl) to
  track every non-trivial change. For anything more than a typo fix,
  please file an issue first (or reference an existing one) so the
  decision conversation has a home.
- All code must pass:
  ```bash
  cargo fmt --check
  cargo clippy --all-targets -- -D warnings
  cargo test --workspace
  cargo doc --workspace --no-deps           # warnings are errors in CI
  ```
- New commands or flags must follow the AI-first conventions documented
  in `/ai-first-cli-canon` — `--json` output,
  JSONL logs, strict input validation, informative error envelopes, no
  interactive prompts. New SKILL.md examples are CI-gated against the
  actual binary CLI surface.
- Keep commits focused. One concern per commit; one issue per PR.

## Repo layout

- `crates/taskfleet-core/` — schema, file I/O, locking, supervisor protocol.
  Library crate; new public items get rustdoc (the crate carries
  `#![warn(missing_docs)]`).
- `crates/taskfleet/` — the `taskfleet` binary and the bundled
  SKILL.md files (`crates/taskfleet/skills/`). Validate CLI-surface edits with a
  repository-local build and explicit `./target/release/taskfleet …`
  invocations. For `SKILL.template.md` edits, run and review the insta snapshot
  loop documented in `crates/taskfleet/AGENTS.md`; no install is needed.
- `issues/<slug>/item.md` — every issue + epic (flat layout, no
  numeric prefix, status in frontmatter).
- `/ai-first-cli-canon` — externally maintained CLI design canon; treat the
  installed skill as canonical, not a project-local copy.

## License

By contributing, you agree your contributions will be licensed under the
[MIT License](LICENSE).
