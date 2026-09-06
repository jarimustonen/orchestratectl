---
created: 2026-06-12
updated: 2026-06-29
type: improvement
status: fixed
priority: normal
closed: 2026-06-29
---

# Escape control characters in human-text CLI output

## Description

Free-form fields (`topic`, `context`, `note`, `choice`, `severity`,
`title`) are printed raw in the human-text (`--format text`) output of
multiple subcommands. A field value containing `\n` or `\t` can spoof
additional columns or rows in tab-separated `list` output and break
field alignment in `show` output.

`--json` is the agent contract and is unaffected. But the text path
still needs an `escape_one_line` helper applied across:

- `discussion list`
- `discussion show`
- `discussion resolve` (choice, note)
- `run list` / `run show`
- `node list` / `node show`
- `event tail --format text`

Single helper in `taskfleet-cli/src/output.rs` consumed by every text printer.

Discovered during: discussion-cli review (history/review-discussion-cli.md F17).
