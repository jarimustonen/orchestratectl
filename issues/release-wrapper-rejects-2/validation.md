# ossctl 0.10.0 release-protocol validation

Validated against installed `ossctl 0.10.0` commit
`a35b9917fc65a6354fe855b7c956521b47669907`. All mutating exercises used a
throwaway clone, isolated HOME, and a local bare `origin`; no GitHub release tag,
registry publication, or global installation was touched.

## Observed command surfaces

- `release plan --bump minor --json` remains non-mutating and emitted schema 1,
  a content-addressed `data.plan_id`, `data.bump` (`0.4.1 -> 0.5.0`), the exact
  intra-workspace pin rewrite, and the seven planned phases.
- 0.10 adds a load-bearing cut input: a bump-owned plan must be executed with the
  matching `release cut --plan <id> --bump <level> --json`. Omitting `--bump`
  fails before the bump. The sealed plan is stored at
  `<git-common-dir>/ossctl/plans/<plan-id>.json`; the wrapper now reads only its
  validated `plan.bump.level`, then ossctl independently rechecks the plan seal.
- A real isolated cut completed bump, dry-run, build, and delegated publish, made
  the bump commit and annotated local tag, then the temporary pre-push hook
  rejected the local-bare-remote tag push. Exit was `release_failed`; no remote
  tag existed.
- `release list --json` reported exactly one matching `in_flight` run and no
  unreadable journals. `release show --json` reported journal schema 5,
  `status=in_progress`, `current_phase=null`, phases
  `bump/dry_run/build/publish=ok, tag=failed`, one local tag with
  `created_local=true`, `pushed_remote=false`, and both GitHub flags false.
  `last_seq == applied_seq == 25`; the last three events remained
  `phase_entered(tag)`, `tag_created_local`, `phase_completed(tag, failed)`.
- In the local-only fixture, raw `release resume` retried tag, pushed it only to
  the local bare remote, recorded `tag_pushed_remote`, delegated the GitHub
  Release to cargo-dist, completed tag and dist, and entered verify. This confirms
  that a bare resume before the wrapper's CI gate would still cross the
  irreversible boundary in production.
- `release verify` is still a read-only remote reconciliation. The exact 0.10.0
  source documents an authoritative event-log read with no journal/repository/
  registry writes and maps unavailable observations to `unknown`, not `missing`.
  The isolated resume was deliberately terminated while remote observations were
  pending; its journal stayed at `current_phase=verify`, demonstrating that the
  operation does not invent completion on unavailable destinations.

## Wrapper conclusions

The 0.10 held-tag journal and marker coordinates match the strict checkpoint
assertion. The wrapper admits only the validated 0.10.0 commit, rejects every
other version/build, extracts 0.10's sealed
bump input, and keeps all existing journal/event/tag/checkpoint/exact-main-SHA
checks. The two abandoned v0.5.0 run IDs are explicitly denied by `resume` before
`release show`.

Automated coverage is split into fast stripped-PATH near-miss tests and
`scripts/test-ossctl-release-0.10-protocol.sh`, which uses isolated HOME,
Cargo home, temp directory, and local remote with the real installed engine. It
proves seal tampering is rejected before a journal, reaches the held checkpoint,
asserts the exact-SHA CI query and required `--bump` argv, and proves the release
tag is absent remotely. It intentionally stops at the fake exact-SHA CI lookup
and never resumes the tag.
