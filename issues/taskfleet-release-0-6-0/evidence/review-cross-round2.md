[thread_id:group_547a01f5f6fe402184394af94e246039]

## Model: gemini-3.1-pro-preview
[model:gemini-3.1-pro-preview] [thread_id:api_f2ae0714a5ea4fab80b43e2299530fe8]

### 1. Remaining Blocking Findings (Phase A/B)

The most critical remaining blockers stem from broken CI configuration and incorrect API error handling introduced in the new changes. 

*   **Incoherent Validator Mode in CI:** In `.github/workflows/ci.yml`, the new `release-topology` job hardcodes the `active` validation state (`./scripts/validate-distribution-topology.sh ... active`). However, `release/taskfleet-distribution.json` checks in the state `"tap_secret_state": "pending-r10-proof"`. The script asserts `tap_secret_state` must be `active-proven-r10` when `active` is passed, ensuring this CI job will **fail on every PR** and permanently block merges.
*   **Broken GitHub API 404 Handling:** The script `scripts/shipshape-release.sh` relies on `$response` being empty if a ref does not exist. However, `gh api` outputs a JSON error body (`{"message": "Not Found"}`) to stdout on a 404 response. The check `[[ -z "$response" ]]` will evaluate to false, skipping the `POST` payload. The subsequent `jq -er .ref` command will fail to parse `.ref` from the error JSON, crashing the script. This breaks all valid releases.

### 2. Wrong / Exaggerated Claims

*   **Exaggerated claim:** *"cargo-dist's permissive skipped-build host condition... authorization failure then yields local failure, which host rejects."*
    *   **Why it's wrong:** This fundamentally misunderstands the cargo-dist pipeline execution order. By removing `custom-taskfleet-release-gate` from `.github/workflows/release.yml`, the `plan` job is now entirely ungated. The `plan` job executes `cargo dist host --steps=create`, which creates the public GitHub Release draft/object. Because `plan` runs *before* `build-local-artifacts` (where `verify-release-tag-authorization.sh` now resides), an unauthorized tag will successfully create an empty GitHub Release on the repository before the local build can fail and halt artifact upload.
*   **Wrong claim:** *"calling it cryptographic proof of the wrapper is too strong is a vulnerability."*
    *   **Why it's wrong:** Under the explicit repository-admin trust boundary, this is structurally correct. Because an admin has the rights to bypass branch protection, forge the exact-main commit, push the tag, and push the durable `taskfleet-release-authorizations/*` ref, the mechanism is a *policy and structural* proof, not a cryptographic one. It ensures correct pipeline flow but does not protect against malicious admins.

### 3. New Concrete Release-Safety Bugs

**Bug A: Inverted Logic on `gh api` 404 Response**
*File: `scripts/shipshape-release.sh`* lines 275-278
```bash
  # BUG: gh api outputs {"message":"Not Found"} to stdout on 404. 
  # || true masks the exit code. [[ -z "$response" ]] is FALSE.
  response="$(gh api "repos/$expected_repo/git/ref/heads/$ref_name" 2>/dev/null || true)"
  if [[ -z "$response" ]]; then
```
**Solution:** Check the exit code of `gh api` instead of capturing and checking for an empty string:
```bash
  if ! response="$(gh api "repos/$expected_repo/git/ref/heads/$ref_name" 2>/dev/null)"; then
    response="$(gh api --method POST "repos/$expected_repo/git/refs" \
      -f ref="$ref" -f sha="$bump_commit")" || {
      echo "could not create release authorization $ref" >&2
      exit 2
    }
  fi
```

**Bug B: CI Validator Mode Mismatch**
*File: `.github/workflows/ci.yml`* line 166
The topology is verified against `active`, but the checked-in JSON is in the pending/prepared state.
**Solution:** Pass `prepared` until the final R10 activation commit.
```diff
- ./scripts/validate-distribution-topology.sh "$RUNNER_TEMP/taskfleet-dist-plan.json" active
+ ./scripts/validate-distribution-topology.sh "$RUNNER_TEMP/taskfleet-dist-plan.json" prepared
```

**Bug C: Ungated Release Object Creation (TOCTOU / Pipeline flaw)**
*File: `.github/workflows/release.yml`*
By removing `custom-taskfleet-release-gate` and shifting the authorization check into the `build-local-artifacts` job, the `plan` job (which requires `contents: write`) runs unconditionally when a tag is pushed. This exposes the repository to state-mutation (empty GitHub releases) by unauthorized tags.
**Solution:** Either restore a pre-plan validation job that fails the workflow before `plan` executes, or configure `cargo-dist` to invoke the `verify-release-tag-authorization.sh` script as a step *inside* the `plan` job before `dist host --steps=create` runs.

### 4. Top Three Priorities

1.  **Fix the `gh api` 404 error handling in `shipshape-release.sh`:** Immediate structural blocker; releases cannot be authorized or recorded without this fix because the script crashes trying to parse the JSON error payload.
2.  **Fix the CI topology validation in `ci.yml`:** Change `active` to `prepared`. Current state prevents any pull request from passing CI.
3.  **Restore pre-`plan` authorization gating in `release.yml`:** Stop unauthorized tags from triggering the `plan` job, which prevents cargo-dist from mutating repository state (creating Release objects) before the wrapper's authorization ruleset is verified.

---

## Model: gpt-5.6-sol
[model:gpt-5.6-sol] [thread_id:api_88a25b2940ea4cd3bff9f43574201111]

# Revised release-security assessment

## 1. Findings that remain truly blocking

### Blocker: `release-topology` CI always requests `active` while the honest repository state is `prepared`

**File:** `.github/workflows/ci.yml:163-168`

```bash
./scripts/validate-distribution-topology.sh \
  "$RUNNER_TEMP/taskfleet-dist-plan.json" active
```

The checked-in ledgers still contain:

```json
// release/taskfleet-release.json
"activation": "blocked-r8-r9-r10"

// release/taskfleet-distribution.json
"activation": "prepared-blocked-r10",
"tap_secret_state": "pending-r10-proof"
```

Therefore every PR and main push containing this CI change will fail `release-topology`. This prevents the hardened prepared state from receiving the exact-main CI that Phase B requires.

Changing the CI argument to `prepared` is insufficient because the final atomic activation commit would then fail.

Select the mode from the exact, coherent ledger pair and reject every partial combination:

```bash
release_state="$(jq -er .activation release/taskfleet-release.json)"
distribution_state="$(jq -er .activation release/taskfleet-distribution.json)"
secret_state="$(jq -er .cargo_dist.tap_secret_state release/taskfleet-distribution.json)"

case "$release_state:$distribution_state:$secret_state" in
  blocked-r8-r9-r10:prepared-blocked-r10:pending-r10-proof)
    state=prepared
    ;;
  ready:ready:active-proven-r10)
    state=active
    ;;
  *)
    echo "incoherent release activation state" >&2
    exit 2
    ;;
esac

./scripts/validate-distribution-topology.sh \
  "$RUNNER_TEMP/taskfleet-dist-plan.json" "$state"
```

This does not weaken the atomic transition: each accepted tuple remains exact, while mixed states fail.

---

### Blocker for activation, not for prepared Phase A/B: the acceptable Homebrew credential and canary receipt are still absent

The implementation can merge and run prepared-state Phase A/B once the CI bug above is fixed. It cannot transition to `ready`, create a valid authorization receipt, or cut the release until all of these exist:

1. Repository-scoped fine-grained credential for only `jarimustonen/homebrew-taskfleet`.
2. Required contents permission and no unnecessary repository scope.
3. Reversible exact-head/CAS write canary.
4. Evidence that the tap returned to its original head.
5. Sanitized credential-scope and secret-name receipts.
6. Atomic ledger transition to:
   - `release.activation = "ready"`
   - `distribution.activation = "ready"`
   - `tap_secret_state = "active-proven-r10"`
7. Green final exact-main CI on that active commit.

The inert random secret is safe and honest, but it is not proof of deployability.

---

### Conditional activation blocker: server policy must be verified in the tag workflow before any mutation-capable path

The prompt states that the live policy verifier is now called before receipt creation and by tag jobs. That resolves the earlier server-policy blocker **if** every relevant tag workflow invokes it before its mutation-capable operations.

For crates.io, the verifier in `release-version` is a transitive prerequisite of `publish-core`, so that path is covered.

For cargo-dist, the generated source shown still first verifies authorization inside `build-local-artifacts`; `plan` runs first. Given the explicit assumptions that:

- all tag creation is server-restricted;
- repository administrators are trusted;
- `dist host --steps=create` is locally non-mutating;
- the exact generator is checksum-pinned and reproduced in CI;

this is no longer a Phase A blocker. It remains an accepted cargo-dist 0.28.2 limitation, not proof that every workflow operation is gated before execution.

The activation evidence must state this accurately. It should not claim that authorization executes before all cargo-dist code.

---

## 2. Prior claims that are wrong or exaggerated

### “An older commit can bypass the hardened workflow, so activation is structurally unsafe”

This is exaggerated after an active ruleset covers **all tags**, assuming its creation restrictions are verified and repository administrators are inside the trust boundary.

An ordinary writer or workflow cannot create a tag pointing at an old commit. A trusted administrator can bypass or change the policy, but that is explicitly out of scope. The same administrator could also replace workflows, ledgers, rulesets, or secrets, so treating historical workflow selection as a separately solvable malicious-admin attack is misleading.

There is still an operational risk of a trusted privileged actor manually creating a bad tag. The ruleset cannot distinguish that action from the wrapper without a dedicated identity. That is an accepted limitation, not a defect unique to historical commits.

---

### “The authorization branch is forgeable, so the design is not structural”

No longer correct if the active authorization ruleset:

- restricts creation;
- denies update and deletion;
- denies force pushes;
- excludes ordinary Actions tokens and writers;
- is checked live before receipt creation and in tag jobs.

Under the stated administrator trust boundary, the immutable version-scoped ref is a credible structural authorization coordinate.

It is not cryptographic proof that the local wrapper executed. It proves only that the protected authorization coordinate exists at the tag commit. The repository metadata string:

```json
"authorization": "wrapper-ref-exact-tag-main-green-ci"
```

should be interpreted as the trusted release protocol, not as an independently verifiable cryptographic attestation of the wrapper or CI run.

A more precise name would be:

```json
"authorization": "protected-ref-after-wrapper-exact-main-ci"
```

---

### “The authorization ref must be atomically created with the tag”

Exaggerated as a blocker.

Atomic create-ref correctly prevents replacement or fast-forward races. A failure after receipt creation leaves an authorized-but-untagged coordinate. With:

- immutable tag and authorization namespaces;
- one version-scoped coordinate;
- one held Shipshape journal;
- explicit same-journal recovery;
- no retag/version reuse;

that is a valid resumable-saga state.

The diagnostics do not technically prevent a trusted privileged actor from manually pushing the exact tag. No repository-local design can distinguish that actor from the wrapper under the stated trust boundary. The contract must remain procedural:

> Once authorization exists, only the same Shipshape journal may reconcile the authorized coordinate.

Do not describe the ref as proof that Shipshape pushed the tag.

---

### “A post-tag live-main check is required”

Wrong for this release protocol.

Checking live `main` in the tag job creates an availability and version-burning race: a valid tag could be rejected solely because `main` advanced after its push.

The correct release-time protocol is:

1. bump SHA is on `main`;
2. exact-SHA main CI succeeds;
3. wrapper re-fetches and confirms `main` still equals the bump SHA;
4. protected authorization ref is created;
5. held tag is resumed.

The protected ref preserves authorization for that commit. A later `main` advance must not invalidate a published tag.

There remains a small race between the final main check and receipt creation. Under the trusted-admin model and normal concurrent merges, this is principally an audit-precision issue rather than a publication-safety failure: the authorized commit was green and had been exact main immediately before the receipt. A merge freeze can close it operationally if the acceptance language requires identity at the exact instant of receipt creation.

---

### “Workflow-wide `contents: write` alone blocks Phase A”

Too strong under the explicit constraints.

It is undesirable that cargo-dist 0.28.2 grants every job `contents: write`, especially build jobs executing dependencies. Job-scoped least privilege would be better. But with:

- tag creation restricted server-side;
- authorization refs immutable and restricted;
- repository administrator trusted;
- exact generated topology pinned;
- no PR execution;
- no inherited secrets;
- Homebrew using a separate token;

this is a bounded generator limitation rather than an automatic Phase A blocker.

It remains supply-chain exposure: compromised build code could mutate other unprotected repository content or releases. The activation record should explicitly accept that cargo-dist 0.28.2 limitation rather than claiming least privilege.

---

### “Permissive skipped-build host handling is inherently unsafe”

Not for the pinned graph as currently constrained.

The actual safety chain is:

1. the admitted plan has a non-empty local matrix;
2. every local matrix job executes the authorization verifier;
3. verifier rejection produces `failure`, not `skipped`;
4. global build depends on local and therefore skips/fails;
5. host accepts local `success` or `skipped`, but rejects local `failure`;
6. topology validation pins the graph and matrix assumptions.

Thus an unauthorized tag cannot exploit the `skipped` allowance in this exact topology. The condition remains fragile to future generator/plan changes, so the plan and generated workflow checks are load-bearing.

---

## 3. New concrete release-safety bugs

### New bug: checksum verification can still execute the wrong `dist` binary

**File:** `.github/workflows/ci.yml:149-160`

The archive itself is checksum-pinned, but binary selection is not tied to the extracted archive:

```bash
tar -xJf "$RUNNER_TEMP/cargo-dist.tar.xz" -C "$RUNNER_TEMP"
dist_bin="$(find "$RUNNER_TEMP" -type f -name dist -perm -u+x -print -quit)"
```

`RUNNER_TEMP` is shared with earlier and action-created temporary files. `find ... -print -quit` may select an unrelated executable named `dist`. Selection order is not a security contract.

Extract into a new empty directory and require exactly one expected executable:

```bash
archive="$RUNNER_TEMP/cargo-dist.tar.xz"
extract_dir="$RUNNER_TEMP/cargo-dist-0.28.2"
mkdir -m 700 "$extract_dir"

curl --proto '=https' --tlsv1.2 -LsSf "$url" -o "$archive"
printf '%s  %s\n' \
  'c1ba58b5458646ab86f5e0c6f8641c7162eb04d92251fcd949db2c018b9e90e8' \
  "$archive" | sha256sum --check

tar -xJf "$archive" -C "$extract_dir"

mapfile -t candidates < <(
  find "$extract_dir" -type f -name dist -perm -u+x -print
)
[[ ${#candidates[@]} -eq 1 ]] || {
  echo "expected exactly one cargo-dist executable" >&2
  exit 2
}

echo "DIST_BIN=${candidates[0]}" >> "$GITHUB_ENV"
```

Also run:

```bash
"${candidates[0]}" --version
```

and assert exact `0.28.2`.

---

### New bug: exact-main CI can potentially accept a superseded run result

**Files:**

- `.github/workflows/ci.yml` concurrency configuration from the complete file
- `scripts/shipshape-release.sh`, `wait_for_exact_main_ci`

CI uses:

```yaml
concurrency:
  group: ci-${{ github.workflow }}-${{ github.ref }}
  cancel-in-progress: true
```

All `main` pushes share the same group. A later main push cancels the bump commit’s CI. The wrapper correctly treats cancellation as failure. That part is safe.

However, `wait_for_exact_main_ci` obtains one run ID before watching it:

```bash
gh run list ... --commit "$sha" --limit 1
gh run watch ... "$id" --exit-status
```

If the selected run is a previously successful rerun or duplicate push run for the same SHA, the wrapper accepts it as long as its `headSha`, branch, and event match. This may be acceptable, but it is not necessarily the exact CI run triggered by the wrapper’s main push.

For stronger evidence, require:

- status `completed`;
- conclusion `success`;
- workflow database/path identity;
- run attempt;
- creation time after the bump’s main push, if that timestamp is available;
- all required jobs present, especially `release-topology` and self-hosted macOS.

At minimum, record the selected run and job IDs as immutable Phase C evidence. This is not a fail-open publication bug because the SHA is exact, but it can weaken the claimed “final push CI” provenance.

---

### New bug: the policy identifiers are statically validated, but ID equality alone does not prove policy content

**Files:**

- `release/taskfleet-distribution.json`
- `scripts/validate-distribution-topology.sh:48-59`
- `scripts/verify-release-activation.sh:17-29`

These checks prove only that the ledgers name rulesets `22234415` and `22234417`. A ruleset can retain its ID while its enforcement mode, conditions, bypass actors, or rules are edited.

The prompt states that `verify-release-github-policy.sh` checks the live rulesets. That live check is the actual security control; the static ID assertions are only coordinates.

Ensure the live verifier checks at least:

- `enforcement == "active"`;
- target is the correct ref type;
- include patterns cover every tag and exactly the authorization namespace;
- no unintended exclude patterns;
- creation restriction;
- update/deletion/force-push restrictions;
- exact bypass actor set and bypass modes;
- repository identity.

The active topology CI should invoke the live policy verifier if `GITHUB_TOKEN` can read the endpoint. If not, wrapper and tag-job checks remain fail-closed, but Phase B should store a sanitized API receipt separately.

---

## 4. Top three priorities

### 1. Fix prepared/active CI mode selection

This is the immediate implementation blocker. As written, the hardened prepared commit cannot pass CI and therefore cannot satisfy Phase B.

Use exact tuple selection and reject all partial ledger transitions.

### 2. Complete and prove the Homebrew credential boundary

Do not flip activation until the repository-scoped credential exists and the reversible exact-head canary proves write capability without leaving a mutation. Ensure the broad classic token never enters Actions.

The final ledger transition, generated topology check, live ruleset verification, self-hosted main CI, and exact-main CI must all pass on the same active commit.

### 3. Tighten the CI/tooling evidence path

At minimum:

- extract checksum-pinned cargo-dist into an isolated directory;
- assert exact binary version;
- verify live ruleset contents, not only IDs;
- record exact CI run/job IDs including release topology and self-hosted macOS;
- retain executable malformed-ref and unsafe-topology fixtures.

## Bottom line

After the active rulesets and live policy verifier, the authorization-ref architecture is a credible fail-closed boundary against ordinary and accidental unauthorized tag creation within the stated repository-admin trust model. The remaining authorization-to-tag gap is a deliberate resumable-saga state, not a structural bypass, provided recovery is restricted procedurally to the same Shipshape journal.

The immediate code blocker is the unconditional `active` CI validation. The external activation blocker remains the missing repository-scoped Homebrew credential and canary proof. The checksum-pinned cargo-dist job also needs isolated extraction so the checked archive is necessarily the binary that CI executes.

---

## Model: claude-fable-5
[model:claude-fable-5] [thread_id:api_82a1f113da96411e84d1023d7feff1d6]

# Final-round assessment

## 1. Findings that remain genuinely blocking for Phase A/B

**BLK-1 — The new `release-topology` CI job is red against this commit's own ledgers.**
`ci.yml` invokes `./scripts/validate-distribution-topology.sh "$plan" active`, and `active` mode requires `.activation == "ready"`, `tap_secret_state == "active-proven-r10"`, and `taskfleet-release.json` `.activation == "ready"`. This same diff sets `tap_secret_state: "pending-r10-proof"` and leaves `activation: "prepared-blocked-r10"` / `"blocked-r8-r9-r10"`. Consequences, pick one:

- If this merges as-is, every main push and PR fails `release-topology`, `wait_for_exact_main_ci` can never succeed, and Phase B deadlocks — the exact B1 contradiction, now actually wired onto the enforcement path.
- If the flip is supposed to be simultaneous, the commit is internally inconsistent (it flips CI to `active` while flipping the ledger to `pending`).

The `prepared|active` mode split was built and then **nothing invokes `prepared`**. Fix: derive the mode from the ledger itself and assert pair coherence, so the flip commit touches only the two JSON files:

```bash
state=prepared
[[ "$(jq -r .activation release/taskfleet-distribution.json)" == ready ]] && state=active
# then assert taskfleet-release.json activation matches the same state, else exit 2
```

This makes CI green in both epochs and makes the atomic flip actually atomic.

**BLK-2 — Orphaned authorization ref: still no retraction protocol, and it still bricks or arms the version.**
The improved resume-failure diagnostics ("only run $run_id may reconcile") are messaging, not mechanism. If resume fails post-`record_release_authorization` and the run is later abandoned:

- The ref persists as a standing, consumable authorization for `(v0.6.0, bump_commit)`. The tag ruleset narrows who can exploit it to bypass actors — but the bypass actor is the operator, who is exactly the person who will be manually reconciling a failed release and is the most plausible source of an accidental tag push at that coordinate.
- A retry plan producing a *different* bump commit at the same version hard-fails in `record_release_authorization` ("points at X, expected Y"). The recovery policy is ambiguous here: `item.md` says both "abandon the run and fix forward; remove only the unpushed local tag" and "never … version reuse." If pre-tag abandonment permits reusing 0.6.0 (the natural reading, since nothing published), the version is bricked without ruleset-bypass ref surgery that no protocol authorizes.

Required before flip: an explicit wrapper `abandon <run-id>` operation that (a) verifies the remote tag is absent, (b) deletes the authorization ref via bypass, (c) appends the run to `never_resume_runs` — in that order — plus a one-line policy statement on same-version retry after pre-tag abandonment. The `ci.yml` `cancel-in-progress: true` concurrency group makes this path likely, not theoretical: any push to main during `gh run watch` cancels the bump commit's run.

**BLK-3 — Verifier runtime environment on the release-critical runners is still unverified in the tree.**
Neither `verify-release-tag-authorization.sh` nor `test-release-authorization.sh` appears in this diff. The summary *claims* executable unsafe-topology/ref fixtures now exist, but I can only assess what's shown; if those scripts are unchanged from the last reviewed versions, two prior findings stand:

- The fixture's `env -i PATH="$tmp/bin:/usr/bin:/bin"` requires `jq`/`git`/`awk` at those exact paths. On the self-hosted macOS runner (Homebrew-only `jq` at `/opt/homebrew/bin`), the *positive* fixture case fails → activation fails → the aarch64-apple-darwin leg fails on every legitimate tag. Fail-closed bricking of every release.
- The tag verifier chain needs `gh` + `jq` on the mac runner; nothing provisions or preflights them. The new plan assertion `all(.container == null)` closes the container case — good — but the mac runner provisioning is still an unproven activation precondition. Add a tool-preflight to the verifier (`command -v gh jq git`) with a distinct error, and prove it via a disposable run on the actual runner before flip.

**BLK-4 — Ruleset *content* is only verified wrapper-side, and `verify-release-github-policy.sh` is unreviewed.**
The ledger pins numeric IDs `22234415`/`22234417`; validators check only that the ledger *says* those numbers — self-referential. Everything depends on the unseen policy verifier asserting: enforcement `active`, target coverage (`refs/tags/**` creation+update+deletion; `taskfleet-release-authorizations/**` update+deletion), and a bypass list containing only the release actor. It's also called only on the untagged wrapper path — not on the `pushed_remote == true` resume path and not by CI tag jobs. The CI-side omission is defensible (ruleset deletion is admin-only = declared boundary), but the resume-after-push path gap means post-tag reconciliation runs without the policy attestation that the pre-tag path recorded. Cheap to add. This item blocks only in the sense that the flip checklist must include an adversarial read of that script; the design shape is fine.

## 2. Claims that are wrong or exaggerated under the admin trust boundary

- **"Ref is not cryptographic proof of the wrapper" — correct, but the remedy some reviews imply (a GitHub App) is out of scope.** Under the declared boundary, the ref is a bearer authorization for one `(tag, commit)` pair created by a bypass-privileged actor. That's the design's honest ceiling; demanding more is scope creep. What's *not* exaggerated is the corollary: bearer authorizations need a revocation path (BLK-2).
- **"cargo-dist gives every job `contents: write` and permits host after skipped local builds" — true but now materially defanged.** With the all-tags creation ruleset live, the historical-commit / empty-matrix routes require a bypass actor. My own earlier framing of the empty-matrix host path as a primary blocker is downgraded: it's now defense-in-depth commentary, contingent solely on the ruleset being verified live (BLK-4).
- **Any residual demand that the CI-side verifier recheck live main is wrong.** The durable-ref semantics are correct; a live-main comparison at matrix-job time would burn valid versions on benign merges. The wrapper's pre-receipt exact-main + green-CI attestation is the right place for that check, and this diff even adds the post-CI refetch/recheck before receipt creation.
- **"Rulesets must cover every tag and the authorization namespace" — correct and apparently done**, but the convergent phrasing treats ruleset *existence* as sufficient. Existence (ID in a JSON ledger) proves nothing; content and enforcement state do (BLK-4).
- **"host --steps=create runs before the build verifier" — true and acceptable.** The plan job's `host --steps=create` performs no public mutation (the checked-in `host-create-no-mutation.json` receipt attests exactly this) and holds no tap secret. Treating its ordering as a finding overstates it.

## 3. New concrete release-safety bugs introduced by this diff

1. **BLK-1 above** — the `active`-mode CI wiring against pending ledgers is the one outright new bug.
2. **`record_release_authorization` GET conflation now has a real failure scenario**: after a resume interrupted *post*-ref-creation, re-running resume re-enters this function; a transient GET failure (`2>/dev/null || true` masks 5xx/network as "absent") falls through to POST, gets 422, and exits 2 with "could not create" — even though the ref is valid and correct. Retryable and fail-closed, but the error message actively misleads the operator during the most delicate recovery window. Distinguish 404 from other GET failures.
3. **Duplicated fixture execution in `release-topology`**: `test-release-authorization.sh` runs directly *and* inside `validate-distribution-topology.sh`. Harmless waste, but it signals the ownership problem — the fixture invocation still lives in three places (`ci.yml`, distribution validator, activation verifier).
4. **`branch` assignment in `resume_after_gate` is not `local`** — clobbers/creates a global. Cosmetic today; a latent trap given the script's heavy global state.
5. Positive note, not a bug: the checksum-pinned `dist` fetch, `generate --check` in CI, container-nullity plan assertion, and main-push self-hosted coverage all correctly close previously open items (M5, N4-container-leg, and the Phase B self-hosted-evidence gap respectively).

## 4. Top three priorities

1. **Make CI state-coherent and the flip ledger-only atomic (BLK-1).** Derive `prepared|active` from the ledgers with a pair-coherence assertion. Until this lands, either CI is red or the flip cannot be atomic — everything downstream (Phase B green exact-main SHA) is blocked on it.
2. **Ship the abandonment protocol for the authorization ref (BLK-2).** A wrapper `abandon` verb: assert remote tag absent → delete ref (bypass) → record run in `never_resume_runs`; plus an explicit written rule on same-version retry after pre-tag abandonment. Without it, the single most likely operational failure (canceled main CI run → abandoned release) leaves either a live bearer authorization or a bricked version.
3. **Prove the release-time verifier environment and the policy verifier before flipping.** (a) Tool preflight in `verify-release-tag-authorization.sh` and PATH-portable fixtures, exercised in a disposable run on the actual self-hosted mac; (b) adversarial review of `verify-release-github-policy.sh` asserting enforcement/targets/bypass content — not just IDs — and add its invocation to the `pushed_remote == true` resume path. If the claimed fixture/verifier script changes from the summary are not actually in the tree, land them; nothing in this diff evidences them.

---

## Model: deepseek-v4-pro
[model:deepseek-v4-pro] [thread_id:api_12904303436345528272cba2946bff18]

## Blockers remaining after the current changes

### 1. CI cannot be green in the blocked state because `release-topology` validates `active`

`.github/workflows/ci.yml`:

```yaml
- run: ./scripts/validate-distribution-topology.sh "$RUNNER_TEMP/taskfleet-dist-plan.json" active
```

Current ledgers are intentionally blocked:

- `release/taskfleet-release.json`: `"activation": "blocked-r8-r9-r10"`
- `release/taskfleet-distribution.json`: `"activation": "prepared-blocked-r10"`, `"tap_secret_state": "pending-r10-proof"`

But `active` mode in `scripts/validate-distribution-topology.sh` requires:

```jq
.activation == "ready"
tap_secret_state == "active-proven-r10"
release/taskfleet-release.json .activation == "ready"
```

Therefore every PR and every push to `main` fails this job. That means `wait_for_exact_main_ci` in `scripts/shipshape-release.sh` can never observe green exact-SHA push CI, and Phase B/Phase C cannot proceed.

This is not a security boundary; it is a broken CI gate.

Fix: the checked-in CI invocation must use `prepared` while the ledgers are blocked, and only the final atomic activation commit should flip CI, ledgers, and validator mode to `active` together.

---

### 2. Release workflow still is not structurally fail-closed

This remains true in `.github/workflows/release.yml`.

The `plan` job runs before any authorization:

```yaml
run: |
  dist ${{ (!github.event.pull_request && format('host --steps=create --tag={0}', github.ref_name)) || 'plan' }} --output-format=json > plan-dist-manifest.json
```

with:

```yaml
permissions:
  "contents": "write"
env:
  GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
```

In cargo-dist 0.28.2, `host --steps=create` is the release-creation step. It is not a local-only plan. It runs before the only verifier invocation in the generated workflow. Any tag that reaches this workflow can therefore mutate the GitHub Release object before authorization.

The verifier itself is embedded only in `build-local-artifacts`:

```yaml
if: ${{ fromJson(needs.plan.outputs.val).ci.github.artifacts_matrix.include != null && ... }}
```

If `artifacts_matrix.include` is `null`, that job is skipped and the verifier never runs. `host` then explicitly allows the skip:

```yaml
if: ${{ always() && needs.plan.outputs.publishing == 'true' && ... &&
       (needs.build-local-artifacts.result == 'skipped' || needs.build-local-artifacts.result == 'success') }}
```

The source design relies on the checked-in topology validator pinning a non-empty local matrix, but that validator does not run in `release.yml` before `host`. Therefore the assumption is not enforced at tag time.

`scripts/test-release-authorization.sh` currently codifies the unsafe condition as a security requirement:

```bash
grep -A12 '^  host:' "$release" | grep -F 'needs.build-local-artifacts.result == '\''skipped'\''' >/dev/null
```

That fixture should fail on a permissive skip, not require it.

The fix must either:

- introduce an always-run authorization job that `plan` depends on, so no create step and no publish path runs before verification; or
- make `host` require `build-local-artifacts.result == 'success'` and ensure the verifier always executes.

---

### 3. Live ruleset verification is still not wired into tag-time authorization

`scripts/shipshape-release.sh` now calls:

```bash
"$repo_root/scripts/verify-release-github-policy.sh" >/dev/null
```

before `record_release_authorization`.

But the attached `scripts/verify-release-tag-authorization.sh` does not call that script. It calls only:

```bash
./scripts/verify-release-activation.sh >/dev/null || exit 1
```

and the attached `scripts/verify-release-activation.sh` only checks JSON ruleset IDs:

```jq
.cargo_dist.release_tag_ruleset == 22234415 and
.cargo_dist.authorization_ref_ruleset == 22234417
```

No live GitHub policy check appears in that path.

Consequence: tag workflows do not verify that the rulesets still exist or are active at release time. They trust checked-in JSON IDs. If a ruleset is deleted, disabled, or weakened after activation, tag-driven `release.yml` and `publish-crates.yml` can still pass authorization as long as `release/taskfleet-distribution.json` is unchanged.

The fixture test would also have failed if the verifier called `verify-release-github-policy.sh`, because the fixture only stubs `verify-release-activation.sh` and `gh`. That is further evidence the call is absent from the tag path.

`verify-release-activation.sh` or `verify-release-tag-authorization.sh` must invoke the live policy verifier, and the fixture must account for it.

---

### 4. `github.sha` is still used as a commit SHA when annotated tags are supported

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

`scripts/verify-release-tag-authorization.sh` deliberately peels checkout to a commit:

```bash
release_commit="$(git rev-parse 'HEAD^{commit}')"
```

and `scripts/shipshape-release.sh` accepts an annotated local tag as long as its peeled commit equals `bump_commit`.

If Shipshape ever creates an annotated tag, `github.sha` on a tag push may identify the tag object, not the commit. `gh release create --target` and crates source-commit verification would then target/record the wrong SHA.

Compute the peeled commit in the workflows:

```bash
RELEASE_COMMIT="$(git rev-parse "refs/tags/$GITHUB_REF_NAME^{commit}")"
SOURCE_COMMIT="$(git rev-parse "refs/tags/$GITHUB_REF_NAME^{commit}")"
```

---

## Which prior claims are now wrong or exaggerated

### Wrong under the current implementation

- **“Authorization-ref creation has a fast-forward race.”**  
  `record_release_authorization` no longer uses `git branch push`. It uses the atomic GitHub create-ref API and then verifies the returned ref and object SHA exactly. The create-ref API cannot fast-forward an existing ref.

- **“The broad Homebrew token is reachable from Actions.”**  
  The installed `HOMEBREW_TAP_TOKEN` is inert random data, not the broad classic OAuth token. The activation-blocking token concern is still valid in the sense that activation remains blocked, but there is no current broad-token exposure.

- **“The tag verifier compares live `main` after tag push.”**  
  It does not. That prior TOCTOU finding is resolved.

### Still true and correctly scoped

- **The authorization ref is not cryptographic proof.**  
  The current wording already limits the claim. A repository admin can still imitate the wrapper. Under that explicit trust boundary, the ref is a durable coordination receipt, not proof of origin.

- **Rulesets must cover both namespaces.**  
  `refs/tags/*` and `refs/heads/taskfleet-release-authorizations/*` both need server-side creation/update/deletion restrictions. JSON ruleset IDs alone do not enforce this.

---

## Top three priorities

1. **Make CI validation state coherent.**  
   Change the main-branch `release-topology` invocation to `prepared` while the ledgers are blocked, and make the final `prepared` → `active` transition atomic across CI, ledgers, and validators.

2. **Make tag-time release enforcement independent of cargo-dist skip semantics.**  
   Add an always-run authorization job before `plan`, prevent `host --steps=create` before authorization, and fail closed when local artifacts are skipped before authorization. Fix `scripts/test-release-authorization.sh` so it rejects the current permissive host-skip condition.

3. **Wire live ruleset verification into tag-time authorization and activation.**  
   `verify-release-tag-authorization.sh` or `verify-release-activation.sh` must call `verify-release-github-policy.sh`, and the fixture must stub/assert that live check as part of the fail-closed boundary.
