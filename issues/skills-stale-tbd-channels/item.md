---
created: 2026-08-17
updated: 2026-08-18
type: improvement
reporter: jari
status: done
priority: normal
lane: skills
lane_seq: 60
closed: 2026-08-18
---

# Bundled skills still claim publishing channels are TBD

_Source: crates/octl-cli/skills_

## Description

Five bundled SKILL templates (octl-spawn-spinoff, orchestratectl-overview, octl-run-overview, worktree-merge, worktree-spinoff) carry the line '(Publishing channels are TBD; the placeholders above mirror issuectl conventions and will be replaced once the release pipeline ships.)' in their Install-or-upgrade sections. The pipeline shipped long ago: the Homebrew tap (jarimustonen/orchestratectl), crates.io, and the cargo-dist shell installer have all been live since the 0.1.x/0.2.x releases and the commands above the parenthetical are the real ones. Remove the stale parenthetical from all five templates (grep 'Publishing channels are TBD' under crates/octl-cli/skills/). Same public-artifact-staleness class as the README rewrite done 2026-08-17. Needs the insta snapshot loop + skill install --force redeploy after the edit.

## Resolution

### 2026-08-18T05:41:21Z · @issuectl

Removed the stale publishing-channel disclaimer from all five bundled templates and verified the live Homebrew, crates.io, and cargo-dist release artifacts.
