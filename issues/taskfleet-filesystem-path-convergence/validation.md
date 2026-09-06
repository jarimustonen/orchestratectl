# Validation

Validated from canonical Taskfleet commit `2be1be0931c3796566e901024132ae5f5704d47e` on 2026-09-06.

## Host evidence

- **Gertrud:** clean canonical `main`; canonical Git common directory and worktree root; the preserved decision worktree remains registered, readable, and clean under that root. This smoke run was created from the canonical checkout and records the canonical source repository and worktree path.
- **Hauis:** clean canonical `main` at the same commit and canonical origin; recovered orphan content is preserved under the canonical worktree root and was not modified.
- **Haapa:** clean canonical `main` at the same commit and canonical origin. `intakectl.service` is active; `intakectl doctor --json` reported 9 ok, 1 expected missing caller-token warning, and 0 failures, including healthy database, queue, migration, `/healthz`, and `/readyz` checks.
- **Brunhild:** unreachable during convergence and therefore explicitly unverified. No convergence claim is inferred from stored fleet data.

## Identity and storage evidence

- `./scripts/check-canonical-identity.sh` passed with zero tracked path or text references.
- Gertrud, Hauis, and Haapa resolve configuration from `~/.taskfleet/config.toml`, state from `~/.taskfleet`, source from `~/Sources/taskfleet`, and the canonical GitHub remote.
- Active manifests, configuration, Git worktree registrations, launch paths, and service paths contain no retired filesystem identity. Retained pre-convergence state and checkout material is isolated as neutral, inactive safety archives outside active lookup roots.
- Tracked path and content scans returned zero retired identity references in every maintained owner repository: Taskfleet, Homebase, issuectl, Shipshape, project-canon, intakectl, 3DBear, blog, and Deutschpad.

## Focused checks

- `issuectl doctor --json`: no warnings or findings.
- `taskfleet doctor --json`: 57 ok, 1 expected installed-binary/source-commit warning, 0 failures; the current run manifest and supervisor are healthy.
- `git fsck --full --no-dangling`, canonical remote inspection, worktree registration inspection, and per-worktree `.git` verification passed.
- Post-rename create succeeded for run `01m1vxs01fapy4yc8dtz7e1b7a`; its successful `taskfleet run merge` is the closing half of this smoke.

No state, worktree, checkout, symlink, archive, installed tool, or external repository content was moved, deleted, or modified by this validation worker.
