<!--
Thanks for contributing to orchestratectl! Please read CONTRIBUTING.md first.
Keep PRs focused — one concern per commit, one issue per PR.
-->

## What & why

<!-- What does this change and why? Link the issue it addresses. -->

Closes: <!-- issues/<slug> or #<gh-issue>, or "n/a (typo fix)" -->

## Green gate

All must pass locally before review (CI enforces them):

- [ ] `cargo fmt --check`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] `cargo doc --workspace --no-deps` (warnings are errors in CI)

## Checklist

- [ ] Filed or referenced an `issuectl` issue for anything beyond a typo fix
      (see [CONTRIBUTING.md](../CONTRIBUTING.md)).
- [ ] New commands/flags follow the AI-first conventions in
      [`AGENTS-AI-FIRST-CLI.md`](../AGENTS-AI-FIRST-CLI.md) — `--json` output,
      JSONL logs, strict input validation, informative error envelopes, no
      interactive prompts.
- [ ] CLI-surface or `SKILL.template.md` changes: re-deployed locally
      (`cargo install --path crates/octl-cli --force && orchestratectl skill install --force && orchestratectl doctor`)
      and the insta snapshots are updated.
- [ ] Commits are focused (one concern each).

## Notes for reviewers

<!-- Anything worth flagging: trade-offs, follow-ups, areas wanting extra eyes. -->
