# ossctl 0.10.1 release-protocol validation

Validated against the fleet-installed `/opt/homebrew/bin/ossctl` 0.10.1 build at
commit `6879e040a520a7a9c6196ed77791b4f2f10ad6f4`. Mutating exercises use an
isolated HOME and Cargo home, a throwaway clone, and a local bare `origin`; they
do not install tools, push a GitHub tag, or publish a registry artifact.

## Compatibility evidence

- `version --json` retains the schema-1 outer and inner envelopes and reports the
  exact release commit. Admission is keyed by the version/commit pair, not by a
  SemVer range.
- `release plan/cut/show/list/resume/verify` retain their schema-1 CLI surfaces.
  A bump plan is still content-addressed, and `cut` accepts the matching
  `--bump` recovered from that sealed plan. ossctl independently rejects a plan
  whose sealed bump level was edited.
- The isolated real-engine cut still creates the bump commit and one local
  annotated version tag, then reaches the wrapper's pre-push hook. The hook's
  six coordinates identify a new `refs/tags/v…` push to `origin`; its rejection
  leaves the remote tag absent.
- `release list --json` identifies exactly one in-flight run for the sealed
  plan. `release show --json` retains journal schema 5 with
  `bump/dry_run/build/publish=ok`, `tag=failed`, no current phase, one local but
  unpushed tag, neither GitHub-release flag set, an applied sequence equal to
  the last sequence, and the exact final event triplet `phase_entered(tag)`,
  `tag_created_local`, `phase_completed(tag, failed)`.
- The bump commit remains the local tag target and a descendant of the sealed
  main commit. The wrapper advances and pushes only `main`, with follow-tags
  disabled, then requests `ci.yml` for branch `main`, event `push`, and that
  exact bump SHA. It rechecks journal, checkpoint, repository identity, and
  remote-tag absence before `release resume`.
- 0.10.1 changes fresh-plan sealing and delegated-destination observation, but
  does not change the held pre-tag journal protocol. Resume still crosses the
  irreversible boundary by pushing the already-created tag; verify remains the
  engine-owned reconciliation surface. The isolated test exercises their JSON
  and journal transitions against controlled local/stub destinations.

## Fail-closed conclusions

The wrapper continues to admit the previously validated 0.10.0 build and now
also admits only the exact fleet 0.10.1 build above. It rejects future versions,
same-version rebuilds, malformed version envelopes, abandoned run ids, absent
or mismatched wrapper checkpoints, changed tag coordinates, remote tag presence,
non-descendant bumps, journal phase/event near-misses, repository mismatches,
and CI runs that do not attest the exact main SHA. No version range or protocol
shape is inferred from the `0.10` prefix.
