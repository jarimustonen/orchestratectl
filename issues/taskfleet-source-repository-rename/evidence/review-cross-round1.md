[thread_id:group_9d11aa80c37742eaa8576b70631fbe46]

## Model: gemini-3.1-pro-preview
[model:gemini-3.1-pro-preview] [thread_id:api_e42b5d75563d4947811111f749218108]

Here is a critical cross-review of the other models' assessments, followed by the top three remaining source blockers.

### Critical Cross-Review of Other Reviewers

**1. Valid and severe findings caught by the panel:**
*   **Self-Hosted Runner PR Exposure (gpt-5.6-sol & claude-fable-5):** Both correctly identified that changing `ci.yml` to run unconditionally on `[self-hosted, macOS, ARM64]` for all `pull_request` events is a massive security regression. Executing arbitrary fork-PR code on a persistent self-hosted runner compromises the machine prior to cargo-dist release builds.
*   **Release Gate Structural Bypass (gpt-5.6-sol & deepseek-v4-pro):** Both correctly proved that the generated `host` job accepts skipped dependencies via `always()` and does not strictly require `custom-taskfleet-release-gate`. The gate relies entirely on an asynchronous API cancellation request, creating a race condition where `host` could execute and publish before the workflow cancels.
*   **Broken Gate Permissions (gpt-5.6-sol):** Correctly identified that the `custom-taskfleet-release-gate` caller in `release.yml` only grants `actions: write`. The called workflow uses `actions/checkout@v4`, which will fail without `contents: read`.

**2. Where the reviewers failed or conflicted with the ADR boundary:**
*   **Hand-editing generated files:** All three reviewers demand manual YAML edits to `.github/workflows/release.yml` to fix the permissions, the `host` dependency graph, and the `secrets: inherit` issues. **This directly violates ADR 0002**, which mandates: *"Regenerate its workflow; do not hand-edit generated release identity."* If `cargo-dist 0.28.2` generates an unsafe job graph or broken permissions, the fix must be an automated post-generation patch script (e.g., `sed` or `yq`), not manual drift. The reviewers failed to respect this constraint.
*   **Ignoring explicit prompt context (deepseek-v4-pro):** DeepSeek listed the inert Homebrew token as a "HIGH" finding that blocks acceptance, explicitly ignoring your instruction that *"The canonical tap secret remains inert until R10."*
*   **False positives on `cargo-dist` planning (All):** The reviewers flag `dist host --steps=create` in the `plan` job as a blocker because it uses `GH_TOKEN`. While passing a token to an ungated job is risky, cargo-dist's `host --steps=create` does not mutate GitHub Releases during a plan; it merely generates the manifest. This is an upstream cargo-dist behavior, not a novel R9 regression.

---

### Top 3 Remaining Source Blockers

These require source changes **before** candidate CI is run, as they either introduce active security vulnerabilities or structurally break the release pipeline.

#### 1. Arbitrary Code Execution on Self-Hosted Runner (ci.yml)
**Path:** `.github/workflows/ci.yml:85-88`
**Why it blocks:** The updated `test` matrix maps all PRs, including malicious fork PRs, to `[self-hosted, macOS, ARM64]`. This gives untrusted code access to the persistent machine used for actual release builds.
**Required Source Fix:** Restrict the self-hosted runner to `push` events or same-repository PRs. Provide a safe `macos-latest` fallback for external fork PRs to maintain coverage without exposure.
```yaml
# Example fix for ci.yml strategy inclusion:
- os: self-hosted-macos-arm64
  runner: ${{ (github.event_name == 'push' || github.event.pull_request.head.repo.full_name == github.repository) && '[\"self-hosted\", \"macOS\", \"ARM64\"]' || 'macos-latest' }}
```

#### 2. Structural Bypass of the Release Gate (release.yml)
**Path:** `.github/workflows/release.yml` (generated)
**Why it blocks:** The generated `host` job does not `need` the `custom-taskfleet-release-gate`, and its `if: always() ...` logic accepts skipped builds. An API cancellation request from the gate is a race condition, not a dependency boundary. A failed gate could still result in a published release.
**Required Source Fix:** Because ADR 0002 forbids hand-editing, you must write a determinist post-generation patch script (to be run after `cargo dist generate`) that injects `custom-taskfleet-release-gate` into the `host` job's `needs` array and adds a strict requirement for `needs.custom-taskfleet-release-gate.result == 'success'`.

#### 3. Reusable Gate Lacks `contents: read` Permission (release.yml)
**Path:** `.github/workflows/release.yml:91-96`
**Why it blocks:** `cargo-dist` generated the custom gate caller with only `permissions: actions: write`. The called workflow (`taskfleet-release-gate.yml`) invokes `actions/checkout@v4`, which will fail with HTTP 403. The gate will crash before it can either authorize or cancel the release.
**Required Source Fix:** Similar to Blocker 2, add a deterministic post-generation patch script to inject `contents: read` into the `custom-taskfleet-release-gate` caller block in `release.yml`. Ensure `scripts/validate-distribution-topology.sh` is updated to assert the presence of both permissions rather than just `actions: write`.

---

## Model: gpt-5.6-sol
[model:gpt-5.6-sol] [thread_id:api_69962a87549d49db8c4c7ac30d8e4284]

# Cross-review verdict

## Gemini assessment

### Wrong: “ledger state corruption”

**Claim:** `release/taskfleet-distribution.json.source_repository.current` must remain `jarimustonen/orchestratectl`.

This is unsupported and conflicts with the field’s operational use. `current` is the active source repository consumed by release validation; after the one-way rename it must be:

```json
"current": "jarimustonen/taskfleet"
```

Keeping the old value would make active topology depend on the redirect and cause `verify-release-activation.sh` to reject the canonical repository later.

The before-state is already preserved in:

- `issues/taskfleet-source-repository-rename/evidence/before-gates.json`
- historical R7 receipts
- repository ID/node-ID continuity evidence

A mutable topology file is not a cryptographically signed migration ledger. Reverting `current` would be a regression.

### Wrong as a blocker: `publish-crates.yml` must use the self-hosted runner

**Path:** `.github/workflows/publish-crates.yml`, test matrix

The ADR requires the renamed-repository candidate CI and cargo-dist macOS artifact build to prove self-hosted ARM64 continuity. It does not require the independent crates.io workflow’s test matrix to use that runner.

Keeping `macos-latest` there provides an independent hosted test environment and avoids making crates.io publication depend unnecessarily on the self-hosted runner. The release artifact path is the relevant path that must use the ARM64 machine.

The divergence should be documented, but changing `publish-crates.yml` is not required for R9.

### Valid: R9/R10 token prose conflict

The active R7 plan still says R9 installs and proves a live least-privilege Homebrew token, while the controlling task boundary explicitly defers that to R10.

The implementation is correct for the clarified boundary:

```json
"tap_secret_state": "inert-blocked-r10"
```

The documentation is stale and should be corrected. Do **not** install a live token in R9 merely to satisfy stale prose.

**Classification:** source documentation fix before merge, but not a candidate-CI execution blocker.

---

## Claude assessment

### Valid and blocking: credentialed `host --steps=create` runs outside the gate

**Path:** `.github/workflows/release.yml`, `plan`

On a tag push, `plan` and the activation gate run concurrently. `plan` executes:

```sh
dist host --steps=create --tag=...
```

with `GH_TOKEN`, before activation has succeeded.

The existing receipt proves only the uncredentialed case. It does not prove that the actual credentialed tag path is non-mutating.

The correct fix is structural: gate tag planning before this command. A disposable-repository experiment is useful validation, but it is not an adequate substitute for enforcing job ordering.

### Valid and blocking: `host` can run after gate failure

**Path:** `.github/workflows/release.yml`, `host.needs` and `host.if`

This is the strongest issue found by multiple reviewers.

A failed gate skips both build jobs, but `host` deliberately accepts skipped builds:

```yaml
needs.build-global-artifacts.result == 'skipped'
needs.build-local-artifacts.result == 'skipped'
```

The cancellation API call is therefore the only thing preventing the hosting job from starting. Cancellation is asynchronous defense in depth, not a fail-closed release boundary.

`host` must directly depend on the gate and explicitly require its success.

### Valid and blocking: untrusted PR code on the persistent self-hosted runner

**Path:** `.github/workflows/ci.yml`, `test` matrix

Every pull request currently executes workspace build scripts and tests on:

```yaml
[self-hosted, macOS, ARM64]
```

That exposes the persistent machine to arbitrary fork-PR code. This is especially unacceptable when the same runner class later builds release artifacts.

Repository approval settings reduce accidental execution but do not remove the underlying risk. The source workflow should restrict the self-hosted leg to pushes and trusted same-repository PRs, while retaining hosted macOS coverage for fork PRs.

### Valid verification item, not a source blocker: required-check name changed

The check changes from:

```text
test (macos-latest)
```

to:

```text
test (self-hosted-macos-arm64)
```

Rulesets and required-check configuration must be inspected and updated. This is a GitHub settings/evidence step, not necessarily a source defect. It can block PR acceptance if the old check remains required.

### Valid but minor: stale dispatch branches

**Path:** `.github/workflows/taskfleet-release-gate.yml`

The `workflow_dispatch`/`dry-run` branch is now dead for the sole caller. It is misleading but fail-safe. Remove it or explicitly document that it supports a future caller.

### Valid but minor: implicit Rust availability

The release gate invokes `cargo metadata` without installing a toolchain. Hosted Ubuntu currently supplies Cargo, and absence fails closed. An explicit pinned toolchain would make the gate reproducible and diagnostics clearer, but this is not an R9 blocker.

### Informational only: hosted macOS in `publish-crates.yml`

As above, this is not a missed R9 requirement. Hosted testing in the crates.io workflow is defensible.

---

## DeepSeek assessment

### Valid and blocking: ungated credentialed planning

Its `BLOCKER-1` is correct for the same reason as Claude’s N1.

### Valid and blocking: hosting accepts skipped gated builds

Its `BLOCKER-2` is correct. The current validator does not prove that the actual publication job requires gate success.

### Wrong as an R9 blocker: absence of a tag ruleset

A tag ruleset would be useful defense, but the sealed plan explicitly records that none exists and bases safety on the held-tag wrapper plus activation gates. R9 intentionally restores generated tag dispatch.

The claim that any stray version tag permanently consumes a canonical version is also overstated. A tag ref is not an immutable crates.io version and can be deleted if nothing was published. The serious risk is that the unsafe workflow graph might publish from that tag—which must be fixed independently.

Tag-protection configuration is recommended operational hardening, not a newly discovered source blocker under the accepted task boundary.

### Valid documentation conflict, wrong prescribed direction: Homebrew token

The stale plan says R9 installs a live least-privilege token. The clarified task says R10 owns that action. The fix is to amend the plan, not activate the token during R9.

### Partly valid future issue: activation verifier does not validate token readiness

Before R10 changes activation to `ready`, `verify-release-activation.sh` should require an explicit live/verified tap-token state. Otherwise GitHub Release and crates publication could succeed before Homebrew predictably fails.

However, requiring `old_tap.activation != blocked-r11` would be wrong. ADR 0002 deliberately places old-tap migration activation after the canonical formula is live and verified. R11 cannot be a prerequisite for initially publishing that canonical formula.

**Classification:** R10 source requirement, not an R9 blocker.

### Correct: old installer URL remains deferred

The URL cannot resolve to a future canonical artifact before R10. It must remain explicitly deferred rather than falsely marked as passing.

---

# Top three remaining source blockers

## 1. Make release publication structurally require gate success

**Paths:**

- `.github/workflows/release.yml`, `host`
- `scripts/validate-distribution-topology.sh`
- `crates/taskfleet/tests/package_graph.rs`

Required shape:

```yaml
host:
  needs:
    - plan
    - custom-taskfleet-release-gate
    - build-local-artifacts
    - build-global-artifacts
  if: >-
    ${{
      always()
      && needs.custom-taskfleet-release-gate.result == 'success'
      && needs.plan.outputs.publishing == 'true'
      && needs.build-local-artifacts.result == 'success'
      && needs.build-global-artifacts.result == 'success'
    }}
```

The exact skipped-build logic may need to account for cargo-dist-supported topologies, but the gate result must never be skipped, failed, or cancelled.

The current API cancellation should remain only as defense in depth.

The validator must inspect the `host` dependency and condition rather than count occurrences of the gate name.

**Blocks candidate CI?** It may not make `ci.yml` red, but it blocks merging the candidate.

---

## 2. Gate credentialed tag planning before `dist host --steps=create`

**Paths:**

- `.github/workflows/release.yml`, `plan`
- `.github/workflows/taskfleet-release-gate.yml`

The tag path must not run authenticated hosting preparation concurrently with a blocked activation check.

Restructure so the activation check precedes `plan` for tag events while still allowing PR planning. For example, make the gate return success for PRs and make `plan` depend on it:

```yaml
plan:
  needs:
    - custom-taskfleet-release-gate
```

This may require adjusting the generated/custom-job integration to avoid a dependency cycle.

Also reduce the default token permission:

```yaml
permissions:
  contents: read
```

and grant `contents: write` only to the actual hosting job.

**Blocks candidate CI?** Not necessarily the ordinary CI jobs, but it blocks merge because the restored tag trigger is not currently behind the complete gate.

---

## 3. Prevent fork PR execution on the persistent self-hosted ARM64 runner

**Path:** `.github/workflows/ci.yml`, `test`

Split hosted and self-hosted coverage. The self-hosted job should run only for pushes or trusted same-repository PRs:

```yaml
if: >-
  ${{
    github.event_name == 'push' ||
    github.event.pull_request.head.repo.full_name == github.repository
  }}
```

Keep a `macos-latest` job for fork PRs so external changes still receive macOS coverage.

The R9 candidate should use a same-repository branch, ensuring its self-hosted ARM64 acceptance job still executes before merge.

**Blocks candidate CI?** Yes, as a workflow-security source fix. The eventual candidate must still exercise the trusted self-hosted leg.

---

# Additional source fixes before the candidate run

## Fix the reusable-gate call contract

**Paths:**

- `.github/workflows/release.yml`, `custom-taskfleet-release-gate`
- `.github/workflows/taskfleet-release-gate.yml`

The caller grants only `actions: write`, while the called workflow checks out repository contents. Grant:

```yaml
permissions:
  actions: write
  contents: read
```

Remove:

```yaml
secrets: inherit
```

The gate does not need inherited repository secrets. This also avoids exposing future live release secrets to candidate-controlled reusable-workflow code.

This can directly break the `release.yml` PR run if checkout cannot authenticate with the caller’s effective permissions, so it should be fixed before candidate CI.

## Correct the R7 plan’s post-R9 state

**Path:** `issues/taskfleet-distribution-topology/plan.md`

The post-R9/pre-R10 contract should state:

- canonical tag trigger restored;
- both release paths remain fail-closed;
- canonical tap token remains inert;
- activation is not `ready`;
- R10 owns live token installation and release activation.

Also relabel “Current sealed topology” as the historical R7 pre-R9 topology. Do not delete the substitution history.

## Replace substring validation with semantic workflow checks

**Paths:**

- `scripts/validate-distribution-topology.sh`
- `crates/taskfleet/tests/package_graph.rs`

Current checks can pass while `host` bypasses gate success. Parse the YAML job graph or enforce exact generated fragments for:

- gate permissions;
- no `secrets: inherit`;
- plan gating;
- host direct dependency on the gate;
- host requiring gate success;
- write permissions limited to mutation jobs;
- Homebrew secret access only after successful hosting.

# Planned verification, not source blockers

These are correctly pending until the source is settled:

1. Candidate PR CI in `jarimustonen/taskfleet`.
2. Every Linux job succeeding.
3. Self-hosted macOS ARM64 job executing on the intended runner.
4. `release.yml` PR planning/gate run succeeding.
5. Required-check/ruleset convergence after the job-name change.
6. Post-rename receipt proving repository ID `1265770191` continuity.
7. Canonical clone, fetch, push, PR/check, and API operations without redirect dependence.
8. Final exact-main push CI after merge.
9. Old source name verified as a GitHub redirect only.
10. Old latest-installer URL recorded as deferred until R10.

These evidence items block R9 closure, but they do not justify running candidate CI before the source-level release and runner defects above are fixed.

---

## Model: claude-fable-5
[model:claude-fable-5] [thread_id:api_46e1eba0bebb4cb4bc52ee828981fe68]

# Cross-review of the other three revised assessments

(My own section is the claude-fable-5 text; excluded.)

## gemini-3.1-pro-preview

**Finding 1 (ledger corruption in `source_repository.current`) — wrong, and its fix would be harmful.** `current` is a statement of current fact, not a frozen before-record. The rename has actually occurred (repository ID `1265770191` continuity is receipted in `before-gates.json`, which is the durable before-state). `verify-release-activation.sh` deliberately requires `current == "jarimustonen/taskfleet"` so that activation is impossible unless R9 completed — that is the fail-closed design. Reverting `current` to the old name would (a) make the ledger false against live GitHub state and (b) hard-wire the activation gate to a pre-rename identity, permanently blocking R10 or forcing a second edit of the mutating boundary. "Cryptographically signed ledger" is fabricated; nothing signs these JSONs. Reject.

**Finding 2 (`publish-crates.yml` still on `macos-latest`) — valid observation, wrong severity and dubious fix.** This is a real divergence (matches my N6), but "release executes in an unproven environment" overstates it: hosted `macos-latest` is the environment every prior release's tests ran in, and the publish workflow runs only on trusted tag pushes. Whether to bind it to the self-hosted runner or deliberately keep the release path independent of that machine is a design decision needing one documented line, not a blocker.

**Finding 3 (token gap) — valid, doc-only.** Correct that the R7 plan's R9 token row is now unfulfillable as written; the fix is amending `issues/taskfleet-distribution-topology/plan.md`, not installing a token.

**Overall:** Gemini missed every workflow-graph problem (`host` gating, ungated credentialed `host --steps=create`, `secrets: inherit`, fork-PR self-hosted exposure) and its one "confirmed" blocker is wrong. Weakest of the three.

## gpt-5.6-sol

**P0 — `host` can run after gate failure (`always()` + skipped-accepting condition, cancellation as sole barrier) — valid and the most important source finding.** Independently confirmed by deepseek BLOCKER-2 and consistent with my N1. The gate's `gh api .../cancel` is an asynchronous best-effort request racing job scheduling; if it loses, `host` runs with `contents: write` and executes `dist host --steps=upload --steps=release` plus `gh release create`. The grep validator (`grep -Fc ... -ge 2`) proves string presence, not that `host` is gated — gpt's P2 point that the validator is too textual is proven by exactly this hole. One caveat all reviewers underweight: `release.yml` is generated and must survive `dist generate --check`. The fix must go through cargo-dist's custom-job configuration (test whether 0.28.2's job hooks can attach the gate to `host`'s `needs`) or, failing that, an in-job re-run of `./scripts/verify-release-activation.sh` at the top of `host` injected the same way the gate job itself was injected — plus a validator assertion on the actual `host.needs`/`host.if`.

**P0 — ungated credentialed `dist host --steps=create` in `plan` — valid.** Matches my N1 first half and deepseek BLOCKER-1. The only receipt (`host-create-no-mutation.json`) explicitly proves `credentials: {GH_TOKEN:false, GITHUB_TOKEN:false}`; it does not cover the path the workflow actually runs. gpt's preferred fix (`plan: needs: custom-taskfleet-release-gate`) is likely not expressible through cargo-dist generation for the plan job; the realistic mitigations are an activation check as the first step of `plan` (if injectable) or a credentialed disposable-repo no-mutation receipt. Valid blocker either way.

**P0 — gate caller omits `contents: read` so checkout fails — plausible but unproven; overstated as P0.** Job-level `permissions:` on the caller zeroes unspecified scopes for the called workflow, correct. But `actions/checkout` of a **public** repository generally succeeds without a contents-scoped token (public git read requires no authorization). gpt asserts failure as fact without evidence. Even if it does fail, the failure mode is the gate failing → fail-closed (modulo the `host` hole above). Correct disposition: add `contents: read` anyway (one line, removes the ambiguity), and treat the candidate PR run as the empirical test. Not an independent P0.

**P1 — `secrets: inherit` on the gate call — valid, and a genuine new find the rest of us missed.** The gate uses only `github.token`; inheritance is unnecessary. Because `release.yml` runs on `pull_request` and a same-repo branch controls the referenced local reusable workflow, a same-repo PR could rewrite `taskfleet-release-gate.yml` to exfiltrate inherited secrets (fork PRs don't get secrets, so exposure is same-repo actors — reduced but nonzero, and it becomes serious the moment R10 installs a live `HOMEBREW_TAP_TOKEN` or if `CARGO_REGISTRY_TOKEN` is a repo secret). Remove `secrets: inherit`; verify removal survives regeneration; add a validator assertion. Should land before candidate CI since it's one line in the same gate-hardening edit.

**P1 — workflow-wide `contents: write` — valid least-privilege point, but the fix conflicts with the ADR.** cargo-dist emits the workflow-level `permissions: contents: write` itself; ADR §6 forbids hand-editing generated release identity, and cargo-dist 0.28.2 has no per-job permission narrowing config. This is an accepted generated-tool posture until a cargo-dist upgrade; record it, don't hand-patch. Downgrade to documented accepted risk.

**P1 — fork-PR code on self-hosted runner — valid** (matches my N2). gpt's fix is better than mine: keep `macos-latest` for fork PRs, run the self-hosted leg only for `push`/same-repo PRs, preserving macOS coverage for outside contributions. Adopt that shape.

**P1 — generic `macOS` label — valid concern, wrong disposition.** The R7 plan explicitly accepted the generic label "unless a unique Taskfleet label is provisioned," contingent on the pending runner receipts proving exactly one visible ARM64 runner carries it. That's an evidence item for the pending after-receipts, not a source change gpt can demand now. A dedicated label is a reasonable R10 improvement.

**P2s (YAML-parsing validator, plan.md "Current sealed topology" phrasing, activation label divergence) — all valid, correctly ranked.** The plan.md point is fair: the residual scan's `issue-history-evidence` classification of an *active* plan's "is still `jarimustonen/orchestratectl`" sentence is generous; the section needs a frozen-R7-snapshot heading plus a post-R9 status block.

**Overall:** Strongest review. Two genuinely new valid finds (`secrets: inherit`; validator textual weakness proven by the `host` hole). One overstated P0 (gate checkout permissions) and one ADR-conflicting fix (workflow permissions).

## deepseek-v4-pro

**BLOCKER-1 / BLOCKER-2 — valid**, duplicate gpt's two real P0s with correct analysis, including the correct reading that gate-failure → build skipped → global skipped → `host.if` true. Correctly scoped as merge/acceptance blockers, not candidate-CI-red.

**HIGH-1 (tag-push restored with no tag ruleset) — valid and the one finding that materially sharpens my earlier note.** With dispatch-only/dry-run gone, a stray `v*` tag now (a) creates a permanent ref, (b) triggers both `release.yml` and `publish-crates.yml`, and (c) under ADR §7/"never retag/reuse", **consumes a canonical version forever even though both workflows fail closed**. The activation gates protect publication, not tag-ref creation or version burn. A tag ruleset restricting tag creation to the release wrapper's actor is a repository-settings change, not source, but it should be applied and receipted before or with the candidate run. Correctly ranked HIGH; "blocks safe closure" is defensible.

**HIGH-2 (R7 plan token row) — valid but over-severe.** Same as gpt's finding 3 / my F1-F7: the resolution is a plan amendment deferring token install to R10, which the caller has already authorized. Calling it an acceptance blocker "as written" is technically true but resolved by the doc edit, not a posture change.

**MEDIUM-1 (activation verifier ignores `tap_secret_state`/`old_tap.activation`) — valid and a useful R10-boundary hardening no one else raised.** Cheap fix now: `verify-release-activation.sh` should refuse `activation == ready` while `tap_secret_state` still matches `inert-*`. One jq clause; prevents R10 from flipping activation with a dead Homebrew leg.

**MEDIUM-2 — correct.**

**Gaps:** deepseek entirely missed `secrets: inherit`, the fork-PR self-hosted exposure (the most operationally urgent pre-candidate item), and the required-status-check rename. Its workflow-graph analysis is solid; its CI-surface analysis is incomplete.

## Consensus conflicts resolved

- **`source_repository.current`:** gemini vs. everyone else — gemini is wrong; keep the diff as-is.
- **Tag-push restoration itself:** all reviewers accept it as intentional per the caller constraint; only the compensating controls (host gating, plan credential, tag ruleset) are contested — correctly.
- **Token inertness:** all converge on doc amendment, not token install. Correct.

## Top three remaining source blockers (must land before the candidate CI run)

1. **`host` is not structurally gated** (`.github/workflows/release.yml` `host.needs`/`host.if`): a failed activation gate leaves publication reachable through skipped-build acceptance, protected only by a racy cancel API call. Fix via cargo-dist custom-job wiring or an injected in-job `verify-release-activation.sh` at the head of `host`; add a real (YAML-parsed) validator assertion on `host`'s dependency and condition. (gpt P0-2 / deepseek BLOCKER-2.)
2. **Unconditional fork-PR execution on the persistent self-hosted macOS ARM64 runner** (`.github/workflows/ci.yml` test matrix): restrict the self-hosted leg to `push`/same-repo PRs and retain `macos-latest` for fork PRs; receipt the Actions approval setting. The exposure goes live the moment the candidate PR opens, so this precedes candidate CI. (my N2 / gpt P1.)
3. **Ungated credentialed `dist host --steps=create` in `plan` on tag push** (`release.yml` plan step + `GH_TOKEN` + workflow `contents: write`): either gate/strip it or produce a credentialed disposable-repo no-mutation receipt; the existing receipt explicitly does not cover this path. (gpt P0-3 / deepseek BLOCKER-1 / my N1.)

Bundle into the same gate-hardening commit: remove `secrets: inherit`, add `contents: read` to the gate caller's permissions, and add deepseek's MEDIUM-1 inert-token guard to `verify-release-activation.sh`. Apply the tag ruleset (deepseek HIGH-1) as a receipted repository-settings step alongside the candidate run. Everything else — plan.md rephrasing, activation-label convention, publish-crates macOS comment, gate dead branches, required-check names, runner-label uniqueness — is doc/evidence work or pending receipts, not source blockers.

---

## Model: deepseek-v4-pro
[model:deepseek-v4-pro] [thread_id:api_a6cd68a62356403898b0c93711503268]

## Cross-review verdict

The other reviews converge on two real release-safety holes and add one immediate candidate-CI break. Most source identity substitutions, cargo-dist regeneration, residual classification, and blocked activation labels are correct. The Gemini "ledger corruption" claim is wrong.

---

## Valid source blockers

### 1. Reusable release gate cannot check out the repository

**Paths:**
- `.github/workflows/release.yml:91-96` (`custom-taskfleet-release-gate` caller)
- `.github/workflows/taskfleet-release-gate.yml:4-17`

The caller grants only:

```yaml
permissions:
  "actions": "write"
```

The called gate workflow needs and declares:

```yaml
permissions:
  actions: write
  contents: read
```

and immediately runs `actions/checkout@v4`.

A caller-specified permission block caps the called workflow and cannot be expanded by the callee. With only `actions: write`, the checkout token has no `contents: read`, so the gate fails before `verify-release-activation.sh` runs.

This will make the candidate `release.yml` pull-request check red, even if `ci.yml` is green.

**Required source fix before candidate PR CI:**

```yaml
custom-taskfleet-release-gate:
  uses: ./.github/workflows/taskfleet-release-gate.yml
  permissions:
    actions: write
    contents: read
```

Also remove:

```yaml
secrets: inherit
```

The gate uses `${{ github.token }}`, not repository secrets. Inheriting secrets gives PR-controlled gate code a path to future live `HOMEBREW_TAP_TOKEN`, `CARGO_REGISTRY_TOKEN`, and others. This need not block a dry-run-only R9 candidate, but it must be removed before R10 installs live credentials.

The validator must assert both permissions and the absence of `secrets: inherit`, not only the presence of `"actions": "write"`.

---

### 2. `host` structurally runs after a failed activation gate

**Paths:**
- `.github/workflows/release.yml:184-195` (`host.needs` and `host.if`)
- `.github/workflows/taskfleet-release-gate.yml:25-31` (cancel fallback)
- `scripts/validate-distribution-topology.sh:81-98`

This is the most dangerous finding and it is valid. `host` does not list `custom-taskfleet-release-gate` as a need. On a blocked tag:

1. The gate runs `verify-release-activation.sh`, sees activation != `ready`, and fails.
2. `build-local-artifacts` is skipped because its gate dependency failed.
3. `build-global-artifacts` is skipped through the build-local dependency.
4. `host` evaluates:

```yaml
if: ${{ always()
  && needs.plan.outputs.publishing == 'true'
  && (needs.build-global-artifacts.result == 'skipped' || needs.build-global-artifacts.result == 'success')
  && (needs.build-local-artifacts.result == 'skipped' || needs.build-local-artifacts.result == 'success') }}
```

Both build results are explicitly accepted as `skipped`, so `host` runs `dist host --steps=upload --steps=release` and then `gh release create`.

The only defense is the gate's asynchronous run-cancellation API call. That is not a fail-closed publication boundary.

**Required source fix:**

```yaml
host:
  needs:
    - plan
    - custom-taskfleet-release-gate
    - build-local-artifacts
    - build-global-artifacts
  if: >-
    ${{
      always()
      && needs.custom-taskfleet-release-gate.result == 'success'
      && needs.plan.outputs.publishing == 'true'
      && needs.build-global-artifacts.result == 'success'
      && needs.build-local-artifacts.result == 'success'
    }}
```

Skipped build acceptance is unnecessary for this fixed topology: the attached plan always has a non-null artifacts matrix and both local and global artifacts are required for publication.

The current validator check:

```sh
[[ "$(grep -Fc 'custom-taskfleet-release-gate' .github/workflows/release.yml)" -ge 2 ]]
```

proves only that the string appears twice. It does not prove `host` is gated. Replace it with YAML-structural assertions over the job graph.

---

### 3. `plan` runs credentialed `dist host --steps=create` before activation

**Paths:**
- `.github/workflows/release.yml:47-85` (`plan` job)
- `.github/workflows/release.yml:15-17` (`permissions: "contents": "write"`)
- `issues/taskfleet-distribution-topology/receipts/host-create-no-mutation.json`

On tag push, `plan` and `custom-taskfleet-release-gate` start concurrently. `plan` executes:

```sh
dist host --steps=create --tag="${github.ref_name}"
```

with `GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}` and the workflow default `contents: write`.

The only no-mutation receipt is explicitly credential-less:

```json
"credentials": { "GH_TOKEN": false, "GITHUB_TOKEN": false }
```

It does not prove the credentialed path actually used here is inert.

**Required source fix:**

Make `plan` depend on the gate:

```yaml
plan:
  needs: custom-taskfleet-release-gate
```

On PRs the gate succeeds, so plan-mode cargo-dist still runs. On a blocked tag the gate fails and cargo-dist never reaches `host --steps=create`.

If cargo-dist generation cannot express that order directly, split the reusable gate/generation hook so a read-only early activation check gates `plan`, with cancellation retained only as defense in depth.

Also drop the workflow-wide `contents: write` to `contents: read` and grant `contents: write` only to `host`. That reduces the credentialed surface of the `host --steps=create` path and PR-triggered build jobs.

---

## Valid but secondary or pending

### Self-hosted fork-PR execution risk

**Path:** `.github/workflows/ci.yml:76-105`

Valid. The R9 matrix runs every pull-request checkout and Rust test code on the persistent `[self-hosted, macOS, ARM64]` runner. In a public repository, fork PR code should not run on a persistent self-hosted machine that later performs release builds.

This does not have to abandon the required self-hosted R9 acceptance leg. Fix:

```yaml
if: >-
  ${{
    github.event_name == 'push' ||
    github.event.pull_request.head.repo.full_name == github.repository
  }}
```

Keep hosted macOS coverage for fork PRs if platform test coverage is still desired there. R9 candidate PR is same-repository, so the self-hosted leg still runs.

---

### Required status check name change

**Path:** `ci.yml` test job display name

Claude-fable-5's N3 is valid as a merge-mechanics issue, not a source diff defect.

The required check changes from:

```
test (macos-latest)
```

to:

```
test (self-hosted-macos-arm64)
```

If branch protection still requires the old name, the candidate PR cannot merge. The before-receipt records rule/settings existence but not the required check names. This must be verified and updated in the same transaction.

This is an evidence/config step, not a remaining Rust/YAML source blocker.

---

### cargo-dist release selector is still generic `macOS`

**Paths:**
- `/tmp/r9-cargo-dist-plan.json`, `artifacts_matrix.include[0].runner`
- `release/taskfleet-distribution.json:13`

Valid. `ci.yml` now proves the exact labels `[self-hosted, macOS, ARM64]`, but cargo-dist release builds still use only `macOS`. Candidate CI does not prove the release selector resolves to the same machine unless the pending runner receipt establishes that exactly one repository-visible runner has label `macOS` and that it is the ARM64 machine.

This is pending evidence if uniqueness is proven, or a source/config change if a dedicated label is needed.

---

### Overbroad release permissions and text-based validation

Valid but secondary. `contents: write` at workflow level is excessive for plan/build jobs. The validator's string counting allowed the `host` bypass to pass. Both should be fixed with the release boundary changes above.

---

### `publish-crates.yml` still uses hosted `macos-latest`

**Path:** `.github/workflows/publish-crates.yml:62-69`

Gemini flagged this as a split runner problem. It is not a release blocker. If the release path intentionally avoids the self-hosted machine, document that. If it is supposed to prove the same platform environment as normal CI, align it. Non-blocking for candidate CI.

---

## Wrong or conflicting with ADR/task boundary

### Gemini's "ledger state corruption" is incorrect

The claim that updating:

```json
"source_repository": {
  "current": "jarimustonen/taskfleet"
}
```

destroys migration evidence is wrong. This is current topology state, not an immutable transition ledger. The before-state is separately recorded in:

```json
"repository_id": 1265770191,
"full_name": "jarimustonen/orchestratectl"
```

under `issues/taskfleet-source-repository-rename/evidence/before-gates.json`.

Reverting `current` to `jarimustonen/orchestratectl` would make validation and release activation checks wrong after the one-way rename. Do not revert.

---

## Top three remaining source blockers

1. **Reusable gate caller lacks `contents: read` and inherits all secrets.** This breaks the candidate release PR check and creates a future secret-exposure path. Fix the caller permissions, remove `secrets: inherit`, and update the generated validator.

2. **`host` does not require successful activation.** A blocked tag can still reach `dist host --steps=upload --steps=release` and `gh release create` because the host condition accepts skipped build dependencies. Add the gate to `host.needs` and require its result to be `success`.

3. **`plan` executes credentialed `dist host --steps=create` outside the gate.** The current no-mutation receipt does not apply to the credentialed tag path. Make `plan` depend on the gate, remove its need for a write-capable token, and restrict `contents: write` to the `host` publishing job.

---

## Candidate disposition

The release activation state remains correctly blocked. The tap token staying inert until R10 is consistent with the supplied deferral. The cargo-dist regeneration, canonical URLs, and residual classification are largely sound.

Merge must remain blocked until source fixes 1–3 are applied. Requirements such as the self-hosted ARM64 acceptance run, canonical after-state repository receipt, redirect-independent operations, and old installer URL deferral are evidence/verification steps that remain pending and are not cured by source changes alone.
