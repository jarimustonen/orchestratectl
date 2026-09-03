---
created: 2026-09-04
updated: 2026-09-04
type: task
reporter: jari
status: open
priority: high
related: ['@rename-taskfleet']
lane: taskfleet-rename
lane_seq: 100
collision: [repository-identity]
blocked_by: ['@taskfleet-integrated-validation']
---

# Rename Taskfleet source repository

## Goal

Execute ADR 0002 R9 after immutable R8 authorization: rename the GitHub source repository from `jarimustonen/orchestratectl` to `jarimustonen/taskfleet`, immediately converge maintained source identity without relying on redirects, and prove canonical repository operations plus CI/runner continuity.

## Preconditions

- R8 evidence commit `488d6cab7fc8ca883f7c660a695097441cf9c407` verifies 69 artifacts and authorizes only R9.
- Tested production SHA is `c3ef8b740ac531f12ce81c759ed209d178cf36bd`; CI run `33764612111` is green.
- Revalidate immediately before mutation that the old repository has the expected immutable identity, the canonical repository name is unoccupied, and no release/tag/tap/publication action is in flight.

## Required execution

- Capture sanitized before-state: repository ID/name/default branch/visibility, rules/settings, Actions status, secret names only, runner visibility, remotes, and relevant exact-URL inventory.
- Perform the one-way GitHub repository rename to `jarimustonen/taskfleet`. Do not recreate the old repository and do not routinely roll back the rename; fix forward.
- Immediately change local/common remotes and all maintained exact source URLs, GitHub Actions references, badges, repository metadata, release-wrapper identity checks, cargo-dist/source links, and operator documentation to canonical `jarimustonen/taskfleet` where ADR/R0 classification assigns ownership to R9.
- Preserve intentional legacy compatibility, historical evidence, protocol IDs, `OCTL_*`, old-tap migration references, and bounded wrapper identities. No blind replacement.
- Verify canonical clone, fetch, branch push, PR/check operation, and authenticated repository API calls without redirect dependence.
- Trigger candidate CI in the renamed repository so the self-hosted macOS runner and all Linux jobs execute. Review every job and snapshot change. The final merged exact-main SHA must receive a fresh green `ci.yml` push run before R9 is accepted.
- Record immutable before/after evidence and exact IDs/SHAs/run URLs under this issue. Scan maintained surfaces for unintended old source identity and classify every residual.

## Acceptance criteria

- GitHub canonical source is exactly `jarimustonen/taskfleet`; the old name exists only as GitHub-managed redirect and is not recreated.
- Maintained remotes and source-owned links use the canonical URL without depending on redirect behavior.
- Canonical clone/fetch/push and GitHub API operations pass.
- A renamed-repository candidate run proves the self-hosted macOS runner and all CI jobs; final exact-main CI is green before closure is considered authoritative.
- Evidence clearly distinguishes intentional legacy/protocol/history residuals.
- No crate publication, release tag, GitHub Release, Homebrew tap/formula activation, global install, skill install, real state migration, or dependent-repository edit occurs.
- R10/release remain blocked; R9 does not authorize publication.

## Recovery

After the GitHub rename, fix forward. If a local/code/CI check fails, preserve the canonical repository identity, record the failure, keep this issue open, and repair through a focused follow-up. Never recreate `jarimustonen/orchestratectl`.
