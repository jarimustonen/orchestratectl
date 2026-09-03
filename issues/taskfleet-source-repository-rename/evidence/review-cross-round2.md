[thread_id:group_9d11aa80c37742eaa8576b70631fbe46]

## Model: gemini-3.1-pro-preview
[model:gemini-3.1-pro-preview] [thread_id:api_e42b5d75563d4947811111f749218108]

Based on the updated diff and the `cargo-dist` 0.28.2 source code, here is the reassessment of the R9 candidate.

### Resolution of Prior Findings

**1. Reusable Gate Caller Permissions and Secrets: RESOLVED**
The diff demonstrates that `secrets: inherit` was successfully removed, and the caller now correctly receives `permissions: { actions: write, contents: read }`. Crucially, this was achieved via the `github-custom-job-permissions` config in `dist-workspace.toml`, satisfying the ADR 0002 requirement to natively regenerate the workflow rather than hand-editing it. The shell validator and Rust test assertions were updated accordingly.

**2. Arbitrary Code Execution on Self-Hosted Runner: RESOLVED**
The `ci.yml` matrix now safely partitions macOS testing. The conditional logic ensures that fork pull requests run on `macos-latest` (`pr_scope: fork`), while trusted pushes and same-repository PRs run on the `[self-hosted, macOS, ARM64]` machine (`pr_scope: trusted`). This successfully eliminates the security regression of running untrusted fork code on the persistent release machine, while maintaining coverage for external contributors.

**3. Credentialed `dist host --steps=create` Mutation Risk: DEBUNKED**
The provided `host.rs` source from `cargo-dist` 0.28.2 proves that `HostingStyle::Github` is completely inert during the `create` step. The `compute_hosting` function only calculates JSON metadata (`artifact_base_url`, `artifact_download_path`, `owner`, `repo`) and explicitly notes: `// implemented in CI backend`. There are no network requests made to GitHub Releases during this step. Therefore, running this with a `GH_TOKEN` is perfectly safe and non-mutating.

**4. `host` Job Structural Bypass: CLASSIFIED AS ACCEPTED RISK**
Per your directive, the generated `host` job's permissive `if` condition (accepting skipped builds) is an acknowledged R7 workaround. Because manual YAML edits are forbidden by ADR 0002, and `cargo-dist` 0.28.2 cannot natively express a strict dependency on the custom gate, we rely on the asynchronous cancellation fallback. Given that the `custom-taskfleet-release-gate` takes seconds to execute and `plan` + build jobs take minutes, the practical likelihood of the `host` job starting before cancellation succeeds is functionally zero. This is an accepted toolchain limitation, not an R9 source blocker.

**5. Inert Tap Token Contract: RESOLVED**
`issues/taskfleet-distribution-topology/plan.md` has been amended to reflect the R10 deferral, harmonizing the prose with the JSON topology (`inert-blocked-r10`).

---

### Confirmed Source Blockers

**There are NO remaining source blockers.**

The source code, configurations, test fixtures, and generated distribution workflows are structurally sound, safe, and aligned with ADR 0002. The candidate is ready for CI.

### Pending Evidence / Candidate CI Checks
For R9 acceptance to complete, the following previously planned execution and evidence items must now be gathered:

1.  **Candidate CI Execution**: A successful `ci.yml` PR run proving the `self-hosted-macos-arm64` leg executes and passes for this trusted branch.
2.  **Gate Execution**: A successful `release.yml` PR run proving the `custom-taskfleet-release-gate` checks out correctly and succeeds in plan-only mode.
3.  **Runner Identity**: Evidence proving exactly one repository-visible runner matches the labels and architecture.
4.  **Rename Receipts**:
    *   Canonical after-state receipt proving repository ID `1265770191` continuity.
    *   Clone, fetch, push, API, PR, and check-operation receipts executed directly against `jarimustonen/taskfleet` (no redirects).
    *   Confirmation that the old source name was not recreated and acts only as a GitHub-managed redirect.
5.  **Final Main CI**: A fresh green exact-main push run after merge.

---

## Model: gpt-5.6-sol
[model:gpt-5.6-sol] [thread_id:api_69962a87549d49db8c4c7ac30d8e4284]

# Round 2 disposition

## Conclusions that change

- **Gate checkout permission is fixed.** `dist-workspace.toml` now generates both `actions: write` and `contents: read` for `custom-taskfleet-release-gate`. The previous checkout-permission blocker is resolved.
- **The original self-hosted-runner policy is conceptually fixed.** Fork PRs are intended to use `macos-latest`, while pushes and same-repository PRs use `[self-hosted, macOS, ARM64]`.
- **The Homebrew-token boundary is corrected.** The active plan now explicitly leaves the token inert during R9 and assigns installation/proof to R10.
- **Credentialed `dist host --steps=create` is not a GitHub mutation risk for this topology.** The supplied cargo-dist 0.28.2 implementation shows `HostingStyle::Github` only writes manifest download coordinates during `Create`; it does not call GitHub. Actual GitHub Release mutation remains in the generated `host` job. The prior “ungated planning” blocker is withdrawn.
- **The generated host/cancellation topology remains risky but is not a new R9 blocker under the accepted R7 contract.** On a blocked tag, cancellation is still the effective protection against cargo-dist’s skipped-build host condition. The gate should normally finish before cargo-dist installation/planning, but “normally” is not a hard safety property. Given that this exact workaround was explicitly accepted in R7, release remains blocked, and no tag exists, it should be recorded as accepted residual risk rather than repaired by hand-editing generated YAML in R9.

# Confirmed source blockers

## P0 — `matrix` is unavailable in the job-level `if`; the test job is invalid

**Path:** `.github/workflows/ci.yml:76-108`

The updated workflow uses:

```yaml
jobs:
  test:
    strategy:
      matrix:
        include:
          # ...
    if: >-
      matrix.pr_scope == 'all' ||
      ...
```

GitHub evaluates `jobs.<job_id>.if` before applying `strategy.matrix`. The `matrix` context is not available at that location. This is not merely a skipped-leg risk: the workflow expression can fail validation/evaluation with an unrecognized `matrix` context, preventing the test matrix from running.

The security policy is correct, but it must be implemented with separate jobs or conditions below the matrix expansion.

### Required fix

The least ambiguous solution is to split the jobs:

```yaml
test-linux:
  name: test (ubuntu-latest)
  runs-on: ubuntu-latest
  # common test steps

test-macos-arm64:
  name: test (self-hosted-macos-arm64)
  if: >-
    ${{
      github.event_name != 'pull_request' ||
      github.event.pull_request.head.repo.full_name == github.repository
    }}
  runs-on: [self-hosted, macOS, ARM64]
  # common test steps

test-macos-fork:
  name: test (macos-latest)
  if: >-
    ${{
      github.event_name == 'pull_request' &&
      github.event.pull_request.head.repo.full_name != github.repository
    }}
  runs-on: macos-latest
  # common test steps
```

Use a composite action for the repeated setup/test steps if duplication is unacceptable. Do not move the condition to individual steps while still allocating the self-hosted runner: that would avoid code execution but still schedule untrusted jobs on the persistent machine.

**Blocks candidate CI:** **Yes.** This is the one confirmed execution blocker that must be fixed before opening/running the candidate PR.

---

## P1 — `secrets: inherit` remains unnecessary and unsafe

**Paths:**

- `.github/workflows/release.yml:89-95`
- `.github/workflows/taskfleet-release-gate.yml`

Round 2 adds the missing permission but retains:

```yaml
custom-taskfleet-release-gate:
  uses: ./.github/workflows/taskfleet-release-gate.yml
  secrets: inherit
```

The gate uses `${{ github.token }}` and declares no legitimate need for repository secrets. Inheriting every repository secret gives same-repository PR-controlled reusable-workflow code access to unrelated credentials. This becomes especially dangerous when R10 installs the live Homebrew credential; it may already expose the crates.io token if that is repository-scoped.

This is not needed for cancellation or checkout.

### Required fix

Remove `secrets: inherit` through the supported cargo-dist/custom-job generation mechanism if available. Add a validator assertion that rejects it:

```bash
gate_block="$(grep -A10 '^  custom-taskfleet-release-gate:' \
  .github/workflows/release.yml)"

if grep -F 'secrets: inherit' <<<"$gate_block" >/dev/null; then
  echo "release gate must not inherit repository secrets" >&2
  exit 2
fi
```

If cargo-dist 0.28.2 cannot generate the safe form, do not maintain an undocumented manual edit. Either:

- add a deterministic, checked generation-hardening step whose result is enforced by `dist generate --check`; or
- move all publication credentials into protected environments unavailable to the PR gate before R10 activation.

**Blocks current candidate CI:** Not mechanically, assuming the candidate branch is trusted and current secrets remain inert.
**Blocks merge/R10 readiness:** **Yes.** It should be fixed in R9 rather than knowingly carrying broad secret inheritance into activation work.

---

## P1 — The R7 plan still contains stale `ready` instructions in its R9 substitution table

**Path:** `issues/taskfleet-distribution-topology/plan.md`, “Exact R9 substitutions”

Round 2 corrects the Homebrew credential row, but the previously supplied active table still says:

> set distribution trigger/activation to `tag-push`/`ready`

and:

> update both checks to require canonical tag-push, live least-privilege proof and ready state

Those rows still conflict with the actual and required R9 posture:

- tag trigger restored;
- release activation blocked;
- distribution prepared but blocked on R10;
- Homebrew token inert.

The added paragraph does not eliminate the contradictory instructions in the table immediately above it.

### Required fix

Change the rows to state explicitly:

```text
Post-R9:
- trigger = tag-push
- release activation remains blocked
- distribution activation = prepared-blocked-r10
- tap secret remains inert-blocked-r10
- R10 alone may install/prove the token and move activation toward ready
```

Also relabel the earlier “Current sealed topology” section as the frozen R7/pre-R9 posture if that heading remains unchanged.

**Blocks candidate CI:** No.
**Blocks R9 source acceptance:** Yes; active release instructions must not direct an operator to authorize publication during R9.

# Accepted residual risk, not a new source blocker

## Generated `host` still relies on cancellation after gate failure

**Paths:**

- `.github/workflows/release.yml`, `host.needs` and `host.if`
- `.github/workflows/taskfleet-release-gate.yml`

The structural concern remains real:

- gate failure skips local/global builds;
- `host.if` accepts skipped build results;
- `host` does not directly inspect gate success;
- run cancellation prevents publication under the accepted design.

Practical likelihood during R9 is low because:

1. no tag exists;
2. candidate PRs set `publishing == false`;
3. activation remains blocked;
4. the gate performs only checkout plus local verification;
5. plan installs cargo-dist and performs substantially more work before downstream scheduling;
6. cancellation is requested before the failed gate exits.

The risk is not zero if the cancellation API fails or scheduling behavior changes. However, this is the explicitly documented R7 workaround for cargo-dist 0.28.2, not a repository-identity regression introduced by R9. Hand-editing the generated workflow would violate the release-generation contract.

The validator should continue asserting all assumptions supporting this accepted workaround:

- gate has `actions: write`;
- gate runs for every tag event;
- gate invokes the canonical activation verifier;
- blocked activation invokes run cancellation;
- local artifacts depend on the gate;
- `host` has no path on PR planning;
- release authority remains non-`ready`.

A future cargo-dist upgrade should remove the cancellation dependency rather than perpetuate it.

# Pending verification and evidence, not source fixes

After fixing the invalid `ci.yml` expression, the following remain planned acceptance checks:

1. Candidate PR workflow parses and starts successfully.
2. All Linux CI jobs pass.
3. The same-repository candidate runs `test (self-hosted-macos-arm64)`.
4. The candidate does **not** run the hosted fork-only macOS leg, except as an explicitly skipped check.
5. A fork-path test or workflow inspection proves fork PRs use only `macos-latest`.
6. The release workflow PR run completes in non-publishing plan mode.
7. Required status-check configuration is updated for the new job names. In particular, a skipped `test (macos-latest)` must not accidentally satisfy the only required macOS check while the ARM64 job remains optional.
8. Runner receipts prove the selected self-hosted runner is ARM64 and repository-visible after the rename.
9. After-state API evidence proves repository ID `1265770191` now has `full_name = jarimustonen/taskfleet`.
10. Canonical clone, fetch, branch push, PR/check, and authenticated API operations avoid redirects.
11. Final exact-main push CI is green.
12. The old installer URL remains recorded as deferred to R10.
13. No tag, release, crate, formula, tap migration, or state mutation occurs.

# Ranked remaining source issues

1. **Invalid use of `matrix` in `jobs.test.if`** — confirmed candidate-CI blocker.
2. **Unnecessary `secrets: inherit` on the reusable release gate** — confirmed security defect; fix before merge and certainly before R10.
3. **Contradictory `ready` instructions left in the active R9 distribution plan** — confirmed contract defect; no CI impact, but blocks clean R9 acceptance.

The canonical identity substitutions, blocked activation, inert tap token, generated tag trigger, and GitHub `HostStyle::Create` behavior do not require further R9 source changes based on the supplied evidence.

---

## Model: claude-fable-5
[model:claude-fable-5] [thread_id:api_46e1eba0bebb4cb4bc52ee828981fe68]

# Round-2 reassessment

## Resolved by the round-2 diff and cargo-dist source inspection

- **Credentialed `dist host --steps=create` (prior top-3 #3 / gpt P0 / deepseek BLOCKER-1) — downgraded to residual, effectively closed.** `cargo-dist-0.28.2/src/host.rs` confirms the GitHub `HostingStyle` arm of `do_host`/`compute_hosting` is purely local: `HostStyle::Create` only populates `GithubHosting { artifact_base_url, artifact_download_path, owner, repo }` in the in-memory manifest; every network path (gazenot upload/release/announce, `create_artifact_sets`) is Axodotdev-only, which is not configured. `GH_TOKEN` in the `plan` env is unused for mutation on this path. The credential-less receipt plus source inspection together now cover the actual behavior. No fix needed; record the source-inspection finding as the receipt.
- **Gate caller permissions — fixed correctly.** `dist-workspace.toml` `github-custom-job-permissions = { "taskfleet-release-gate" = { actions = "write", contents = "read" } }` routes the fix through generation, not hand-editing, and the regenerated `release.yml` shows both scopes. gpt's P0-3 and gemini's blocker 3 are closed; my prior position (checkout of a public repo likely worked anyway) becomes moot. The `grep -A8` window in `validate-distribution-topology.sh` and the anywhere-in-file `contains("\"contents\": \"read\"")` in `package_graph.rs` are weak assertions (the string also can't be distinguished from a future unrelated block), but they fail closed against removal — acceptable.
- **Token deferral doc conflict — closed.** The plan table row and trailing paragraph now state R9 keeps the token inert and R10 owns installation/activation.
- **`host` skipped-build design — reclassified per instruction as accepted R7 workaround; likelihood assessment below.**

## CONFIRMED source blocker requiring a fix before candidate CI

### B1 — `matrix` context is not available in job-level `if:`; the entire `test` job will be skipped

**Path:** `.github/workflows/ci.yml`, `test` job, new `if:` block referencing `matrix.pr_scope`.

GitHub Actions evaluates `jobs.<job_id>.if` **before** matrix expansion; the available contexts there are `github`, `needs`, `vars`, `inputs` — not `matrix`. Unavailable context references evaluate to empty without error. Therefore:

- `matrix.pr_scope == 'all'` → `'' == 'all'` → false
- the `trusted` and `fork` clauses likewise false

→ the whole expression is false, once, for the job — and the **entire test job, including the ubuntu leg, silently skips on every event** (push and PR). Worse, skipped jobs satisfy branch protection, so the candidate PR could merge with zero test execution while appearing green. This is strictly worse than round 1's security exposure: it removes the test gate entirely and simultaneously fails the R9 acceptance requirement that the self-hosted macOS leg *executes*.

**Fix:** per-leg conditioning cannot be done at job level with matrix values. Split into jobs whose `if:` uses only the `github` context:

```yaml
  test-linux:
    name: test (ubuntu-latest)
    runs-on: ubuntu-latest
    ...

  test-macos-selfhosted:
    name: test (self-hosted-macos-arm64)
    runs-on: [self-hosted, macOS, ARM64]
    if: >-
      github.event_name != 'pull_request' ||
      github.event.pull_request.head.repo.full_name == github.repository
    ...

  test-macos-hosted:
    name: test (macos-latest)
    runs-on: macos-latest
    if: >-
      github.event_name == 'pull_request' &&
      github.event.pull_request.head.repo.full_name != github.repository
    ...
```

(or a setup job emitting a `fromJSON` dynamic matrix, which is more machinery for no benefit here). Deduplicate steps via a composite action or accept the repetition. Then re-verify required-check names: the set becomes `test (ubuntu-latest)` / `test (self-hosted-macos-arm64)` / `test (macos-latest)`, where the last two are mutually exclusive per event — required-check configuration must tolerate the skipped one (GitHub treats skipped Actions jobs as satisfying required checks, which is acceptable here but should be receipted deliberately).

The *intent* of the round-2 change (trusted-only self-hosted, hosted fallback for forks) is correct and answers round 1's N2/P1; only the mechanism is broken.

## Residual risks — classified, no source fix demanded

### R1 — `host` reachable after gate failure: low practical likelihood, one documented failure chain

Accepting the caller's framing (R7-accepted workaround, no hand-edit of generated YAML): the realistic bad path requires **both** (a) an unauthorized/accidental version-tag push — currently impossible without push access, and no tag exists — and (b) the gate's cancel to not take effect before `host` schedules. On (b), note the precise single point: the gate step runs under `bash -e`, so in

```sh
if ! ./scripts/verify-release-activation.sh; then
  gh api --method POST ".../cancel"
  exit 2
fi
```

a transient `gh api` failure (5xx, rate limit, token hiccup) aborts the step *before* `exit 2` is meaningless — the job still fails, builds skip, and `host`'s skipped-accepting `if:` becomes true with no cancellation in flight. Even then, the blast radius is a GitHub Release object on an already-burned tag: crates.io is independently fail-closed (`publish-crates.yml` re-runs the verifier), the tap token is inert, and the tag ref itself was created by the push, not the workflow. Classification: **accepted residual, low likelihood, bounded blast radius.** Compensating controls to receipt (settings, not source): a tag ruleset restricting `v*` tag creation (deepseek HIGH-1 — still the cheapest control, and it also protects against version burn), and an R10 hardening item to add an in-`host` activation re-check if cargo-dist's custom-job config ever permits it. Make `gh api ... || true` inside the gate so a cancel-API failure cannot suppress the cancel *and* is still followed by `exit 2` — that is a one-line change in the **checked-in** `taskfleet-release-gate.yml`, not generated YAML, so it violates nothing. Recommended, though not a candidate-CI blocker.

### R2 — `secrets: inherit` on the gate call remains

Round 2 did not remove it, and cargo-dist 0.28.2 emits `secrets: inherit` for custom plan-jobs itself, so removal would be a hand-edit. The exposure is real but narrow: same-repository PR authors can modify the checked-in `taskfleet-release-gate.yml` and receive inherited secrets — which today includes a presumably **live `CARGO_REGISTRY_TOKEN`** (fork PRs get no secrets). For a single-maintainer repository this is low likelihood, but it is live-credential surface. Non-source mitigation available now: move `CARGO_REGISTRY_TOKEN` into a protected Actions *environment* bound to the publish jobs in `publish-crates.yml`; environment secrets are not exposed through `secrets: inherit` to jobs that don't declare the environment. Classification: **documented residual with a recommended settings mitigation before R10 installs the live tap token.** Not a candidate-CI blocker.

### R3 — stale sentence in the active plan

`issues/taskfleet-distribution-topology/plan.md` line ~39 still reads "the public source repository is still `jarimustonen/orchestratectl`" and the heading remains "Current sealed topology"; round 2 amended only the token row and closing paragraph. False operational prose in an active document; fix with a one-line relabel ("R7 sealed topology (pre-R9)") in the same doc pass. Non-blocking.

## Unchanged pending evidence (not source defects)

After-state repository receipt (ID `1265770191` continuity), redirect-independent clone/fetch/push/API receipts, candidate PR CI with the self-hosted ARM64 job actually executing (now contingent on B1 being fixed first — with B1 in place the run would falsely "pass" by skipping), required-check name reconciliation, runner-label uniqueness receipt for the generic `macOS` cargo-dist selector, final exact-main green push run, tag-ruleset receipt, and the deferred-not-passed record for the old latest-installer URL.

## Bottom line

Exactly **one confirmed source blocker** remains: **B1**, the `matrix`-in-job-`if` defect in `.github/workflows/ci.yml`, which disables the entire test job on every event and would let the candidate merge untested. It must be fixed before the candidate PR is opened. The recommended `gh api ... || true` hardening in the checked-in gate workflow (R1) is a cheap, non-generated one-liner worth bundling. Everything else is residual-risk documentation, repository settings (tag ruleset, secret environment scoping, required checks), or the already-planned pending receipts.

---

## Model: deepseek-v4-pro
[model:deepseek-v4-pro] [thread_id:api_a6cd68a62356403898b0c93711503268]

## Confirmed source blocker before candidate CI

### BLOCKER: `ci.yml` uses `matrix.*` inside a job-level `if`

**Path:** `.github/workflows/ci.yml`, `test` job

```yaml
test:
  strategy:
    matrix:
      include:
        - os: ubuntu-latest
          pr_scope: all
        - os: self-hosted-macos-arm64
          pr_scope: trusted
        - os: macos-latest
          pr_scope: fork
  if: >-
    matrix.pr_scope == 'all' ||
    (matrix.pr_scope == 'trusted' &&
      (github.event_name != 'pull_request' ||
       github.event.pull_request.head.repo.full_name == github.repository)) ||
    (matrix.pr_scope == 'fork' && github.event_name == 'pull_request' &&
     github.event.pull_request.head.repo.full_name != github.repository)
  runs-on: ${{ matrix.runner }}
```

**Problem:** GitHub Actions does not provide the `matrix` context in `jobs.<job_id>.if`. Matrix expansion happens after the job-level condition. Referencing `matrix.pr_scope` here makes the expression invalid or evaluates every leg to false, so the required test jobs—including the self-hosted ARM64 acceptance leg—will not run.

This would directly contradict the R9 acceptance criterion that all CI jobs execute in the renamed repository before merge.

**Fix:** Split into separate jobs rather than filtering a shared matrix in a job-level `if`.

Do not rely on matrix filtering at job level; do not put the self-hosted runner in a matrix that can be scheduled for fork PRs before steps are skipped.

Minimal shape:

```yaml
test-linux:
  runs-on: ubuntu-latest
  steps: [ ...shared test steps... ]

test-macos-self-hosted:
  if: >-
    github.event_name == 'push' ||
    github.event.pull_request.head.repo.full_name == github.repository
  runs-on: [self-hosted, macOS, ARM64]
  steps: [ ...shared test steps... ]

test-macos-fork:
  if: >-
    github.event_name == 'pull_request' &&
    github.event.pull_request.head.repo.full_name != github.repository
  runs-on: macos-latest
  steps: [ ...shared test steps... ]
```

Use a composite action or a local reusable workflow for the shared steps.

**Candidate CI:** **Blocked.** Fix before running the R9 candidate.

---

## Previously claimed blockers now resolved or not candidate blockers

### `plan` running `dist host --steps=create` is not a mutation vector

**Path:** `cargo-dist-0.28.2/src/host.rs`, `compute_hosting`

Source inspection confirms that for `HostingStyle::Github`, the create step only populates download URLs and GitHub hosting metadata in the local manifest. It performs no network call and creates no remote GitHub Release. Actual release creation remains in the generated workflow’s separate `host` job via `dist host --steps=upload --steps=release` and `gh release create`.

The credentialed `host --steps=create` concern was overbroad. The `GH_TOKEN` in `plan` is unnecessary, but not a blocker for candidate CI.

### Gate caller now has `contents: read`

**Paths:**
- `.github/workflows/release.yml`, `custom-taskfleet-release-gate`
- `dist-workspace.toml`, `github-custom-job-permissions`

The generated caller adds:

```yaml
permissions:
  "actions": "write"
  "contents": "read"
```

The validator and `package_graph.rs` assert the new permission. This resolves the checkout failure.

### Self-hosted fork exposure fix is directionally correct but blocked by the matrix bug

The intended `pr_scope: trusted` versus `pr_scope: fork` split is correct. The self-hosted leg should only run for pushes and same-repository PRs. The implementation placement is invalid; fix per the blocker above.

### R7 plan now defers the tap token properly

The active plan now says R9 keeps the `HOMEBREW_TAP_TOKEN` inert and R10 installs/proves the live token. This aligns with the R9 task boundary.

---

## Accepted residual risks: not candidate CI blockers, but must close before R10

### 1. `host` publication still relies on run cancellation after a failed gate

`host` does not directly depend on the gate and still accepts skipped build jobs. On a blocked tag, publication can only be stopped by the gate’s asynchronous cancel request.

This remains the already accepted R7 workaround. Practical likelihood of the race is low here: the gate is much faster than the much larger `plan`/build path, and `host` cannot evaluate until `plan` finishes. But it is not a structural fail-closed boundary.

Do not hand-edit generated cargo-dist YAML to enforce this. Before R10 removes the block, close the boundary through tag creation controls, held-tag wrapper behavior, and activation-gate verification.

### 2. `secrets: inherit` remains on the reusable release gate

The gate inherits all repository secrets but needs only `github.token`. With the stated inert tap token and no release authorization, this is latent rather than an active R9 candidate blocker.

Before R10 installs any live credential, remove `secrets: inherit` or prove that no inherited secret is live. A same-repository branch can modify the local reusable workflow, so a live inherited secret becomes an exfiltration path even without fork-PR access.

### 3. Workflow-wide `contents: write` remains

Generated cargo-dist output grants `contents: write` at workflow level. This exceeds least privilege for `plan` and build jobs. However, changing it would violate the ADR’s no-hand-edit rule for generated release identity. Record it as an accepted generated-tool posture, not an R9 source blocker.

---

## Pending evidence, not source blockers

These still block authoritative R9 closure, but they are already planned verification/evidence steps and do not require another source edit before candidate CI:

1. Candidate PR CI on `jarimustonen/taskfleet`.
2. Fresh green exact-main push CI, including the trusted self-hosted ARM64 leg.
3. Post-rename repository receipt with repository-ID continuity `1265770191`.
4. Canonical clone/fetch/push/API operations without redirect dependence.
5. Required-check/ruleset update after any test job name change.
6. Evidence that the release selector `macOS` resolves to exactly the ARM64 runner.
7. Old latest-installer URL recorded as deferred, not passed.

---

## Bottom line

One confirmed source blocker remains before candidate CI: the invalid job-level `matrix.*` condition in `ci.yml`. Fix that by splitting trusted self-hosted and hosted fork macOS jobs into separate jobs.

The release workflow’s gate-bypass race is a real release-safety gap inherited from the accepted R7 design, but it is not a candidate CI blocker while release activation remains blocked and no tag exists. It must be closed, together with `secrets: inherit`, before R10 installs live credentials or enables release activation.
