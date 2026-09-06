---
created: 2026-07-13
updated: 2026-07-31
type: task
status: done
priority: normal
related: ['@workmux-extract-libs']
closed: 2026-07-31
---

# Vendor workmux's tmux multiplexer code into taskfleet (raine declined the crate split)

_Source: workmux (raine/workmux) src/multiplexer/_

## Description

raine declined splitting workmux into lib crates (see @workmux-extract-libs) and suggested duplicating the multiplexer code instead. Vendor the minimal tmux slice of src/multiplexer/ (kill_window / new_session(headless) / window-lookup) into an taskfleet-local module so the supervisor makes typed calls instead of shelling out. NOT the full kitty/wezterm/zellij abstraction; git side stays on create.sh for now.
