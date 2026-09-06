---
created: 2026-08-11
updated: 2026-08-15
type: chore
status: obsolete
priority: normal
closed: 2026-08-15
commits:
- hash: 6263c1e
  summary: DAG advanced after confirming workmux docs list pi as built-in agent
---

# workmux pi agent preset for --harness pi

## Description

run create --harness pi forwards --agent pi to create.sh -> workmux add -a pi. For that to launch pi, workmux must have a 'pi' agent configured in its .workmux.yaml (homebase/dotfiles concern, outside taskfleet). Without it, --harness pi will fail at workmux add. Add the pi agent preset (pi -p ... invocation) to the workmux config so --harness pi works end-to-end on Jari's box. Cross-repo: the config change lands in homebase, not taskfleet.
