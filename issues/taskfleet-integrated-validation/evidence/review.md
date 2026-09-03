# R8 evidence review synthesis

Reviewed with `/llm-review` defaults, two cross-review rounds, plus the required one-shot context-request follow-up. Exact reviewers:

- `gemini-3.1-pro-preview`
- `gpt-5.6-sol`
- `claude-fable-5`
- `deepseek-v4-pro`

Raw independent reviews, context revisions, and both cross-rounds are preserved as `review-round1.md`, `review-context-followups.md`, `review-cross-round1.md`, and `review-cross-round2.md`.

## Final consensus

Three reviewers ended at **PASS after mechanical finalization** or its equivalent. OpenAI and DeepSeek used **FIXABLE EVIDENCE GAPS** while the corrected env-isolated Cargo rerun and final residue files were still pending; both explicitly said R8 can pass after those complete. No reviewer retained a verified product defect. The final exact-SHA source remains unchanged; every accepted change is evidence harness/reporting only.

## Confirmed findings fixed

1. **Machine authority was too weak.** `verify-evidence-index.py` now enforces the complete required-ID set, per-command allowed outcomes, exact tested commit/tree, terminal acceptance matrix, four unique reviewed model identities, strict commit-bound residue fields, explicit exception dispositions, Homebrew identity consistency, output existence, artifact bytes, and permanently false release authorization. Normal verify mode works after finalization.
2. **Harness bytes were not indexed.** The index now includes `validation.md`, all four verification/sanitization scripts, and every evidence/log artifact.
3. **Raw outputs were hash-only.** Complete sanitized logs for the main gates are committed; unsanitized source-log hashes remain recorded as transformation provenance.
4. **Homebrew checks had weak isolation/assertions.** The final harness pins exact Homebrew version and git commit before and after update, uses a run-private cache, checks formula substitutions and archive digest fail-closed, catches broken symlinks, validates canonical receipt ownership and old-name JSON resolution, upgrades through the fully qualified old identity, uninstalls, fresh-installs, and leaves an empty Cellar. Its corrected run passed.
5. **Install channels were incomplete.** The final harness verifies pre-recorded artifact hashes, uses private HOME/CARGO_HOME/target, invokes the direct immutable Rust toolchain under `env -i`, checks canonical Cargo and bounded legacy Cargo roots separately, checks all executable names and exact old-command archive members/symlinks, and validates the generated shell installer. Its corrected run passed.
6. **CLI inventory was not exhaustive enough.** The final harness derives 33 visible paths from structured help; compares full trees after normalizing only each `command` field; invokes all 33 under both names with forced-invalid stdout, filtered-stderr, exit, and exactly-one-warning comparison; retains seven representative valid text/JSON/JSONL comparisons plus help and hidden-child checks. The bounded interpretation is explicit: all-path parser/surface parity plus shared dispatcher and representative valid behavior, not 33 unsafe valid mutations.
7. **Mandatory-state traceability was weak.** `acceptance-matrix.json` maps terminal, active, pending merge, removed-kind/unknown, config, provenance, rollback, refusal, and byte-preservation criteria to named tests/artifacts.
8. **The ignored stress-test wording was false.** It now states accurately that the source-marked expensive test is not a required R8 leg and was not executed separately.
9. **Homebrew/stripped failures were conflated with acceptance.** Every superseded setup failure is classified separately; only clean bounded successful runs count.
10. **Stale issue/DAG evidence conflicted with final Homebrew facts.** Mistaken uncommitted intake `nominally-numberless-hand` was dispositioned obsolete during diagnosis, omitted from the final R8-only candidate, and the DAG was refreshed.
11. **Private artifact scanning was missing.** `sanitization-report.json` records a fail-closed scan of the final indexed payload for token/header/private-path patterns.

## Findings assessed as incorrect or non-blocking

- **Generated formula cannot install due String/Symbol mismatch — incorrect.** Ruby quoted-label syntax creates a Symbol key; three reviewers corrected OpenAI, and the exact formula installed twice in the bounded Homebrew run.
- **Local 0.6.0 formula relabel violates production generation — incorrect.** The rewrite is an exact-one, digest-checked, local-only receipt/upgrade simulation. It is not committed production release identity and grants no R10/R11 authority.
- **Ignored flock stress test blocks — incorrect.** It is source-marked expensive and outside the documented R8/CI gate. No claim of separate execution remains.
- **Native-only artifact install leaves Linux unvalidated — non-blocking boundary.** Local execution is macOS ARM64; cargo-dist plan/check proves configured Linux musl targets; R10 owns hosted cross-platform artifacts.
- **Nextest `LEAK` marker proves product leak — unsupported.** The marker moved between two process-free tests under load; every assertion passed, nextest exited zero, and the ordinary exact-SHA run had no marker. Final process residue is corroboration, not retroactive proof. It remains a disclosed warning.
- **Final `xcrun` warning invalidates isolation — non-blocking disclosed warning.** The final stripped environment retained an SDK lookup ENOENT but still built every release test binary and passed all 1,115 tests. The report no longer calls this corrected.
- **Exploratory real-log write silently waived — false characterization.** The incident is explicit, complete restoration is not claimed, and the task owner's direction distinguishes failed exploratory diagnostics from clean bounded authoritative gate evidence. `exceptions.json` records that disposition.

## Residual limitations accepted for R8

- Valid success/refusal fixtures are not executed for every mutating command because doing so would create unnecessary side effects; complete parser/surface parity, one shared dispatcher, representative valid outputs, and the release integration suite are the bounded R8 proof.
- Sanitized logs are transformed evidence; unsanitized original hashes are retained, but raw machine-private files are intentionally not committed.
- The stripped-path delayed-exit and SDK warnings are accepted only as disclosed warnings; they are not rewritten as clean outcomes.
- The exploratory dispatch-log incident changed mtimes and has no pre-probe digest. It is excluded from gate evidence.

## Model-performance record

Per-model scores are retained in `model-performance-assessment.json`. The normal `/assess-models` remote corpus append was attempted once and failed closed because haapa's dedicated corpus worktree filesystem was out of space; `model-performance-log.json` preserves the actionable failure. No repository or machine state was changed locally by that failed append.

## Final verdict

**PASS after successful final residue, assessment, sanitization, and steady-state index verification.** This authorizes only ADR 0002 R9's source-repository rename. It does not authorize release, tagging, publishing, installing, tap activation, or R10/R11 work.
