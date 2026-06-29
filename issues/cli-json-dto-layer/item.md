---
created: 2026-06-12
updated: 2026-06-29
type: improvement
status: fixed
priority: normal
closed: 2026-06-29
---

# Introduce DTO layer for CLI --json payloads

## Description

`discussion show --json` (and `run show`, future `node show`) serializes
the on-disk projection struct (`Discussion`, `Manifest`, `Node`) directly
into the success envelope's `data` field. The wire contract is therefore
permanently coupled to the disk schema — every internal schema change
(adding `last_applied_seq`, splitting fields, renaming for clarity)
leaks straight into the public JSON API.

Land a thin DTO layer per noun:

```rust
#[derive(Serialize)]
struct DiscussionView<'a> { ... }
impl<'a> From<&'a Discussion> for DiscussionView<'a> { ... }
```

Lets the disk schema evolve while keeping the AI-first CLI contract
stable.

Discovered during: discussion-cli review (history/review-discussion-cli.md F18).
