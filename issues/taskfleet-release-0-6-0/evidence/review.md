# R10 Phase A release-safety review

Reviewers: `gemini-3.1-pro-preview`, `gpt-5.6-sol`, `claude-fable-5`, and `deepseek-v4-pro`. Two adversarial rounds plus one bounded context follow-up reviewed the generated cargo-dist workflow, authorization and wrapper scripts, ledgers, fixtures, CI, ADR, and issue requirements.

## Confirmed findings fixed

1. **Bash conditional suppressed authorization failures.** The first reusable-gate draft called a multi-command function from `if`, where Bash ignores `errexit`. That entire design was deleted. The final standalone verifier uses explicit `|| exit 1` checks and executable negative fixtures.
2. **The reusable gate could fail and skip builds into cargo-dist's permissive host condition.** The reusable workflow and plan-job dependency were deleted. Exact cargo-dist 0.28.2 now generates no reusable call or `secrets: inherit`; authorization executes in every non-empty local matrix job, where rejection is a failed dependency that host does not admit.
3. **PR-controlled release code inherited secrets.** Generator-supported `pr-run-mode = "skip"` removes the PR trigger. The tag-only generated workflow contains no reusable secret inheritance. Ordinary CI owns PR topology testing.
4. **Wrapper provenance was initially only exact-main CI.** The held wrapper now creates an atomic, version-scoped authorization ref only after exact-main CI and a second exact-main check. Both release workflows bind the peeled tag commit to that ref. Active remote rulesets restrict all tags and authorization refs to the repository-administrator bypass boundary and prevent ordinary update/deletion/force-push.
5. **Intermediate and active ledger validation conflicted.** Distribution validation now has exact `prepared` and `active` modes. CI derives the mode from one of two admitted complete ledger tuples and rejects every mixed transition.
6. **GitHub create-ref 404 handling was incorrect.** `gh api` writes a JSON error body on stdout. The wrapper now branches on command status, then uses GitHub's atomic create-ref endpoint and validates the exact returned ref/SHA.
7. **Live-main checking in tag jobs could burn an authorized version.** Tag jobs now verify the durable authorization coordinate and peeled commit. The wrapper, not a queued post-tag job, owns the exact-main and green-CI checks.
8. **Unsafe-topology fixture was initially dead code.** The same checker now runs against current output and deliberately unsafe PR-triggered and secret-inheriting variants. The executable verifier fixture covers repository ID, event/ref/tag/version, activation, live policy, missing ref, and mismatched SHA.
9. **Release runner prerequisites and cargo-dist selection were weak.** The verifier preflights `gh`, `jq`, `git`, `awk`, and `cargo`; the fixture resolves declared tools explicitly. CI downloads the exact Linux cargo-dist archive, verifies its SHA-256, extracts to an empty private directory, requires one executable, and checks version 0.28.2 before generation/plan.
10. **Ruleset IDs alone were self-attestation.** A live verifier checks exact rule IDs, names, targets, active enforcement, include/exclude sets, exact administrator bypass actor, and creation/update/deletion/non-fast-forward rules. It runs before wrapper authorization and in both tag workflows.
11. **The self-hosted exact-main proof was missing.** The ARM64 macOS job remains same-repository only and now runs for both same-repository PRs and main pushes.
12. **The Homebrew credential remained pending.** Homebase commit `ac96cacd5e7659dba4138915668eb6e1a0645b4c` synchronized the SOPS-managed token, proved a reversible exact-head temporary-ref canary, and left both taps unchanged. Taskfleet independently verified only the secret name/timestamp and tap receipts before atomically activating the ledgers.

## Incorrect or bounded findings

- Claims that cargo-dist 0.28.2 `host --steps=create` mutates GitHub during planning were rejected against exact source and the existing credential-free before/after receipt. Public mutation occurs in the later generated host job.
- The generated workflow's workflow-wide `contents: write` is an upstream 0.28.2 limitation. It is bounded by tag-only execution, all-tag rules, no PR secret path, authorization before build code, and separate tap credentials; no generator-supported job-level replacement exists.
- The generated host still permits skipped local builds. For the admitted plan this route is unreachable: topology validation requires one Taskfleet release and a non-null three-target, container-free local matrix; each local job runs authorization and rejection yields `failure`, not `skipped`. This exact plan assumption is load-bearing and tested in CI.
- The protected authorization ref is not cryptographic proof against the repository administrator. The administrator is the explicit trust boundary and the sole ruleset bypass role. Once the ref exists it authorizes one tag/commit coordinate; an interrupted resume must reconcile only the same Shipshape journal. No source claim treats model agreement or ref existence as independent human confirmation.
- Rechecking live main after the tag is incorrect for this resumable saga because a benign later main push would permanently block the already-pushed immutable version.

## Verdict

The final source topology is structurally fail-closed within the declared repository-administrator boundary and removes both mandatory R9 residuals. Activation remains valid only while the live rulesets and SOPS-managed tap credential receipts match the recorded state. Phase C remains conductor-owned after merged exact-main CI; no review result authorizes a tag or publication in this worker.
