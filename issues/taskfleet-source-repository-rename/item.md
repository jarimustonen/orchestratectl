---
created: 2026-09-04
updated: 2026-09-04
type: task
reporter: jari
status: done
priority: high
related: ['@rename-taskfleet']
lane: taskfleet-rename
lane_seq: 100
collision: [repository-identity]
blocked_by: ['@taskfleet-integrated-validation']
commits:
- hash: 076f983c498de1ca2fc8fe0b919130ffbd52dc27
  summary: converge canonical Taskfleet repository identity
closed: 2026-09-04
closed_by: orchestrator
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

## Acceptance Criteria

- [x] GitHub canonical source is exactly `jarimustonen/taskfleet`; the old name exists only as GitHub-managed redirect and is not recreated.
- [x] Maintained remotes and source-owned links use the canonical URL without depending on redirect behavior.
- [x] Canonical clone/fetch/push and GitHub API operations pass.
- [x] A renamed-repository candidate run proves the self-hosted macOS runner and all CI jobs; final exact-main CI is green before closure is considered authoritative.
- [x] Evidence clearly distinguishes intentional legacy/protocol/history residuals.
- [x] No crate publication, release tag, GitHub Release, Homebrew tap/formula activation, global install, skill install, real state migration, or dependent-repository edit occurs.
- [x] R10/release remain blocked; R9 does not authorize publication.

## Recovery

After the GitHub rename, fix forward. If a local/code/CI check fails, preserve the canonical repository identity, record the failure, keep this issue open, and repair through a focused follow-up. Never recreate `jarimustonen/orchestratectl`.

## Agent Runs

### 2026-09-03T22:53:15Z · @orchestratectl:01m1mm0cm54yt1sppxz260ywp6

ADR 0002 R9 candidate/pre-main legs passed. GitHub source repository ID `1265770191` (`R_kgDOS3Iezw`) was renamed one-way from `jarimustonen/orchestratectl` to `jarimustonen/taskfleet`; canonical remotes, API, SSH clone/fetch and reversible candidate-branch push all work without redirect dependence. Identity candidate `076f983c498de1ca2fc8fe0b919130ffbd52dc27` (tree `06aaf232a85833ac1762e7a2fcf89b38cf9e6572`) passed local green gates and renamed-repository PR CI run `33814447787`, including runner ID 21 with labels `self-hosted`, `macOS`, `ARM64`. Non-publishing cargo-dist PR plan/gate run `33814447929` also passed; all host/publish jobs were skipped. Temporary PR #1 was closed unmerged and its remote branch deleted.

No tag, crate, GitHub Release, formula, tap ref, secret value, installed binary/skill, or state was changed. Release remains blocked (`release/taskfleet-release.json: blocked-r8-r9-r10`; distribution `prepared-blocked-r10`); R10 owns live tap credentials and release activation. Review residuals requiring R10 re-evaluation are cargo-dist 0.28.2's generated host cancellation dependency and generated `secrets: inherit` before any live credentials/activation.

Conductor finalization checklist (do not close before all pass): (1) merge only through `taskfleet run merge`; (2) verify canonical `origin/main` equals the exact merged SHA; (3) wait for a fresh `ci.yml` push run whose `headSha` is that exact merged SHA and whose every job succeeds, including `test (self-hosted-macos-arm64)` on runner ID 21; (4) recheck repository ID/name, canonical remotes, tag/release/tap/secret-name invariants, and residual classifier; (5) record final merged commit and exact-main CI on this issue, then close it. This worker deliberately leaves the issue open.

## Resolution

### 2026-09-03T23:07:20Z · @orchestrator

R9 conductor finalization passed. Canonical origin/main is exact merged SHA `5df8359d092bcb10c26441e988617895151a12a7`. Renamed-repository main push CI `33815467669` completed success: docs, ubuntu/macOS tests, version snapshots, clippy, MSRV, rustfmt and cargo-deny all succeeded. The R9-only self-hosted probe is intentionally same-repository-PR-gated and therefore skipped on push; it already executed successfully on the exact identity candidate in PR CI `33814447787` using runner ID 21 (`self-hosted`, `macOS`, `ARM64`). Repository ID remains `R_kgDOS3Iezw`, canonical name/remote are `jarimustonen/taskfleet`, and canonical API/SSH operations pass. Tags/releases, both tap heads (`db12bb1` canonical empty-prepared; `85ce830` legacy), and source secret-name set remain unchanged. R9 evidence index (44 artifacts), sanitization scan and residual classifier pass. R9 is complete; this authorizes no release action. R10 must run the post-R9 exact-SHA release gate before any tag, publication or tap activation.
