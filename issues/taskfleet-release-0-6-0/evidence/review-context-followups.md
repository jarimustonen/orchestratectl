[thread_id:group_3797a7b30ff342e4bb6e13db8ece8451]

## Model: gemini-3.1-pro-preview
[model:gemini-3.1-pro-preview]

**Error:** Thread 'api_5f3' not found. It may have expired or never existed.

---

## Model: gpt-5.6-sol
[model:gpt-5.6-sol] [thread_id:api_88a25b2940ea4cd3bff9f43574201111]

# Revised assessment

The complete files resolve several earlier concerns, but the design still has release-safety blockers.

## Prior conclusions that change

- **Activation is composed with authorization.** `scripts/verify-release-tag-authorization.sh:20` calls `verify-release-activation.sh`, and `shipshape-release.sh:543` calls `assert_cut_activated` before creating the receipt. My earlier conclusion that authorization could bypass blocked Homebrew activation is no longer valid.
- **The ledgers are intentionally intermediate.** The current mismatch between blocked ledger files and the final-state validator is not a false claim because this tree is explicitly not a merge candidate. It remains an engineering problem for validating Phase A safely, discussed below.
- **Most resume preconditions now precede receipt creation.** `assert_run_may_resume`, checkpoint validation, repository identity, remote-tag absence, and activation validation all run before `record_release_authorization`.
- **The verifier handles annotated tags correctly.** Comparing `HEAD^{commit}` rather than `github.sha` is correct.
- **The verifier intentionally avoids a post-push current-main lookup.** This prevents a valid release from failing merely because `main` advances after tag publication.
- **Ref validation and API lookup are adequate for the admitted tag syntax.** The wrapper calls `git check-ref-format`; the tag is constrained and tied to Cargo’s workspace version.
- **The generated host condition rejects local build failure.** With the shown DAG, a verifier failure makes `build-local-artifacts` fail, `build-global-artifacts` skip, and `host` reject the run because local is neither `success` nor `skipped`.
- **Generator determinism has been established externally.** `dist generate --check` succeeding addresses the hand-edit concern, although CI does not currently enforce it.

## Remaining blockers

### 1. The gate is not authoritative for tags targeting older commits

**Files:**

- `.github/workflows/release.yml:42-46`
- `.github/build-setup.yml:1-5`
- `scripts/verify-release-tag-authorization.sh`
- repository settings: no tag ruleset

The authorization verifier is loaded from the tagged commit. An unauthorized tag targeting an older commit can run the older workflow from that commit, which may not contain this verifier or may contain the discarded permissive gate.

Therefore the current design only fails closed for tags whose target already contains the hardened workflow. It does not structurally protect the repository’s full release-tag namespace.

This is the most important remaining source-level limitation. A current-commit authorization branch cannot force historical workflow revisions to honor it.

**Required before activation:**

Create a repository ruleset covering the exact release namespace, not just the authorization namespace. At minimum:

```text
refs/tags/v*
```

The ruleset must restrict tag creation to the smallest possible actor set and deny updates and deletion. Existing historical tags need an explicit exception only if GitHub requires one; they must not become mutable.

However, without a dedicated actor, this still cannot distinguish the wrapper from a manual tag push by the same operator. Consequently:

- the in-repository verifier is credible protection against accidental tags at the hardened commit;
- the tag ruleset is credible protection against ordinary users/workflows and historical-workflow bypass;
- neither proves that the local wrapper, rather than the same privileged operator, pushed the tag.

That limitation must be stated in the activation evidence. It is broader than merely “a malicious repository admin can imitate the wrapper”: an accidental manual push by an actor permitted to create release tags is also indistinguishable.

A dedicated GitHub App is not the only possible future separation mechanism. A tightly controlled default-branch workflow using an environment-protected credential and a ruleset-authorized bot actor could also separate tag authority, but it would require redesigning Shipshape’s tag-resume ownership.

---

### 2. The authorization namespace is currently mutable and forgeable by repository writers

**Files:**

- `scripts/shipshape-release.sh:275-308`
- repository settings: no authorization ruleset
- `.github/workflows/release.yml:17-18`

The create-ref API gives atomic **create-if-absent** behavior, but the resulting branch is not immutable. With no ruleset, a writer can:

- create the expected authorization ref before the wrapper;
- force-update it;
- delete it and recreate it;
- create it for a tag whose wrapper transaction never reached authorization.

The API operation is correctly fail-closed under races, but remote policy is still the security boundary.

**Required before activation:**

Protect:

```text
refs/heads/taskfleet-release-authorizations/**
```

with a ruleset that:

- restricts creation;
- blocks updates;
- blocks force pushes;
- blocks deletion;
- excludes `GITHUB_TOKEN` and unrelated automation from bypass;
- records all bypass actors explicitly.

The tag and authorization rulesets should be exported as sanitized JSON and checked by an activation preflight against exact ruleset IDs and expected fields. Source fixtures cannot establish server-side immutability.

---

### 3. Release artifact jobs receive unnecessary `contents: write`

**File:** `.github/workflows/release.yml:17-18`

The workflow declares:

```yaml
permissions:
  "contents": "write"
```

at workflow scope. Consequently, all jobs—including the three platform build jobs and the plan job—receive a write-capable token.

This creates avoidable exposure:

- self-hosted and GitHub-hosted build jobs execute compilers, build scripts, dependencies, shell installers, and cargo-dist;
- a compromised build dependency can mutate repository refs or releases;
- the authorization verifier runs before the build, but it does not constrain what later build code does with the write token;
- the authorization namespace is currently unprotected, so the token can potentially alter release receipts.

Only the GitHub release-hosting portion should need source-repository write permission.

**Required change:**

Generate job-scoped permissions if cargo-dist 0.28.2 supports them:

```yaml
permissions:
  contents: read

jobs:
  plan:
    permissions:
      contents: read

  build-local-artifacts:
    permissions:
      contents: read

  build-global-artifacts:
    permissions:
      contents: read

  host:
    permissions:
      contents: write
```

`publish-homebrew-formula` uses the separate tap token and should not receive source-repository write access.

If cargo-dist 0.28.2 cannot generate these permissions, document that limitation and compensate with rulesets that deny `GITHUB_TOKEN` all authorization/tag namespace mutations. The broad workflow permission remains a supply-chain weakness even with those rulesets.

---

### 4. The plan job runs before authorization with a write token

**File:** `.github/workflows/release.yml:48-85`

The first authorization check occurs only inside `build-local-artifacts`. Before that, `plan`:

1. checks out tag-controlled source;
2. downloads and executes cargo-dist;
3. executes:

```bash
dist host --steps=create --tag=...
```

4. holds `contents: write`.

The existing `host-create-no-mutation.json` receipt proves a credential-free local invocation did not mutate public state. It does not prove that a credentialed invocation in Actions cannot mutate state, and the command name itself is too consequential to leave outside the gate.

Even if cargo-dist 0.28.2’s `create` step is currently local-only, an unauthorized tag should not execute release-host planning under a write token before authorization.

**Required change:**

Run the authorization verifier in `plan` immediately after checkout and before installing or invoking cargo-dist:

```yaml
- uses: actions/checkout@v4
- name: Require release authorization
  env:
    GH_TOKEN: ${{ github.token }}
  run: ./scripts/verify-release-tag-authorization.sh
```

Keep the checks in every local build job as defense in depth.

If generator-supported build setup cannot add this step to `plan`, remove write permission from `plan` and produce exact evidence that `dist host --steps=create` is non-mutating with a valid read token. Prefer both changes.

---

### 5. Receipt creation is not atomic with the final main state or tag push

**File:** `scripts/shipshape-release.sh:526-546`

The sequence is:

```bash
fetch/check main
...
record_release_authorization
assert_remote_tag_absent
shipshape release resume
```

The create-ref API atomically creates the authorization ref, but it does not atomically assert that `main` still equals `bump_commit`, nor does it atomically push the tag.

Two gaps remain.

#### Main race

`main` can advance after the final fetch and before authorization creation. The receipt would then claim the wrapper’s “exact-main” authorization even though the commit was no longer main at receipt creation.

This is not a malicious-admin problem; an ordinary concurrent merge can cause it.

#### Authorized-but-untagged state

After receipt creation, the wrapper can fail before Shipshape pushes the tag. The durable receipt then remains, and any later push of the exact tag at the exact commit passes CI—even if it is not initiated by resuming the journal.

This is deliberate for recovery, but it means the receipt authorizes the coordinate, not a particular successful wrapper invocation.

**Required change:**

At minimum:

1. Re-fetch and check `main` immediately before receipt creation.
2. Treat receipt creation as an irreversible release transaction boundary.
3. Record the receipt in the held checkpoint/journal.
4. After receipt creation, permit only reconciliation of that same run; never abandon it.
5. Add a test for failure after receipt creation and before tag push.

For a stronger exact-main guarantee, perform a remote compare-and-swap operation tied to `main`. One possible approach is an atomic push containing:

- a no-op leased update of `main` at the expected SHA; and
- creation of the authorization ref.

Whether GitHub accepts this under the intended main ruleset must be tested. If not, use an explicit merge freeze/branch lock for the short authorization-to-tag interval and record it as operational evidence.

Atomic receipt-and-tag publication would eliminate the second gap, but it conflicts with Shipshape owning the resumed tag push. Without modifying Shipshape, the honest contract is:

> The authorization ref irreversibly authorizes one tag/commit coordinate after exact-SHA CI; Shipshape remains the required procedural mechanism for publishing and reconciling that coordinate.

It should not be described as cryptographic proof that Shipshape itself pushed the tag.

---

### 6. Exact-main CI does not test the new release authorization boundary

**Files:**

- `.github/workflows/ci.yml:18-43`
- `scripts/shipshape-release.sh:209-236`

`wait_for_exact_main_ci` trusts a successful `ci.yml` push run. But the shown `ci.yml` does not directly run:

- `scripts/test-release-authorization.sh`;
- `dist generate --check`;
- the cargo-dist plan plus `validate-distribution-topology.sh`;
- a prepared-state equivalent of activation validation.

Therefore exact-main green CI does not currently attest the security mechanism that later relies on that CI result.

The release workflow will retest authorization after the tag, but that is too late to establish that the receipt was created only for a commit whose release topology was reviewed and deterministic.

**Required change:**

Add a dedicated CI job, for example:

```yaml
release-topology:
  runs-on: ubuntu-22.04
  permissions:
    contents: read
  steps:
    - uses: actions/checkout@v4
    - name: Install pinned cargo-dist
      run: ...
    - run: dist generate --check
    - run: ./scripts/test-release-authorization.sh
    - run: dist plan --output-format=json > /tmp/dist-plan.json
    - run: ./scripts/validate-distribution-topology.sh /tmp/dist-plan.json
```

Because the current validator accepts only final activation, split validation into:

- structural/prepared topology validation, valid while credentials remain blocked;
- final activation validation, requiring `ready` and `active-proven-r10`.

The final merged exact-main SHA must run the final-state variant successfully before the wrapper can create a receipt.

---

### 7. The final-state-only validator prevents safe integration of Phase A

**Files:**

- `scripts/validate-distribution-topology.sh:42-61`
- `release/taskfleet-release.json:3`
- `release/taskfleet-distribution.json:3,13`

The current validator requires:

```jq
.activation == "ready"
.tap_secret_state == "active-proven-r10"
```

while the honest ledgers remain:

```json
"activation": "prepared-blocked-r10"
"tap_secret_state": "pending-r10-proof"
```

The clarification says this intermediate tree is intentionally not mergeable. That avoids lying, but it also prevents the hardened topology from receiving ordinary integrated CI before the credential transition. It forces code hardening, server-side credential proof, and activation-ledger mutation into one large final transition.

That undermines Phase A’s intended ordering: harden first, prove the hardening, then activate after credential proof.

**Required change:**

Separate structural validity from activation readiness:

```bash
validate-distribution-topology.sh --state prepared plan.json
validate-distribution-topology.sh --state active plan.json
```

Shared structural assertions should run in both modes. The prepared mode should require exactly:

```json
{
  "activation": "prepared-blocked-r10",
  "tap_secret_state": "pending-r10-proof"
}
```

The active mode should require exactly:

```json
{
  "activation": "ready",
  "tap_secret_state": "active-proven-r10"
}
```

Do not allow either mode to accept both states generically. CI must select the expected state explicitly so a partial ledger transition fails.

This allows the hardening commit to merge and receive CI while publication remains impossible.

---

### 8. The authorization fixture is useful but does not test several security-relevant cases

**File:** `scripts/test-release-authorization.sh`

The executable fixture is materially better than grep-only validation, but missing cases remain:

1. authorization object has the right SHA but wrong `.object.type`;
2. malformed or incomplete GitHub JSON;
3. repository API returns empty/malformed node ID;
4. `HEAD` does not resolve to a commit;
5. tag contains valid-looking but noncanonical SemVer spelling;
6. GH API returns 403/429/5xx;
7. authorization ref has an unexpected full `.ref`;
8. activation verifier is absent or non-executable;
9. receipt exists, but wrapper resume fails before tag publication;
10. historical tag target lacks the verifier entirely.

The final two require wrapper/repository-level tests rather than only the isolated verifier fixture.

The workflow graph checks remain textual and brittle:

```bash
grep -A12 '^  host:'
```

A generated formatting change can break them, while a sufficiently subtle expression change could pass substring checks without preserving semantics.

**Required change:**

- Add malformed/error response cases to the executable fixture.
- Parse workflow YAML structurally with a pinned parser.
- Assert the exact `host.if` expression or normalized AST.
- Assert the authorization step is present in `plan` and every matrix build.
- Add wrapper fixtures for post-authorization/pre-tag failure and recovery.
- Add a repository-policy test/evidence for historical-tag rejection.

This is not independently blocking if server rulesets and CI integration are implemented, but fixture strengthening should be completed before activation because this script is being treated as security regression evidence.

---

### 9. Runtime cargo-dist installation is version-pinned but not integrity-pinned

**File:** `.github/workflows/release.yml:59-65`

The workflow executes:

```bash
curl .../v0.28.2/cargo-dist-installer.sh | sh
```

Pinning the release version does not pin the bytes. A compromised upstream release asset or installer path can execute with the plan job’s token. The resulting binary is then uploaded and reused by host jobs.

This is especially serious while the plan job has `contents: write`.

**Required change:**

Where generator support permits:

- verify the installer or binary against a checked-in SHA-256;
- avoid pipe-to-shell;
- download to a file, hash it, then execute;
- keep the plan/build tokens read-only.

If exact cargo-dist 0.28.2 cannot generate integrity verification, record this as an accepted upstream supply-chain risk and compensate with job-scoped permissions. Leaving both mutable runtime installation and workflow-wide write authority is not acceptable for activation.

## Non-blocking observations

### `publish-crates.yml` dispatch is safe but misleading

**File:** `.github/workflows/publish-crates.yml:10`

`workflow_dispatch` remains enabled despite the header saying the workflow “never publishes on workflow_dispatch.” The current dependency behavior does prevent publication:

- `publish-core` is skipped because its `if` requires `push`;
- downstream publish jobs skip due to the skipped dependency;
- only `package-archives` runs on dispatch.

This is correct but indirect. Add explicit `if: github.event_name == 'push'` to all three publication jobs for defense in depth, or move package inspection into a separate workflow.

### Push CI does not run the self-hosted macOS job

**File:** `.github/workflows/ci.yml:85-109`

The self-hosted job runs only for same-repository pull requests. Thus `wait_for_exact_main_ci` can accept a push run where that job is skipped. This is acceptable only if the recorded Phase B candidate PR provides the required self-hosted proof and the final-main requirement does not demand repetition. The release evidence should state that distinction explicitly.

### API failure handling is fail-closed

Both verifier API calls fail nonzero, and `record_release_authorization` converts a failed GET into a POST that itself must succeed. This is safe. Diagnostics are sparse, but no observed API error is interpreted as authorization success.

## Bottom line

The revised authorization-ref design is credible for preventing an accidental unauthorized tag **at the hardened commit**, and the generated local/global/host DAG fails closed on verifier failure.

It is not ready for activation because:

1. historical commits can supply older release workflows that do not honor the gate;
2. neither tag nor authorization refs are protected server-side;
3. all cargo-dist jobs receive `contents: write`;
4. the plan job executes before authorization;
5. receipt creation is not atomic with current-main state or tag publication;
6. exact-main CI does not test the new release boundary;
7. prepared and active topology validation are not separated.

The missing fine-grained Homebrew token is therefore not the only activation blocker. The repository rulesets, CI topology job, permission reduction, pre-plan authorization, and prepared/active validator split should be completed before credential proof can legitimately transition the ledgers to `ready`.

---

## Model: claude-fable-5
[model:claude-fable-5] [thread_id:api_82a1f113da96411e84d1023d7feff1d6]

# Revised review against the complete implementation

## Prior conclusions that change

- **B1 (ledger contradiction / "CI red")** — partially wrong. `validate-distribution-topology.sh` is not wired into `ci.yml` or `publish-crates.yml` (only `validate-release-topology.sh` is, and it accepts `blocked-r8-r9-r10`), so ordinary CI is not red. The contradiction stands but is confirmed intentional (non-merge-candidate intermediate). Residual issues below.
- **B2 (mutable receipt)** — largely resolved. `record_release_authorization` now uses `POST /git/refs`, which is atomic create-only (422 on existing); fast-forward mutation and the TOCTOU are closed. Residuals: no ruleset, orphaned-ref hazard (new blocker below).
- **B3 (CI-side main-equality bricking)** — resolved. The verifier checks only the durable ref, per the comment in `verify-release-tag-authorization.sh:26-29`. The wrapper-side orphan trap survives in a worse form (new blocker N1).
- **B5 (unseen verifier)** — resolved as a design; annotated-tag peeling is handled correctly (`HEAD^{commit}` after checkout, not `GITHUB_SHA`), the exact-ref lookup is defended against GitHub's prefix-array quirk (`keys | index("ref")` fails on arrays), `.object.type == "commit"` blocks tag-object targets, and repo node ID `R_kgDOS3Iezw` is pinned. New concerns are environmental, not logical.
- **M1 (token scopes)** — resolved. Both `gh api` reads need only `metadata` (implicit) and `contents: read`; both workflows grant that.
- **M2 (auth branches trigger workflows)** — resolved: `ci.yml` push trigger is `branches: [main]`; the other workflows are tag-only. Accumulation/branch-UI noise remains cosmetic.
- **M3 (`origin/main` ambiguity)** — resolved; `refs/remotes/origin/main` is used everywhere it matters.
- **M4 (orphaned activation script)** — resolved; `verify-release-activation.sh` is actively invoked by the tag verifier and by `assert_cut_activated`.
- **M5 (generator determinism)** — resolved per clarification; recommendation to enforce `dist generate --check` in CI stands (nothing currently guards regression between now and activation).

---

## Remaining blockers (pre-activation)

### N1. Orphaned authorization ref is a standing, consumable authorization

`shipshape-release.sh` creates the ref (`record_release_authorization`) after CI success and immediately before `shipshape release resume`. If resume fails after ref creation (network, shipshape fault, the wrapper's own second `assert_remote_tag_absent` racing) and the run is later abandoned:

- The ref `refs/heads/taskfleet-release-authorizations/v0.6.0` persists at the abandoned bump commit.
- That commit was exact-main CI-green with workspace version 0.6.0. **Any subsequent accidental/manual `git push origin v0.6.0` at that commit passes the verifier completely** — repo ID, tag/version match, activation, ref target — and both release legs publish. The structural boundary is fail-open exactly in the abandonment case.
- Meanwhile a re-plan of v0.6.0 at a *new* bump commit hard-fails in `record_release_authorization` ("points at X, expected Y"), so the version is unusable without manually deleting the ref — which the design calls immutable and has no protocol for.

Note the abandonment frequency is nontrivial: `ci.yml` has `cancel-in-progress: true` on the main group, so any concurrent push to main cancels the bump commit's run, `gh run watch --exit-status` fails, and `advance_main_to_bump` will thereafter reject the moved `origin/main` forever.

Required: an explicit abandon protocol that retracts or tombstones the ref (e.g., a wrapper `abandon <run-id>` verb that deletes the ref before adding the run to `never_resume_runs`), and a documented resolution of the immutability tension. `never_resume_runs` alone does not neutralize the ref.

### N2. Rulesets remain the actual boundary and don't exist yet

Confirmed: no rulesets/branch protection. Consequences that must gate the ledger flip:

- Any writer can delete/force-move `taskfleet-release-authorizations/*` via plain `git push` — the API create-only property protects creation, not existence.
- Historical-commit tag pushes run pre-verifier workflow versions. Mitigations that currently hold: the tap token is inert random data (confirmed), old `publish-crates.yml` tag/version matching plus crates.io duplicate rejection closes the crates leg, and existing tags can't be recreated. Residual: a tag like `orchestratectl/0.5.1` on an old commit matches `'**[0-9]+.[0-9]+.[0-9]+*'` and can mint a spurious GitHub Release with old artifacts under `contents: write`. Also note **`CARGO_REGISTRY_TOKEN` is presumably live today**; the crates leg is protected only by version/duplicate arithmetic, not by any structural control, until the tag ruleset lands.
- The planned tag ruleset + authorization-namespace ruleset must be verified *before* `activation: ready` — add them to the flip-gate checklist explicitly (they are currently only "planned").

### N3. `test-release-authorization.sh` — the negative workflow fixture is dead code

Lines building `$unsafe`: the file is constructed, its construction is asserted, and then **nothing is ever run against it**. The final check greps the *real* `$release` for `secrets: inherit` — a duplicate of the check at the top of the script. The comment "restoring either fails" tests nothing:

- No validator/verifier is invoked against `$unsafe` to prove the tag-only check would catch a restored `pull_request:` trigger.
- No `secrets: inherit` unsafe variant is constructed at all.

Fix: factor the workflow-safety greps into a function taking a file path; assert it passes on `$release` and fails on both unsafe fixtures. As written, the fixture suite proves the *verifier script's* fail-closed behavior well (the 9-case mutation loop is good) but proves nothing about workflow-topology regression detection beyond plain grep duplication.

### N4. Release-critical environment fragility on the self-hosted macOS runner (and any container leg)

The full chain executed inside every matrix build job on tag push is:
`verify-release-tag-authorization.sh` → `gh`, `jq`, `git` → `verify-release-activation.sh` → `cargo metadata --locked`, `test-release-authorization.sh` → fixture with `env -i PATH="$tmp/bin:/usr/bin:/bin"`.

Problems:

- The restricted-PATH fixture requires `jq`, `git`, `awk`, `bash` under `/usr/bin:/bin`. `jq` is only in `/usr/bin` on recent macOS; on an older self-hosted image (Homebrew-only `jq` at `/opt/homebrew/bin`), the *positive* `run_auth` fails → `set -e` aborts → the verifier fails → **every legitimate release fails on the aarch64-apple-darwin leg**. Fail-closed, but bricking. Fix: resolve real tools via `command -v` and symlink them into `$tmp/bin` instead of hardcoding `/usr/bin:/bin`.
- `gh` is not guaranteed on self-hosted runners and is absent from typical containers. `release.yml` only installs Rust in the container branch, never `gh`/`jq`. Verify `matrix.container` is null for all three pinned targets (topology validation pins runners, not container nullity — add that assertion to `validate-distribution-topology.sh`'s plan check: `[.ci.github.artifacts_matrix.include[].container] | all(. == null)` or equivalent) and provision `gh`+`jq` on the mac runner as a documented activation precondition.

### N5. Atomic flip commit has an internal contradiction to resolve

`validate-distribution-topology.sh` simultaneously requires ledger `tap_secret_state == "active-proven-r10"` **and** receipt `final-secret-state-attestation.json` `.state == "deliberately-inert-after-r7-proof"`. Once a live fine-grained token is installed, the receipt assertion is a stale claim being enforced as current truth. Either re-scope the receipt check as explicitly historical (rename the key / add an `as_of` marker) or update the receipt in the flip commit with the R10 canary evidence. Also: after the flip, `validate-distribution-topology.sh` is still invoked from nowhere in CI — wire it into `publish-crates.yml` or `ci.yml` (it needs a plan JSON; at minimum run its non-plan sections), otherwise the "ready" enforcement only executes when someone remembers to run it.

---

## Major

- **`record_release_authorization` GET error conflation** (`shipshape-release.sh`): `gh api ... 2>/dev/null || true` treats 404, 5xx, and network failure identically; a transient GET failure falls through to POST, which fail-closes on 422 — acceptable — but a prefix-sibling ref (array response) makes `jq -er .ref` abort with a useless error. Distinguish 404 from other failures (`gh api` exit code + `--include` or check stderr) for diagnosability; behavior is fail-closed either way.
- **Exact-main CI gate covers `ci.yml` only, and its main-push run excludes the self-hosted macOS job** (`r9-self-hosted-macos` is `pull_request`-only). The bump commit is pushed directly to main by the wrapper, so the release-gating run never includes the self-hosted proof; `item.md` Phase B requires it on recorded exact SHAs. Coverage exists via hosted `macos-latest`, but reconcile this with the Phase B evidence requirement explicitly (the PR-candidate run is a different SHA than the merged bump commit).
- **`verify-release-activation.sh` / `validate-distribution-topology.sh` duplication**: tag-only trigger, `pr-run-mode`, and the fixture invocation are checked in both, with slightly different failure text. One owner; the other should call it. Drift here silently weakens whichever copy is actually on the enforcement path.

## Minor

- `host` job still accepts `build-local-artifacts.result == 'skipped'` and `test-release-authorization.sh` *asserts* that permissive condition is present. That's correct pinning of generator output, but the assertion reads as endorsing the skip path; add a comment that skip is unreachable for the pinned non-null matrix and that a writer who empties the matrix is outside the threat model.
- Mutation cases in the fixture assert only nonzero exit — they cannot distinguish "failed for the intended reason" from environmental breakage. The positive case guards total breakage, but consider asserting the verifier's failure point (e.g., via a trace env var) for at least `authorization_missing`/`authorization_sha`.
- `wait_for_exact_main_ci` takes `--limit 1` of possibly multiple runs and validates only `ci.yml`; adequate, but a re-run-after-failure on the same SHA could be the one selected — it validates headSha/branch/event and watches to success, so correctness holds.
- URL path encoding of tag names containing `+` (permitted by the verifier's regex) in `gh api .../git/ref/heads/...` is untested; the fixture only covers `v0.6.0`. Add a build-metadata tag case or tighten the regex to plain `v\d+\.\d+\.\d+`.
- Nothing continuously enforces `dist generate --check`; add it to `ci.yml` so `release.yml` cannot drift from `build-setup.yml`/`dist-workspace.toml` between now and the flip.

## Bottom line

The core mechanism is now sound: create-only API ref, peeled-commit comparison, exact-ref lookup with array defense, pinned repo ID, verifier executed in all three build legs and the crates gate, tag-only triggers, no inherited secrets. The credible remaining blockers before `ready` are: the orphaned-authorization fail-open path (N1), rulesets as enforced preconditions rather than plans (N2), the dead negative fixture (N3), verifier tool provisioning on the mac runner/containers (N4), and the flip-commit receipt/enforcement wiring (N5).

---

## Model: deepseek-v4-pro
[model:deepseek-v4-pro] [thread_id:api_12904303436345528272cba2946bff18]

## Revised conclusions

- The previous `origin/main` TOCTOU concern is **resolved**: the verifier now uses the durable authorization ref, not live `main`.
- The crates.io publish chain is now correctly gated through `release-version`, which runs the verifier on tag events.
- The broad Homebrew-token concern is **partially resolved**: the installed `HOMEBREW_TAP_TOKEN` is inert random data, so no broad token is currently reachable from Actions. Activation remains honestly blocked by the ledger.
- Prior missing-verifier/fixture gap is resolved: both scripts are now present and the fixture exercises the verifier and negative cases.

Remaining blockers below are against the actual current files.

---

## Blockers

### 1. Authorization is embedded in a skippable job, and `host` explicitly accepts that skip

`.github/workflows/release.yml`

The only release-workflow invocation of the verifier is inside `build-local-artifacts`:

```yaml
if: ${{ fromJson(needs.plan.outputs.val).ci.github.artifacts_matrix.include != null && ... }}
```

If `artifacts_matrix.include` is `null`, this job is **skipped and the verifier never runs**. `host` then permits that skip:

```yaml
if: ${{ always() && needs.plan.outputs.publishing == 'true' &&
       ... &&
       (needs.build-local-artifacts.result == 'skipped' ||
        needs.build-local-artifacts.result == 'success') }}
```

So a plan producing no local-artifact matrix bypasses the verifier entirely. `build-global-artifacts` and `host` can then publish without any authorization check.

`scripts/test-release-authorization.sh` currently codifies this as a security property:

```bash
grep -A12 '^  host:' "$release" | grep -F 'needs.build-local-artifacts.result == '\''skipped'\''' >/dev/null
```

That is backwards. The structural test should require either:

- a dedicated `authorize` job that all build/host jobs depend on, or
- `host` requiring `build-local-artifacts.result == 'success'` and the verifier always running.

Relying on `validate-distribution-topology.sh` to prove the exact graph has local artifacts is not enough because that script is **not invoked by `.github/workflows/release.yml` at runtime**. The live cargo-dist plan is never checked against that validator before `host` runs.

Fix with an always-run authorization job before `plan`/`host`:

```yaml
authorize:
  runs-on: ubuntu-22.04
  permissions:
    contents: read
  env:
    GH_TOKEN: ${{ github.token }}
  steps:
    - uses: actions/checkout@v4
    - uses: dtolnay/rust-toolchain@1.98.0
    - run: ./scripts/verify-release-tag-authorization.sh

plan:
  needs: authorize
  ...

build-local-artifacts:
  needs: [authorize, plan]
  ...

build-global-artifacts:
  needs: [authorize, plan, build-local-artifacts]
  ...

host:
  needs: [authorize, plan, build-local-artifacts, build-global-artifacts]
  ...
```

---

### 2. `plan` runs a mutating cargo-dist step before any authorization

`.github/workflows/release.yml`, `plan` job:

```yaml
env:
  GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
...
run: |
  dist ${{ (!github.event.pull_request && format('host --steps=create --tag={0}', github.ref_name)) || 'plan' }} --output-format=json > plan-dist-manifest.json
```

`dist host --steps=create` is the cargo-dist step that creates the GitHub Release object. It executes before every build-local verifier, with top-level `contents: write`.

This means a non-wrapper/accidental tag can produce a pre-authorization GitHub Release mutation even when the activation ledger is still blocked. The current guarded build/host condition only prevents artifact upload/finalization; it does not prevent `plan` from running the create step.

Move authorization ahead of `plan`. The `authorize` job above should be a hard dependency of `plan`, and `plan` must run `dist host --steps=create` only after authorization succeeds.

---

### 3. Activation does not verify the planned GitHub rulesets or authorization-ref immutability

`scripts/verify-release-activation.sh` checks only local JSON, TOML, workflow text, and the structural fixture. It does not verify actual GitHub state:

- no tag ruleset for `v*`;
- no branch/ref ruleset for `taskfleet-release-authorizations/**`;
- no protection against force-push, deletion, or arbitrary creation of authorization refs.

`scripts/shipshape-release.sh` currently creates a mutable branch:

```bash
printf 'refs/heads/taskfleet-release-authorizations/%s\n' "$tag"
```

Branches are not immutable. Without a ruleset, any repository writer, not just an admin, can create or move an authorization ref and push the matching tag once the ledger is `ready`.

This is currently masked by the blocked ledger, but it must be an **activation precondition**:

- create the rulesets first;
- then have `verify-release-activation.sh` query GitHub and require them as part of the `ready` transition.

A stronger construction is to use a tag namespace outside the release trigger, e.g.:

```text
refs/tags/release-authorizations/<tag>
```

with tag protection, instead of `refs/heads/taskfleet-release-authorizations/<tag>`.

---

### 4. Stranded authorization ref after pre-tag resume failure

`scripts/shipshape-release.sh`:

```bash
record_release_authorization
assert_remote_tag_absent
shipshape release resume "$run_id" --json
```

If `shipshape release resume` fails before pushing the tag, the authorization ref remains at `bump_commit`. A later accidental/non-wrapper tag at that same commit would pass the verifier without going through the wrapper.

Add a failure path:

```bash
if ! shipshape release resume "$run_id" --json; then
  if [[ "$(remote_tag_commit)" != "$bump_commit" ]]; then
    gh api --method DELETE \
      "repos/$expected_repo/git/refs/heads/$ref_name" >/dev/null 2>&1 || true
  fi
  exit 1
fi
```

Only delete when the remote tag is still absent; if the tag was pushed, the irreversible boundary is crossed and cleanup must not remove the receipt.

---

### 5. `github.sha` is still used as a commit SHA for release/publish while annotated tags are supported

`.github/workflows/release.yml`:

```yaml
RELEASE_COMMIT: "${{ github.sha }}"
...
gh release create "${{ needs.plan.outputs.tag }}" --target "$RELEASE_COMMIT" ...
```

`.github/workflows/publish-crates.yml`:

```yaml
SOURCE_COMMIT: ${{ github.sha }}
```

`scripts/verify-release-tag-authorization.sh` explicitly states:

> Do not compare with github.sha, which may identify an annotated tag object for a tag push.

The wrapper permits annotated tags (`assert_hook_marker` accepts tag object OID while checking the peeled commit equals `bump_commit`). If Shipshape creates an annotated tag, `github.sha` may be the tag object SHA, not the commit SHA.

Compute the peeled commit in the workflow instead:

```bash
SOURCE_COMMIT="$(git rev-parse "refs/tags/$GITHUB_REF_NAME^{commit}")"
```

and use that for `gh release create --target` and source-commit verification. The same applies to the host release step.

---

## Additional risks

### 6. Exact bump-SHA push CI does not include the self-hosted macOS proof

`.github/workflows/ci.yml`:

```yaml
r9-self-hosted-macos:
  if: >-
    github.event_name == 'pull_request' &&
    github.event.pull_request.head.repo.full_name == github.repository
```

This job is skipped on push to `main`. `scripts/shipshape-release.sh` waits only for push-main CI:

```bash
wait_for_exact_main_ci "$bump_commit"
```

So the exact bump SHA pushed by the wrapper gets hosted macOS CI, but not the required self-hosted Apple Silicon proof. This contradicts the Phase A/B requirement that the final exact-main SHA receive the full gate, including a self-hosted macOS proof.

Change the CI condition to also run on same-repo pushes to main:

```yaml
if: >-
  (github.event_name == 'pull_request' && github.event.pull_request.head.repo.full_name == github.repository) ||
  (github.event_name == 'push' && github.ref == 'refs/heads/main')
```

Or add a separate required self-hosted check that the wrapper waits for.

---

### 7. `resume` command lacks the `main` branch guard that `cut` enforces

`scripts/shipshape-release.sh` `cut` checks:

```bash
[[ "$branch" == main ]] || { ... exit 1; }
```

`resume` does not. A user running `scripts/shipshape-release.sh resume <run-id>` from a non-main branch can make `advance_main_to_bump` operate on the wrong branch or fail after partially mutating the local checkout.

`resume` should require:

```bash
branch="$(git symbolic-ref --quiet --short HEAD)"
[[ "$branch" == main ]] || { echo "release resume must run on main" >&2; exit 1; }
```

---

### 8. Cargo dependency in the verifier is ordered before toolchain installation in non-container release jobs

`verify-release-tag-authorization.sh` calls `scripts/verify-release-activation.sh`, which runs:

```bash
cargo metadata --locked --no-deps --format-version=1
```

In `.github/workflows/release.yml`, the verifier runs immediately after checkout/container Rust install. For non-container matrix jobs, especially the self-hosted macOS runner, there is no `dtolnay/rust-toolchain` or rustup step before the verifier.

If `cargo` is not preinstalled on the runner, every authorized tag will fail. Make the runner toolchain a defined precondition for release CI, or split the verifier so authorization does not require `cargo metadata`.

---

### 9. Activation ledger/validator transition must be atomic

Current `release/taskfleet-release.json` is `blocked-r8-r9-r10`, `release/taskfleet-distribution.json` is `prepared-blocked-r10` with `tap_secret_state: pending-r10-proof`. But `scripts/validate-distribution-topology.sh` now requires `ready` and `active-proven-r10`.

This is intentionally not merge-ready, but it means the current tree cannot pass its own validator. Final activation must update all of the following in one atomic change:

- `release/taskfleet-release.json` → `ready`
- `release/taskfleet-distribution.json` → `ready` / `active-proven-r10`
- `scripts/validate-distribution-topology.sh` and `scripts/verify-release-activation.sh` expectations
- the GitHub rulesets from blocker 3

The fixture test `scripts/test-release-authorization.sh` stubs `verify-release-activation.sh`, so it does not catch this inconsistency.
