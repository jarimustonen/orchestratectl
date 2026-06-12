---
created: 2026-06-12
updated: 2026-06-12
type: feature
assignee: jari
status: in-progress
priority: normal
epic: orchestratectl-mvp
commits:
- hash: 6dd88a41f6347ad023383be9574a2d02df899d05
  summary: 'feat(event-tail-cli): event tail with --follow, signals, formats'
---

# Event tail CLI

## Description

orchestratectl event tail with --from-seq, --follow, separate --format=text|json|jsonl, and --output FILE for batch capture (poll-based tail; no inotify in MVP). Without --follow: emits terminal {"event":"result"} on natural EOF. With --follow: traps SIGINT (exit 130) / SIGTERM (exit 143) per AGENTS-AI-FIRST-CLI §12; emits final {"event":"cancelled"} on signal. **Depends on** state-schema-crate.
