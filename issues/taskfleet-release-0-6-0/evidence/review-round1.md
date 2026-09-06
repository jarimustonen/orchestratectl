[thread_id:group_bdad42465a214b7a8c4d880c9290d6bb]

## Model: gemini-3.1-pro-preview
[model:gemini-3.1-pro-preview] [thread_id:api_7eebb5f89a504ffaa3fd8b08cf65a620]

This revised Phase A design introduces a structural exact-SHA authorization check. However, the implementation contains concrete bugs that will instantly fail CI, pollutes the Git repository namespace, and has significant security bypasses if repository permissions are not perfectly aligned.

Here are the specific blockers and architectural problems.

### 1. Blocker: Ledger Inconsistency Will Fail CI Instantly
The validation scripts check for activated JSON states, but the JSON files in the diff do not match those assertions. 

*   In `scripts/validate-distribution-topology.sh` (Lines 42, 46): The script demands `.activation == "ready"` and `tap_secret_state:"active-proven-r10"`.
*   In `release/taskfleet-distribution.json` (Line 13): You set `"tap_secret_state": "pending-r10-proof"` and left the `.activation` key unchanged.
*   In `scripts/validate-distribution-topology.sh` (Line 61): You demand `.activation` in `release/taskfleet-release.json` to be `"ready"`, but the file is not updated in this diff.

**Solution**: Update the JSON fixtures to precisely match the target states checked by the topology validator.
```json
// release/taskfleet-distribution.json
"activation": "ready",
"tap_secret_state": "active-proven-r10",
```

### 2. Architectural Flaw: Namespace Pollution via `refs/heads/*`
In `scripts/shipshape-release.sh` (Line 264), you encode the authorization receipt as a standard branch:
```bash
release_authorization_ref() {
  printf 'refs/heads/taskfleet-release-authorizations/%s\n' "$tag"
}
```
Pushing to `refs/heads/*` creates a standard branch. This has negative side effects:
1. It will trigger any GitHub Actions listening to `on: push` (unless explicitly excluded), wasting runner minutes and confusing status checks on the exact bump commit.
2. It permanently pollutes every developer's local branch list during a routine `git fetch`.

**Solution**: Use a custom, non-standard ref namespace for immutable metadata. Custom refs do not trigger GitHub Actions push events and are ignored by standard `git fetch`.
```bash
release_authorization_ref() {
  printf 'refs/taskfleet/release-authorizations/%s\n' "$tag"
}
```
*Note: The verification script must explicitly fetch this custom ref (e.g., `git fetch origin +refs/taskfleet/*:refs/taskfleet/*` or `git ls-remote origin "refs/taskfleet/release-authorizations/$tag"`).*

### 3. Security: Authorization Boundary Bypass
You state the goal is a "structural fail-closed boundary against unauthorized/non-wrapper tags," but acknowledge a malicious admin could bypass it. However, the current design allows *any developer with standard write access* to bypass it, because pushing the authorization ref is just a standard git push.

If a developer bypasses the wrapper, they can simply:
```bash
git push origin <bad-commit>:refs/heads/taskfleet-release-authorizations/v0.6.0
git push origin <bad-commit>:refs/tags/v0.6.0
```
This satisfies the structural check but completely circumvents the wrapper's safety (exact-main CI, lock validation). 

**Solution**: 
To make this a true fail-closed boundary, you must apply GitHub Repository Rulesets or Branch Protections restricting creation of the authorization namespace (whether `refs/heads/...` or custom) strictly to an elevated service account or GitHub App. If relying on human write access, the verification script `verify-release-tag-authorization.sh` must also actively verify that the target commit is an ancestor of `origin/main` at the exact moment of execution, eliminating circular tag-equals-branch logic.

### 4. Design Flaw: Manual Recovery Disabled
In `scripts/validate-distribution-topology.sh` (Line 78), you ban `workflow_dispatch:` from the generated `release.yml`. 
```bash
if grep -A12 '^on:' .github/workflows/release.yml | grep -Eq 'workflow_dispatch:|pull_request:'; then
  echo "cargo-dist release workflow must be tag-only" >&2; exit 2
fi
```
ADR 0002 states: *"If any permanent leg succeeds and another fails, resume the missing idempotent leg from the same commit"*. By entirely banning `workflow_dispatch`, you remove the ability to manually trigger a payload retry without pushing a new tag or relying solely on the GitHub UI's "Re-run failed jobs" (which may not always be sufficient or preserve the exact payload context).

**Solution**: Allow `workflow_dispatch` triggers for operational recovery, generated via cargo-dist's `dispatch-releases = true` configuration.

### 5. Race Condition: Exact-Main Check vs Ref Creation
In `scripts/shipshape-release.sh` (Line 442):
```bash
wait_for_exact_main_ci "$bump_commit"
git fetch origin +refs/heads/main:refs/remotes/origin/main
[[ "$(git rev-parse origin/main)" == "$bump_commit" ]] || { ... exit 1 }
# <gap where origin/main can advance>
record_release_authorization
```
There is a race condition. If `main` receives a PR merge immediately after the fetch but before the branch push, you will record an authorization for a commit that is no longer the tip of main.

**Solution**: Check `main` *again* after the push, or query the GitHub API to ensure the commit is the exact head of main in a single atomic script evaluation.

## Context request
To evaluate the true strength of the authorization boundary, the implementation of the verification gate is needed.

- Kind: `artifact`
- Need: `scripts/verify-release-tag-authorization.sh`
- Why: This script contains the actual gate logic. If it merely checks `tag == auth_branch`, the security model is entirely circular. It must be reviewed to ensure it verifies ancestry against `main` and handles API/git-fetch edge cases securely.
- Kind: `artifact`
- Need: `scripts/test-release-authorization.sh`
- Why: Needed to see what structural fixtures and edge-cases (e.g., unauthorized push simulation) are actually being tested.
- Kind: `artifact`
- Need: `release/taskfleet-release.json`
- Why: Missing from the diff but explicitly targeted by the modified assertions in `scripts/validate-distribution-topology.sh`.

---

## Model: gpt-5.6-sol
[model:gpt-5.6-sol] [thread_id:api_88a25b2940ea4cd3bff9f43574201111]

# Release-security review

## Blockers

### 1. The checked-in ledgers falsely describe activation as complete

**Files:**

- `scripts/validate-distribution-topology.sh:42-61`
- `scripts/validate-distribution-topology.sh:76-78`
- `scripts/verify-release-activation.sh:17-25`
- `release/taskfleet-distribution.json:9-15`

The stated operational state is “blocked because no acceptable Homebrew token exists,” but validation now requires:

```jq
.activation == "ready"
.tap_secret_state == "active-proven-r10"
```

Meanwhile the JSON diff records:

```json
"tap_secret_state": "pending-r10-proof"
```

These states are contradictory. The topology validator will fail against the shown JSON, and changing the JSON to satisfy it would falsely claim that the credential proof and activation gate have completed.

More importantly, the previous activation gate has been removed from release execution. Authorization now proves only that the wrapper approved the commit; it does not prove that the distribution topology is activated. If the wrapper records an authorization before the acceptable Homebrew credential exists, a tag can start irreversible crates.io/GitHub publication and then fail at Homebrew.

**Required change:**

Keep hardening state and activation state separate:

```json
{
  "activation": "blocked-r10-homebrew-credential",
  "cargo_dist": {
    "tap_secret_state": "pending-r10-proof",
    "authorization": "wrapper-ref-exact-tag-main-green-ci"
  }
}
```

The validator should require that blocked state for the Phase A candidate. Only the credential-proof commit should move both fields to `ready` / `active-proven-r10`.

Also enforce the activation state before creating an authorization receipt:

```bash
record_release_authorization() {
  ./scripts/verify-release-activation.sh
  # create authorization only after activation is proven
  ...
}
```

The tag verifier should additionally require the activated ledger, so an authorization ref accidentally created while blocked cannot become usable later without an explicit reviewed activation change.

Until then, `scripts/verify-release-activation.sh` must not itself require a falsely completed credential state as part of ordinary hardening validation. Provide separate commands such as:

- `validate-release-hardening.sh`
- `verify-release-activation.sh`

---

### 2. Authorization is recorded before all wrapper resume checks finish

**File:** `scripts/shipshape-release.sh:439-456`

The sequence is currently:

```bash
assert_recorded_checkpoint
assert_repo_identity
assert_remote_tag_absent
record_release_authorization
assert_remote_tag_absent
assert_run_may_resume "$run_id"
shipshape release resume "$run_id" --json
```

If `assert_run_may_resume` fails, or `shipshape release resume` fails before pushing the tag, the durable authorization branch remains. A subsequent manual tag push at that SHA passes the release gate even though it did not occur through a successful wrapper resume.

This weakens “wrapper authorization” into “the wrapper once approached the resume boundary.”

**Required minimum change:**

Move every non-mutating and fallible precondition before receipt creation:

```bash
assert_recorded_checkpoint
assert_repo_identity
assert_run_may_resume "$run_id"
assert_remote_tag_absent
./scripts/verify-release-activation.sh

record_release_authorization

# Minimize the receipt-to-tag window.
assert_remote_tag_absent
shipshape release resume "$run_id" --json
```

Also journal the authorization creation and define recovery explicitly. Once the authorization exists, the only permitted recovery should be resuming the same held run. Abandoning that run while leaving the receipt behind is unsafe.

A stronger design would make receipt creation and tag push one atomic remote transaction, for example an atomic push of the authorization ref and held tag with a lease on `main`. That may conflict with Shipshape’s journal ownership and must not be introduced without updating Shipshape’s recovery contract. If atomic publication is unavailable, document that the receipt authorizes later recovery of one specific release run and bind the receipt to that run in immutable metadata rather than only the version.

---

### 3. The “immutable authorization branch” is not inherently immutable

**File:** `scripts/shipshape-release.sh:263-287`

This comment is stronger than the implementation:

```bash
# This immutable receipt is created ...
git ... push origin "$bump_commit:$ref"
```

A branch can be deleted or force-updated by anyone with sufficient repository privileges. The shell code is create-only for this caller, but the remote object is not immutable.

Even below the repository-admin threat boundary, any automation or maintainer with `contents: write` may be able to create the authorization branch. That makes the security boundary depend on repository permissions and rulesets, not merely this script.

**Required change:**

Create and verify a GitHub ruleset for:

```text
refs/heads/taskfleet-release-authorizations/**
```

It should:

- block updates after creation;
- block deletion;
- block force pushes;
- restrict creation as narrowly as GitHub allows;
- not grant ordinary Actions workflows `contents: write` unnecessarily;
- record the permitted bypass actors.

The repository-admin limitation can be documented honestly, but ordinary workflows and maintainers should not automatically have receipt-creation authority.

Topology validation should verify the ruleset through the GitHub API or consume immutable evidence of its exact ID/configuration. A source-level fixture cannot prove remote immutability.

---

### 4. The design no longer has a machine-enforced “activation blocked” boundary

**Files:**

- `.github/workflows/release.yml:124-131`
- `.github/workflows/publish-crates.yml:30-42`
- `scripts/shipshape-release.sh:439-456`

The new verifier is an authorization gate, not necessarily an activation gate. The diff removes `verify-release-activation.sh` from the release paths and does not show it being called by `verify-release-tag-authorization.sh`.

Therefore, once the wrapper creates the authorization ref, publication appears enabled regardless of whether:

- the acceptable Homebrew token exists;
- the secret name is populated with the correct credential;
- the write/CAS/revert proof completed;
- activation ledgers still say blocked.

This is especially dangerous because the broad classic OAuth token must not be used. If it is already stored under `HOMEBREW_TAP_TOKEN`, the generated job could use it despite the prose prohibition. If no token is stored, crates.io or GitHub legs can become irreversible before Homebrew fails.

**Required change:**

`verify-release-tag-authorization.sh` must compose both conditions:

```bash
./scripts/verify-release-activation.sh
verify_tag_shape
verify_exact_main
verify_authorization_ref
```

Additionally, `record_release_authorization` must refuse while activation is blocked. Defense in depth is appropriate because publication is irreversible.

Before activation:

1. Ensure the broad classic token is not present under `HOMEBREW_TAP_TOKEN`.
2. Provision a repository-scoped fine-grained token with access only to `jarimustonen/homebrew-taskfleet`.
3. Perform the required exact-head CAS-guarded write/revert canary.
4. Record the preflight head, temporary commit, restored head, token resource owner, target repository, and permission names without recording the value.
5. Only then transition the ledgers and allow authorization creation.

---

### 5. Current-main verification has a race and can reject a valid wrapper tag after publication

**File:** `scripts/shipshape-release.sh:442-453`

The wrapper:

1. waits for exact-SHA CI;
2. fetches and checks `origin/main`;
3. performs multiple assertions;
4. creates the authorization branch;
5. performs more assertions;
6. invokes Shipshape resume.

`main` can advance after step 2. If the tag verifier checks *current* main when the Actions job starts, a legitimate tag can be pushed and then rejected merely because another merge lands before the job executes. This is fail-closed, but after tag publication it leaves a partially initiated release requiring manual reconciliation.

It also means the wrapper does not prove that main was unchanged at the instant the tag was pushed.

**Required change:**

At minimum, fetch and recheck `origin/main` immediately before receipt creation and again immediately before resume. Prevent merges during this short critical section through the documented release procedure.

A structurally stronger implementation needs a remote compare-and-swap at tag publication time, such as a lease tied to the observed main SHA. If Shipshape cannot express that, document this as a residual availability/recovery risk and ensure a tag-gate failure is recoverable without retagging.

Do not silently weaken the verifier to stop checking main; that would allow a previously authorized commit to be tagged after main has advanced.

---

### 6. The topology validator does not prove the claimed host failure semantics

**File:** `scripts/validate-distribution-topology.sh:82-91`

The new checks prove only that verifier text appears somewhere:

```bash
grep -F './scripts/verify-release-tag-authorization.sh' \
  .github/workflows/release.yml
```

That does not establish:

- every local artifact matrix leg invokes it;
- every global artifact job transitively depends on all required local jobs;
- host jobs reject `failure` and `cancelled`;
- host jobs cannot publish when a required build is `skipped`;
- no alternate publish path bypasses the gated artifact jobs;
- all three currently planned targets produce a local job.

The stated safety argument depends on those exact DAG and `if:` semantics, so a substring check is inadequate.

**Required change:**

Parse the generated YAML and assert the job graph and conditions structurally. At minimum, fixtures must cover these synthetic outcomes:

| Local build result | Global result | Host allowed? |
|---|---:|---:|
| success | success | yes |
| failure | skipped | no |
| cancelled | skipped | no |
| skipped | skipped | only if explicitly intended |
| mixed matrix success/failure | skipped/failure | no |

Also validate the actual pinned cargo-dist plan contains at least one gated local artifact for every admitted release plan. Do not rely only on the current plan as an architectural guarantee; fail if regeneration changes that property.

---

### 7. Explicit GitHub token permissions are required

**Files:**

- `.github/build-setup.yml:1-6`
- `.github/workflows/publish-crates.yml:30-35`
- `.github/workflows/release.yml:124-131`

Both workflows inject:

```yaml
GH_TOKEN: ${{ github.token }}
```

Whether the verifier succeeds now depends on unspecified surrounding workflow permissions and on what API calls the missing script performs.

If it reads refs through REST, it needs `contents: read`. If it independently checks workflow runs, it may need `actions: read`. Relying on GitHub’s default token policy is nondeterministic across repository settings and future changes.

**Required change:**

Declare least privilege explicitly at job or workflow scope:

```yaml
permissions:
  contents: read
  actions: read # only if CI-run APIs are queried
```

Publishing jobs needing broader permissions should receive those separately. Do not give artifact build jobs `contents: write` merely because release hosting later needs it.

The verifier should distinguish authorization failures from API/permission failures, but both must exit nonzero:

```bash
gh api ... >"$tmp" || {
  echo "unable to query authorization ref; refusing release" >&2
  exit 2
}
```

No empty response, 403, rate limit, malformed JSON, or transient network failure may be interpreted as absence/success ambiguously.

---

### 8. Ref construction needs validation and exact API encoding

**File:** `scripts/shipshape-release.sh:263-287`

The authorization ref is formed by direct interpolation:

```bash
printf 'refs/heads/taskfleet-release-authorizations/%s\n' "$tag"
```

For the intended `v0.6.0`, this is valid. However, the release workflow’s trigger pattern admits more than strict semantic versions, and Git refs permit characters that can be mishandled by REST paths or shell/API interpolation.

The missing verifier must not construct an unencoded GitHub API URL by appending `$tag`. Slashes and URL metacharacters can alter endpoint semantics.

**Required change:**

Validate the expected tag before deriving the ref:

```bash
[[ "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$ ]] || exit 2
git check-ref-format "refs/heads/taskfleet-release-authorizations/$tag" ||
  exit 2
```

Prefer an exact `git ls-remote` query or a correctly encoded API parameter. Verify exactly one ref and exactly one 40/64-character object ID as appropriate; reject duplicate, peeled, malformed, or unexpected output.

The verifier must compare:

- event tag name;
- workspace version-derived tag;
- tag target SHA;
- current canonical main SHA;
- exact authorization ref target SHA.

---

### 9. Generator determinism is asserted weakly

**Files:**

- `.github/build-setup.yml`
- `.github/workflows/release.yml`
- `dist-workspace.toml`
- `scripts/validate-distribution-topology.sh`

The task explicitly prohibits hand-editing generated `release.yml`. The diff is consistent with an inlined build-setup fragment, but there is no shown deterministic regeneration check proving that exact cargo-dist 0.28.2 reproduces both generated files byte-for-byte.

Grepping generated output for expected text does not distinguish generator output from manual edits.

**Required change:**

CI should run the pinned generator in a clean temporary worktree and fail on any diff, for example:

```bash
cargo dist generate --artifacts=local
git diff --exit-code -- \
  .github/workflows/release.yml \
  .github/build-setup.yml
```

Use the exact command supported by cargo-dist 0.28.2 and pin the executable checksum/source as the repository convention requires. Validate that repeated generation is idempotent.

The generated workflow and custom build-setup input should have clear ownership. If `.github/build-setup.yml` is source input rather than generated output, only compare generated derivatives and document that distinction.

---

### 10. Unconditional build-setup authorization may break non-release consumers

**File:** `.github/build-setup.yml:1-6`

The fragment now runs the tag-only verifier unconditionally:

```yaml
run: ./scripts/verify-release-tag-authorization.sh
```

This is safe only if `.github/build-setup.yml` is exclusively consumed by tag-only cargo-dist artifact jobs. If ordinary CI, local generation tests, or future cargo-dist PR planning executes it, those paths will fail unless the verifier has carefully defined non-tag behavior.

Making the verifier return success outside tag contexts would be dangerous if a release job can ever lose or spoof the expected event context.

**Required change:**

Either:

- prove and enforce that this setup fragment is release-artifact-only; or
- add an explicit caller contract passed by generated release jobs and reject every unexpected context.

Do not use a broad “not a tag, therefore success” behavior in code shared with publication paths.

---

## Wrapper design assessment

The authorization-ref design is a credible improvement over cancellation-based gating. Assuming the verifier is implemented correctly, an accidental tag at an unapproved SHA causes required local builds to fail rather than skip, which is the right failure mode.

It is not yet a complete structural boundary because:

- authorization and activation are not visibly composed;
- the receipt is created too early;
- remote immutability is not established;
- the claimed host dependency behavior is not structurally tested;
- the exact verifier implementation is absent;
- a receipt can outlive a failed/abandoned resume attempt.

The stated threat boundary is reasonable: without a dedicated GitHub App or external signing authority, a repository administrator can imitate the wrapper. The design should nevertheless prevent ordinary tag creation, ordinary workflows, and accidental maintainer actions from satisfying authorization.

## Changes required before the Homebrew token can unblock activation

1. Keep all ledgers explicitly blocked and internally consistent.
2. Make activation verification mandatory in both receipt creation and tag verification.
3. Move all wrapper preconditions before receipt creation and define receipt recovery.
4. Protect the authorization namespace with a restrictive GitHub ruleset.
5. Add explicit least-privilege workflow permissions.
6. Add structural DAG/result tests for local/global/host jobs.
7. Add deterministic cargo-dist regeneration checks.
8. Verify exact ref encoding and fail-closed API behavior.
9. Remove any broad classic token from the expected secret name.
10. Complete and record the fine-grained token write/revert canary.
11. Transition activation ledgers in one reviewed commit only after that proof.
12. Run final exact-main CI on that activation commit before permitting receipt creation.

## Context request

- **Kind:** artifact  
  **Need:** `scripts/verify-release-tag-authorization.sh` and `scripts/test-release-authorization.sh` in full.  
  **Why:** Their API error handling, exact-SHA comparisons, ref encoding, event validation, and fixture coverage determine whether the authorization check is actually fail-closed.

- **Kind:** artifact  
  **Need:** Complete `.github/workflows/release.yml`, `.github/workflows/publish-crates.yml`, and `.github/build-setup.yml`.  
  **Why:** The omitted job DAG, `if:` expressions, permissions, publish dependencies, and all consumers of build-setup could invalidate the claimed failure-versus-skipped safety property.

- **Kind:** artifact  
  **Need:** Complete `release/taskfleet-release.json` and `release/taskfleet-distribution.json`.  
  **Why:** The shown validator expects `ready`/`active-proven-r10`, while the diff records `pending-r10-proof`; the complete ledgers are needed to determine every inconsistent activation field.

- **Kind:** artifact  
  **Need:** The cargo-dist 0.28.2 regeneration/check command and its clean-worktree diff output.  
  **Why:** This determines whether `release.yml` is reproducibly generated rather than manually modified.

- **Kind:** artifact  
  **Need:** GitHub ruleset/branch-protection JSON for `taskfleet-release-authorizations/**` and workflow default-token settings.  
  **Why:** These determine whether the purported immutable receipt can be created, changed, or deleted by ordinary maintainers or workflows.

---

## Model: claude-fable-5
[model:claude-fable-5] [thread_id:api_82a1f113da96411e84d1023d7feff1d6]

# Release-Security Review — Phase A revised design

Verdict: the structural direction (verifier inside every local build job, no `secrets: inherit`, no PR trigger) is sound, but this commit contains one hard correctness contradiction, several places where "immutable" and "create-only" claims are not actually enforced, a release-bricking race in the CI-side gate as described, and an uncovered publish path. The core verifier and fixture scripts are not in the diff, so the boundary itself cannot be certified yet.

---

## Blockers

### B1. Ledger/validator state machine is internally contradictory in this commit

- `release/taskfleet-distribution.json` sets `"tap_secret_state": "pending-r10-proof"`.
- `scripts/validate-distribution-topology.sh` now requires `tap_secret_state:"active-proven-r10"` and `.activation == "ready"`, and requires `release/taskfleet-release.json` `.activation == "ready"`.
- `scripts/verify-release-activation.sh` also requires `"active-proven-r10"`.

If topology validation runs in ordinary CI, every run is now red — which deadlocks Phase B ("full post-R9 exact-SHA gate must be green") and therefore the wrapper's own `wait_for_exact_main_ci`. If it doesn't run in CI, the validators are dead code until a future flip, and this commit falsely encodes "ready" requirements while the task states activation is honestly blocked pending a fine-grained token.

Fix: the validators must accept the *current truthful* state (`pending-r10-proof`, blocked activation) and the ready-state requirements must land in the same commit that records the credential-gate evidence. As written, either the ledger lies or CI is red. Also, the diff does not show `.activation` in `taskfleet-distribution.json` changing to `"ready"` — if it still says `"prepared-blocked-r10"`, `validate-distribution-topology.sh:42` fails regardless.

### B2. The authorization ref is not create-only and not immutable

`scripts/shipshape-release.sh`, `record_release_authorization`:

1. **Fast-forward silently mutates the "immutable receipt."** `git push origin "$bump_commit:$ref"` succeeds if the existing ref is an ancestor of `$bump_commit`. Concrete scenario: attempt 1 for v0.6.0 records the ref, tag resume fails; attempt 2 re-plans v0.6.0 with a new bump commit descending from the old one → push fast-forwards, receipt moves. The comment "A normal create-only ref push fails if another writer wins the race" is false for fast-forwards. Fix — use the GitHub API, which is strictly create-only (422 on existing ref):
   ```bash
   gh api "repos/$repo/git/refs" -f ref="$ref" -f sha="$bump_commit"
   ```
   or `git push --force-with-lease="$ref:" origin "$bump_commit:$ref"` (empty lease = "must not exist").

2. **TOCTOU** between `git ls-remote` and the push; two concurrent wrappers with different commits race. The API create closes this.

3. **`ls-remote` tail-matching + `NR == 1` is wrong.** `git ls-remote origin "$ref"` pattern-matches ref-name *tails*; a branch like `x/refs/heads/taskfleet-release-authorizations/v0.6.0` also matches, and `NR == 1` picks an arbitrary line. Match the full ref name:
   ```bash
   remote_oid="$(git ls-remote origin "$ref" | awk -v r="$ref" '$2 == r { print $1 }')"
   ```

4. **Nothing protects the namespace.** Any actor with write access can move or delete `refs/heads/taskfleet-release-authorizations/*` (branch UI, `git push :ref`). "Immutable" is aspirational. Before activation, add a repository **ruleset** on `taskfleet-release-authorizations/**` blocking update/delete (creation via bypass actor only).

### B3. Exact-current-main equality in the CI-side gate bricks releases on benign races

The described gate ("checks … current-main") means: tag pushed → someone merges to main → matrix jobs (queued behind the self-hosted macOS runner, possibly minutes/hours) evaluate `main == tag-commit` → fail. Per the ADR and `item.md`, retag/version-reuse is forbidden and post-tag-push the only recovery is "resume/verify the same journal" — but re-running the workflow re-fails the same check forever. One innocuous merge permanently burns v0.6.0.

The CI-side check should be: **authorization ref exists and equals the peeled tag commit ∧ tag commit is an ancestor of main** (or drop the main comparison entirely in CI — the wrapper already enforced exact-main at `resume_after_gate`, and the authorization ref is the durable proof of that). Fail-closed against non-wrapper tags is preserved by the ref check alone.

Related wrapper trap: `record_release_authorization` runs *after* the main-advance check and *before* tag resume. If resume fails after recording, the ref is orphaned; a re-plan at the same version with a new bump commit hits the `already points at … expected` `exit 2` and the version is unresumable without manual ref surgery (which contradicts immutability). Define this recovery path explicitly.

### B4. The verifier does not cover every publishing job — host can still create a GitHub Release

The build-setup step is injected into **build-local-artifacts only**. `host` accepts *skipped* local/global builds. The stated defense is "the plan always has local artifacts for this pinned graph" — but that's pinned by topology validation running on *main's* CI, not on the tagged commit's workflow. A tag on a commit whose dist config yields an empty artifacts matrix (accident on a weird/old commit, or any commit where `artifacts_matrix.include == null` / `publishing != 'true'`) skips all builds; `host` then runs with `contents: write` and can create a GitHub Release having never executed the verifier.

Worse, the **historical-commit hazard**: `on: push: tags:` runs the workflow file *at the tagged ref*. The pattern `'**[0-9]+.[0-9]+.[0-9]+*'` matches a tag on any ancient commit — including pre-gate orchestratectl-era commits whose `release.yml` had no verifier and targeted `jarimustonen/homebrew-orchestratectl` (which "must never receive generated formula writes"). Whether that succeeds depends entirely on secret scoping.

Required before activation:
1. **Fine-grained `HOMEBREW_TAP_TOKEN` scoped to `jarimustonen/homebrew-taskfleet` only** — already the plan; note it is also the *only* mitigation for old-tap writes from historical workflows.
2. **A tag ruleset on `v*` / the release pattern** restricting tag creation to the release actor. This is the strongest structural boundary available without a GitHub App, and it closes both the historical-workflow hazard and the empty-matrix host path — the per-job verifier then becomes defense in depth rather than the sole gate. Its absence is the biggest gap in this design.

### B5. Core artifacts absent — the boundary cannot be reviewed

`scripts/verify-release-tag-authorization.sh` and `scripts/test-release-authorization.sh` are the security-critical components and are not in the diff. Specific defects to check when supplied:

- **Annotated-tag peeling**: for annotated tag pushes, `GITHUB_SHA`/`after` is the *tag object* SHA, not the commit. Comparing `GITHUB_SHA` to the authorization-ref commit fails (fail-closed, but every legitimate release fails if Shipshape creates annotated tags). Must peel: `git rev-parse "refs/tags/$GITHUB_REF_NAME^{commit}"` or API dereference.
- **Exact-ref lookup**: must use singular `GET /repos/{r}/git/ref/heads/...` (exact) — not `git/matching-refs/` (prefix match: `v0.6.0` would match a hypothetical `v0.6.0-x` authorization) — and still assert the returned `.ref` string exactly.
- **Repository identity**: pin repo ID `R_kgDOS3Iezw`, not just `GITHUB_REPOSITORY` (names are reassignable post-rename).
- **No `|| true` / unchecked `gh api` exit codes; fail on missing `GH_TOKEN`.**

---

## Major

### M1. `GH_TOKEN` permission scopes may fail-closed every legitimate release

Generated `release.yml` sets an explicit `permissions:` block (cargo-dist emits `contents: write` only); unspecified scopes are `none`. If the verifier checks "green CI" via `GET /actions/runs` or check-runs, it needs `actions: read` / `checks: read` → 403 → every release fails. If the CI-side verifier only reads refs, `contents` suffices — but then prefer `git ls-remote origin` via checkout's persisted credentials and drop the API dependency entirely. `publish-crates.yml`'s permissions block is not shown; verify it.

### M2. Authorization branches trigger workflows and accumulate

`refs/heads/taskfleet-release-authorizations/v0.6.0` is a normal branch: it triggers any `on: push` workflow without branch filters (wasted CI, potential side effects), appears in branch UI, and is a valid PR source. Either constrain via the B2 ruleset and add push-path filters to ordinary CI, or move to a non-branch namespace (`refs/taskfleet-authorizations/*` — git push/ls-remote support it) accepting reduced ruleset/UI coverage. Document the trade-off; the current design is silent.

### M3. `scripts/shipshape-release.sh:442-446` — `git rev-parse origin/main` ambiguity

A local branch literally named `origin/main` would shadow the remote-tracking ref. Use `git rev-parse refs/remotes/origin/main`. Also, when this check fails ("release remains untagged"), the run is permanently unresumable (main will never move back); state whether the held local tag/journal is cleaned or the run must be explicitly abandoned — `exit 1` with no cleanup leaves a limbo state.

### M4. `verify-release-activation.sh` appears orphaned or self-blocking

`publish-crates.yml` swapped it out for the tag-authorization verifier. If nothing invokes it anymore, it's drift; if the wrapper invokes it pre-cut, then per B1 it currently fails against the pending-state ledger — possibly intended as the credential block, but that intent is implicit and duplicated (`test-release-authorization.sh` is now run from *both* validators — one owner, please).

### M5. Determinism of the generated workflow is asserted, not proven here

The diff hand-shows edits to `release.yml`. They're *consistent* with regeneration under `pr-run-mode = "skip"` + removed `plan-jobs`, but nothing in the diff demonstrates a `dist generate --check`-style guard, and `dist-workspace.toml` doesn't show the `github-build-setup` key (presumably pre-existing). Confirm the checked-in workflow is byte-identical regenerated output and that CI enforces it (cargo-dist's plan job errors on stale CI unless `allow-dirty` is set — verify `allow-dirty` is absent).

---

## Minor

- `validate-distribution-topology.sh`: the `grep -A12 '^on:'` window checks are layout-fragile against future cargo-dist formatting; acceptable while pinned at 0.28.2, but they'll silently mis-scope on version bumps.
- `test-release-authorization.sh >/dev/null` inside a validator discards fixture stdout, weakening failure diagnostics; keep stderr visible and echo the fixture name on failure.
- `publish-crates.yml`: publish steps aren't shown — confirm every `cargo publish` step is behind the same `GITHUB_REF_TYPE = tag` condition as the verifier, not just the verification step.
- Build-setup step ordering: the verifier runs after checkout/toolchain steps in each matrix job; it should be the *first* injected step (it is, within build-setup) — confirm cargo-dist injects build-setup before any dependency-fetching/build steps that could execute tag-controlled build scripts. (Irrelevant to the write-access threat model, relevant to accident containment.)

## Assessment of the boundary claim

Against **accidental/manual/non-wrapper tags by honest writers**: credible once B2–B5 are fixed — the tag alone cannot produce the authorization ref, all three local builds fail, host rejects failure, crates leg independently rejects. Against **any actor with write access**: not a boundary (they can create the ref or gut the script in the tagged commit) — correctly acknowledged, but the ledger value `authorization: "wrapper-ref-exact-tag-main-green-ci"` should be documented with that threat-model bound. A `v*` tag ruleset (B4) meaningfully raises the bar within GitHub's native primitives and should be a precondition of flipping the ledgers to `ready`.

## Context request

- Kind: artifact — Need: `scripts/verify-release-tag-authorization.sh` — Why: determines B3 (main-equality bricking), B5 (tag peeling, exact-ref lookup, repo-ID pinning, error handling), M1 (token scopes); could upgrade or clear several blockers.
- Kind: artifact — Need: `scripts/test-release-authorization.sh` — Why: fixture-quality finding is currently unassessable; determines whether the fail-closed paths (missing ref, wrong SHA, annotated tag, moved main) are actually tested.
- Kind: artifact — Need: full generated `.github/workflows/release.yml` (top-level `permissions:`, `host`, `publish-homebrew`, `announce` job `if:` conditions and `needs:`) — Why: confirms/refutes B4 (host-on-skipped path) and M1 (token scopes).
- Kind: artifact — Need: full `.github/workflows/publish-crates.yml` (triggers, `permissions:`, publish steps) — Why: M1 and M5-adjacent; whether publish steps are tag-gated.
- Kind: artifact — Need: full `release/taskfleet-distribution.json` and `release/taskfleet-release.json` — Why: B1 — whether `.activation` fields match the new validator requirements in this commit.
- Kind: artifact — Need: `scripts/validate-release-topology.sh` and the workflow(s)/locations that invoke `validate-distribution-topology.sh` — Why: B1 — whether ordinary CI is red in this commit.
- Kind: artifact — Need: `scripts/shipshape-release.sh` in full (`wait_for_exact_main_ci`, tag creation — annotated vs lightweight — `assert_run_may_resume`, abandon path) — Why: B3/B5 (tag peeling correctness) and M3 (recovery semantics).
- Kind: clarification — Need: are repository rulesets (tag creation on `v*`, branch-namespace lock on `taskfleet-release-authorizations/**`) planned as activation preconditions, and is the future `HOMEBREW_TAP_TOKEN` scoped to `homebrew-taskfleet` only? — Why: B4 and B2.4 — determines whether the historical-workflow/old-tap hazard and receipt mutability are closed before ledgers flip to `ready`.

---

## Model: deepseek-v4-pro
[model:deepseek-v4-pro] [thread_id:api_12904303436345528272cba2946bff18]

## Blockers

### 1. Activation ledger and topology validator are mutually inconsistent as committed

`scripts/validate-distribution-topology.sh` now requires:

```jq
.activation == "ready" and
...
tap_secret_state:"active-proven-r10"
```

But the same diff leaves `release/taskfleet-distribution.json` at:

```json
"tap_secret_state": "pending-r10-proof"
```

and does not change the existing top-level `.activation` value (likely still `prepared-blocked-r10`). `scripts/verify-release-activation.sh` also now requires `tap_secret_state == "active-proven-r10"`.

At minimum, `tap_secret_state` is contradictory; likely `.activation` is too. If `validate-distribution-topology.sh` runs in CI or from `validate-release-topology.sh`, Phase A fails closed immediately.

This is not a valid “blocked until token” state. Either:

- keep both validators at `pending-r10-proof` / blocked until a fine-grained token exists, then move them together with the ledger, or
- actually set the ledger to `active-proven-r10`, which conflicts with the issue statement that activation remains blocked because no acceptable token exists.

The consistent Phase A state should be:

```json
"activation": "prepared-blocked-r10",
"tap_secret_state": "pending-r10-proof"
```

while the structural validator still expects those blocked values.

---

## 2. “Current-main == tag commit” is a release-killing TOCTOU check

The design says the tag gate checks **current main**. The wrapper only proves:

```sh
wait_for_exact_main_ci "$bump_commit"
git fetch origin +refs/heads/main:refs/remotes/origin/main
[[ "$(git rev-parse origin/main)" == "$bump_commit" ]]
```

This is true at wrapper time. But the actual tag workflow runs later, after the tag is pushed. There is no lock preventing `main` from advancing in between.

Sequence:

1. Wrapper verifies `main == bump_commit`, records auth ref, resumes Shipshape, pushes tag.
2. Ordinary development pushes another commit to `main`.
3. GitHub schedules the tag workflow.
4. Verifier sees `origin/main != tag_commit`, fails.
5. The tag is already public and cannot be retagged.

This can block crates.io and Homebrew for a legitimate, wrapper-authorized release just because `main` moved a few seconds later. Self-hosted queueing delays make this worse.

A tag is an immutable trigger; the verifier must not require the live head of `main` to still equal the tag commit. Instead, the tag should be verified by:

- exact canonical repo;
- exact tag name and workspace version;
- peeled tag commit equals the authorization ref target;
- the authorization ref exists at that peeled commit;
- optionally, exact-SHA CI/green evidence recorded by the wrapper, without `origin/main == tag`.

For example:

```sh
tag_commit="$(git rev-parse "refs/tags/$GITHUB_REF_NAME^{commit}")"
auth_ref="refs/heads/taskfleet-release-authorizations/$GITHUB_REF_NAME"
auth_commit="$(git ls-remote origin "$auth_ref" | awk 'NR == 1 { print $1 }')"

[[ "$tag_commit" == "$auth_commit" ]] || fail
```

Do not compare `origin/main` in the tag workflow.

---

## 3. Authorization ref is a mutable branch, not an immutable receipt

`record_release_authorization()` creates:

```
refs/heads/taskfleet-release-authorizations/<tag>
```

Branches are neither immutable nor protected by default. Any repository writer can:

- force-move the branch;
- delete it;
- accidentally push to it through normal branch operations.

That makes the comment “immutable receipt” false. It also means ordinary branch-push workflows may trigger when the auth ref is created, depending on their `on.push` filters.

Use a ref type that is not normal branch history and can be protected differently, such as a tag outside the release trigger pattern:

```sh
release_authorization_ref() {
  printf 'refs/tags/release-authorizations/%s\n' "$tag"
}
```

The release workflow trigger pattern is:

```yaml
tags:
  - '**[0-9]+.[0-9]+.[0-9]+*'
```

so `refs/tags/release-authorizations/v0.6.0` will not trigger it. Add GitHub tag protection for `release-authorizations/*`.

Even then, the holder of repository write access can move tags, but that is a real privilege boundary, not an accidental branch mutation surface.

---

## 4. Stranded authorization refs can outlive the wrapper transaction

Current order in `resume_after_gate()`:

```sh
assert_recorded_checkpoint
assert_repo_identity
assert_remote_tag_absent
record_release_authorization
assert_remote_tag_absent
assert_run_may_resume "$run_id"
shipshape release resume "$run_id" --json
```

If `assert_run_may_resume` or `shipshape release resume` fails before the tag push, the authorization ref remains at a commit with green exact-SHA CI. Under the current verifier design, that may be enough for a later non-wrapper tag push to pass, especially if the “current-main” check is removed as recommended.

Move the non-mutating validations before recording the auth ref:

```sh
assert_recorded_checkpoint
assert_repo_identity
assert_run_may_resume "$run_id"
assert_remote_tag_absent
record_release_authorization
assert_remote_tag_absent
shipshape release resume "$run_id" --json
```

Additionally, add an abort/cleanup path that deletes the auth ref if resume fails before a tag is pushed. Distinguish a newly created ref from a pre-existing ref so cleanup does not delete another valid run’s receipt.

---

## 5. GitHub token permissions and self-hosted runner dependencies are unverified

The new verifier runs inside:

- `.github/build-setup.yml`
- `.github/workflows/publish-crates.yml`
- `.github/workflows/release.yml`

with:

```yaml
GH_TOKEN: ${{ github.token }}
```

But the diff does not show that the relevant workflows grant the token sufficient permissions.

Required, depending on the missing verifier implementation:

- `contents: read` — to read tags/refs/commits
- `actions: read` or `checks: read` — if the verifier checks CI status or check runs
- `metadata: read` — usually default

If the workflows have top-level `permissions: {}` or omit these scopes, legitimate tag builds fail with 403/404.

Also, the self-hosted macOS runner must have:

- `gh` installed if the verifier uses it, or the verifier should use `git`/`curl`
- network access to the GitHub API
- the repository checkout configured with a usable origin URL

A self-hosted proof runner can be minimal; relying on `gh` introduces an undocumented runner dependency.

---

## 6. Broad Homebrew token risk is not enforced by the ledger value

`release/taskfleet-distribution.json` says `pending-r10-proof`, but the generated release workflow still references:

```yaml
token: ${{ secrets.HOMEBREW_TAP_TOKEN }}
```

If the broad classic OAuth token is still installed in repository secrets, any tag that passes the new authorization path can cause the Homebrew job to use that broad token. The JSON ledger does not remove, revoke, or block the secret.

Before accepting activation, the actual secret state must be changed:

- remove/rotate the broad token, or
- rename and intentionally leave the new fine-grained secret absent until available.

A JSON `tap_secret_state` value is documentation, not an enforcement boundary.

---

## 7. Publish-crates workflow dependency chain may not cover all publish jobs

The diff shows the verifier step at the start of a job in `.github/workflows/publish-crates.yml`, but not the job name or downstream `needs`.

If later jobs (`publish-core`, `publish`, `publish-orchestratectl`) can run independently of this verification job, they may publish without authorization.

The workflow should have a dedicated `verify` job that all crate-publishing jobs use via `needs`, for example:

```yaml
jobs:
  verify:
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@1.98.0
      - name: Verify topology and wrapper authorization
        env:
          GH_TOKEN: ${{ github.token }}
        run: |
          ...
          ./scripts/verify-release-tag-authorization.sh

  publish-core:
    needs: verify
    ...

  publish-taskfleet:
    needs: verify
    ...

  publish-orchestratectl:
    needs: verify
    ...
```

This must be confirmed; the current diff is not sufficient evidence.

---

## 8. Missing verifier and fixture scripts prevent a complete review

The actual gate is implemented in `scripts/verify-release-tag-authorization.sh`, which is not included in the diff. The fixture test `scripts/test-release-authorization.sh` is also missing.

Critical unknown details include:

- Does it blindly use `GITHUB_SHA` for tags?
- Does it peel annotated tags to commits?
- Does it compare `origin/main` to the tag commit?
- Does it use `gh api` with correctly URL-encoded refs?
- Does it query CI status or check runs?
- Are negative fixture cases tested: wrong repo, wrong tag, wrong commit, missing ref, force-moved auth ref, main advanced, annotated tag, malformed ref?
- Are fixtures hermetic, or do they depend on live GitHub state?

The fixture test should be deterministic and structural, with explicit negative cases, not a live-network verifier invocation.

---

## Additional risks

### Annotated tag handling

If Shipshape’s held tag is annotated, `GITHUB_SHA` on a tag push may be the tag object SHA, not the commit SHA. The verifier must explicitly peel:

```sh
git rev-parse "refs/tags/$GITHUB_REF_NAME^{commit}"
```

and compare that to the auth ref target.

### GitHub API ref encoding

The auth branch name contains slashes:

```text
taskfleet-release-authorizations/v0.6.0
```

If the verifier uses:

```sh
gh api "repos/$GITHUB_REPOSITORY/git/ref/heads/$ref"
```

with `ref` containing `refs/heads/...`, the endpoint may be wrong. Prefer `git ls-remote origin "$ref"` in the verifier, which avoids REST encoding entirely.

### Generator determinism

The release workflow must be regenerated with exact cargo-dist 0.28.2 and `pr-run-mode = "skip"`, not hand-edited. The current diff removes `pull_request` and the reusable gate from `release.yml`, but the repository also modifies `.github/build-setup.yml` and the generated workflow. Add a CI check that runs:

```sh
cargo dist generate
git diff --exit-code -- .github/workflows/release.yml
```

If the generated file cannot be produced deterministically in CI, the source of record is ambiguous.

### Auth ref push trigger side effects

Creating `refs/heads/taskfleet-release-authorizations/v0.6.0` may trigger any workflow with `on.push.branches` set to `**`, causing extra CI runs. Switching to a tag-type ref outside the release trigger pattern avoids this.

---

## Context request

- **artifact:** `scripts/verify-release-tag-authorization.sh`
  - **why:** Determine the exact tag/commit checks, API usage, token scopes, annotated-tag handling, and whether it compares `origin/main`.
- **artifact:** `scripts/test-release-authorization.sh`
  - **why:** Evaluate fixture quality, hermeticity, and negative authorization cases.
- **artifact:** full `.github/workflows/publish-crates.yml`
  - **why:** Confirm the verification job is named `verify` and that all publish jobs depend on it; confirm job permissions.
- **artifact:** full `.github/workflows/release.yml`
  - **why:** Confirm top-level `permissions`, host job conditions, all triggers, and whether the build-setup step is generated or hand-inserted.
- **artifact:** full `release/taskfleet-distribution.json` and `release/taskfleet-release.json`
  - **why:** Confirm actual top-level `.activation` and `tap_secret_state` values.
- **artifact:** `scripts/validate-release-topology.sh`
  - **why:** Determine whether it invokes the now-inconsistent distribution validator in normal tag CI.
- **clarification:** Are branch/tag protection rules configured on `jarimustonen/taskfleet` for `main`, `v*`, and `release-authorizations/*`?
  - **why:** The current design depends on ref-creation push failure and cannot rely on branch immutability.
- **clarification:** Is `HOMEBREW_TAP_TOKEN` still present and still a broad classic token?
  - **why:** The ledger says blocked, but the workflow still references the secret and could use it if a valid tag is authorized.
