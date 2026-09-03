[thread_id:group_19ea46c3df7746f387c2d9ee554d111f]

## Model: gemini-3.1-pro-preview
[model:gemini-3.1-pro-preview] [thread_id:api_e42b5d75563d4947811111f749218108]

Based on the available evidence, here is the critical analysis of the R9 repository identity convergence contracts.

### 1. Missing Token Provisioning for CI Proof
**File:** `release/taskfleet-distribution.json`
**Issue:** `"tap_secret_state"` remains set to `"inert-blocked-r10"`.
**Why it matters:** The R9 execution plan (`issues/taskfleet-distribution-topology/plan.md`) explicitly requires R9 to *"install least-privilege token; prove through renamed-repo workflow before activation."* If the token remains inert, the CI runner cannot legitimately complete the tap checkout/push proof workflow. If the token *was* provisioned as a least-privilege PAT for this PR, this JSON contract is stale and incorrect.
**Blocks CI:** Yes. You cannot prove a renamed-repo tap workflow with an inert token.

### 2. Broken State Tracking in Distribution Contract
**File:** `release/taskfleet-distribution.json`
**Issue:** The `source_repository` block lists `"current": "jarimustonen/taskfleet"` and `"after_r9": "jarimustonen/taskfleet"`.
**Why it matters:** The JSON file acts as a state-tracking ledger for the migration. `"current"` must accurately reflect the pre-R9 state (`jarimustonen/orchestratectl`). Setting both to the canonical name destroys the before/after transition evidence required by the R9 acceptance criteria.

### 3. Cargo-Dist Trigger Ambiguity
**File:** `release/taskfleet-distribution.json`
**Issue:** `"trigger": "tag-push"` is configured, complying with the requirement to restore tag dispatch. However, ADR 0002 explicitly mandates: *"Regenerate its workflow; do not hand-edit generated release identity."*
**Why it matters:** It is a critical release-safety regression if the `.github/workflows/release.yml` was manually patched to change the trigger rather than running `cargo dist generate` with `dispatch-releases = false`. I cannot verify this without the diff.

### 4. Over-replacement Risks (Blind Replace)
**Context:** The R9 plan demands identity convergence while strictly preserving intentional legacy compatibility, `OCTL_*` protocols, and the bounded `orchestratectl` wrapper identities.
**Why it matters:** A global find-and-replace of `orchestratectl` to `taskfleet` will corrupt the bounded CLI wrapper (`compat/orchestratectl/Cargo.toml`), the `orchestratectl-installer.sh` stub artifact defined in `taskfleet-distribution.json`, and the old-tap migration metadata (`old-tap-migration/`).

## Context request
- Kind: `artifact`
- Need: The full uncommitted `git diff` (or `git show` output) containing the R9 changes.
- Why: The instructions require reviewing the full uncommitted diff to identify incorrect residual replacements, ensure `cargo-dist` was natively regenerated rather than hand-edited, verify the preservation of intentional legacy references (like `OCTL_` protocols and the old tap metadata), and review the CI/runner proof gaps. It is impossible to declare R9 safe or evaluate the identity convergence without the actual code changes.

---

## Model: gpt-5.6-sol
[model:gpt-5.6-sol] [thread_id:api_69962a87549d49db8c4c7ac30d8e4284]

# Review findings

The supplied artifacts do not include the uncommitted diff, generated workflows, release-gate implementation, CI assertions, or run receipts. Therefore this is a bounded review of the attached contracts, not the requested full-diff review.

## P0 — Release safety cannot be established from the supplied candidate

### 1. Restoring the tag trigger is not enough; the generated workflow’s dependency graph is unverified

**Paths:**

- `.github/workflows/release.yml` — not supplied
- `.github/workflows/taskfleet-release-gate.yml` — not supplied
- `scripts/verify-release-activation.sh` — not supplied
- `dist-workspace.toml` — not supplied
- `release/taskfleet-distribution.json` → `/cargo_dist/trigger`
- `release/taskfleet-release.json` → `/activation`

`release/taskfleet-distribution.json` claims:

```json
"trigger": "tag-push",
"activation_gate": ".github/workflows/taskfleet-release-gate.yml"
```

while `release/taskfleet-release.json` remains blocked. That is safe only if the generated workflow guarantees all of the following:

1. Every non-dry publishing path depends on the activation-gate job.
2. Artifact builds, GitHub Release creation/upload, provenance publication, and Homebrew secret access cannot run after a failed or skipped gate.
3. cargo-dist’s host job cannot exploit skipped dependencies and continue.
4. The cancellation workaround still covers the complete workflow under the renamed repository.
5. The gate validates the exact canonical repository and immutable release state, rather than merely checking that activation is not empty.
6. A tag event cannot bypass workflow-dispatch-specific conditions.

None of this can be inferred from the JSON declarations. A declaration is not an enforcement boundary.

**Required fix/verification:**

- Inspect the generated `release.yml` job graph.
- Run the machine assertion that proves every publishing job is transitively gated.
- Exercise a synthetic rejected tag-trigger invocation, without creating a public release tag, using the repository’s established safe test mechanism.
- Confirm no job reads `HOMEBREW_TAP_TOKEN`, creates a release, or uploads artifacts when activation is blocked.

**Candidate CI:** **Blocks candidate CI review.** The candidate must not merge until the generated workflow and its safety assertions run successfully.

---

### 2. The two release manifests describe inconsistent migration phases

**Paths:**

- `release/taskfleet-release.json:3`
- `release/taskfleet-distribution.json:3`
- `release/taskfleet-distribution.json:7`
- `release/taskfleet-distribution.json:11`
- `issues/taskfleet-distribution-topology/plan.md`, “Exact R9 substitutions”

Current values:

```json
// taskfleet-release.json
"activation": "blocked-r8-r9-r10"
```

```json
// taskfleet-distribution.json
"activation": "prepared-blocked-r10",
"trigger": "tag-push",
"tap_secret_state": "inert-blocked-r10"
```

The distribution manifest says R9 is complete enough to be blocked only on R10, while the authoritative release manifest still says R8 and R9 are unresolved. Keeping publication blocked is correct, but disagreement about the current phase is dangerous if different scripts consume different manifests.

This can cause:

- the generated workflow to treat distribution as prepared while the release wrapper rejects it;
- assertions to encode contradictory expected states;
- an operator to “fix” the mismatch by changing the release manifest to `ready`, accidentally authorizing publication;
- R10 automation to assume R9 receipts exist when they do not.

**Required fix:**

Define one explicit post-R9/pre-R10 blocked state across both files, for example:

```json
"activation": "blocked-r10"
```

Only do this if all consumers recognize that value and it remains fail-closed. Otherwise retain the existing authoritative blocked value in both manifests until R9 closure. Under no circumstances should either manifest become `ready` in this candidate.

Add an assertion that both manifests agree on whether release is blocked:

```sh
test "$(jq -r .activation release/taskfleet-release.json)" != "ready"
test "$(jq -r .activation release/taskfleet-distribution.json)" != "ready"
```

Prefer a typed state model over string conventions spread across scripts.

**Candidate CI:** **Potentially blocks candidate CI**, depending on existing assertions. It unconditionally blocks R9 acceptance because release-state ownership is ambiguous.

---

### 3. The R9 plan still instructs automation to activate release state

**Path:** `issues/taskfleet-distribution-topology/plan.md`, “Exact R9 substitutions”

The table says:

> set distribution trigger/activation to `tag-push`/`ready` only after gates

and:

> atomically update both checks to require canonical tag-push, live least-privilege proof and ready state

That conflicts with the current task contract and review constraint: R9 restores the tag trigger but does **not** authorize publication, and `release/taskfleet-release.json` must remain blocked.

This is not harmless migration prose. It is an active execution plan that can direct R9 operators or assertions to set `ready`.

**Required fix:**

Change the post-R9 contract to distinguish trigger posture from release authorization:

| Field | Post-R9/pre-R10 value |
|---|---|
| cargo-dist trigger | `tag-push` |
| release activation | blocked on R10 |
| distribution activation | prepared but blocked on R10 |
| tap credential | inert after bounded proof |
| activation gate | mandatory and fail-closed |

Replace “ready state” with “canonical tag-push configuration with blocked activation.” Preserve the old values only in an explicitly labeled historical receipt section.

**Candidate CI:** **May not fail ordinary CI**, but this is a release-safety documentation blocker and should block merge.

---

## P1 — Repository identity convergence is incomplete or unsupported

### 4. The operational distribution plan still states that the source repository is `jarimustonen/orchestratectl`

**Path:** `issues/taskfleet-distribution-topology/plan.md`, “Current sealed topology” and “Exact R9 substitutions”

The plan still describes the old source repository as current:

> R7 deliberately keeps source-hosting coordinates truthful while the public source repository is still `jarimustonen/orchestratectl`.

The pre-R9/post-R9 substitution table is useful history, but after the one-way rename it must not remain phrased as current operational truth. This directly conflicts with:

```json
"repository": "jarimustonen/taskfleet"
```

and:

```json
"source_repository": {
  "current": "jarimustonen/taskfleet",
  "after_r9": "jarimustonen/taskfleet"
}
```

The old identity may remain in historical and migration prose, but the section must be relabeled as an R7 historical snapshot. Otherwise maintained-source scans cannot distinguish an intentional historical residual from a stale operational coordinate.

**Required fix:**

- Rename “Current sealed topology” to something like “R7 sealed topology before R9.”
- Mark every old source URL in the substitutions table as historical evidence.
- Add a post-R9 current-state section containing only the canonical repository.
- Ensure machine-readable scanners ignore only narrowly designated historical blocks, not the entire file.

**Candidate CI:** **Blocks candidate CI if the residual-identity/docs assertions are correctly comprehensive.** Otherwise it remains an acceptance blocker that CI currently fails to detect.

---

### 5. `source_repository.after_r9` is now meaningless and preserves a transitional schema indefinitely

**Path:** `release/taskfleet-distribution.json:18-21`

```json
"source_repository": {
  "current": "jarimustonen/taskfleet",
  "after_r9": "jarimustonen/taskfleet"
}
```

After R9, `after_r9` is redundant. More importantly, leaving transition-phase fields in the active release topology encourages scripts to select the wrong coordinate depending on phase assumptions.

**Required fix:**

Collapse this to a canonical source identity:

```json
"source_repository": {
  "canonical": "jarimustonen/taskfleet"
}
```

If consumers require the old schema, version the manifest and migrate the consumers in the same candidate. Historical pre-R9 identity belongs in receipts or issue evidence, not active release configuration.

**Candidate CI:** **Potentially blocks candidate CI** if schema consumers expect the current structure. Otherwise non-blocking for execution but required before declaring identity convergence complete.

---

### 6. No evidence shows that exact source-owned URLs were converged without redirects

**Path:** `issues/taskfleet-source-repository-rename/item.md`, “Required execution” and “Acceptance criteria”

The task requires updates to:

- remotes;
- Cargo repository/homepage metadata;
- GitHub Actions references;
- badges;
- repository metadata;
- release-wrapper checks;
- cargo-dist/source links;
- installer/archive URLs;
- operator documentation.

Only the two JSON files provide post-rename values here. There is no search inventory or classification of remaining `jarimustonen/orchestratectl` references.

A raw search will include intentional residuals, so evidence must classify them rather than demand zero matches. Acceptable classes include:

- ADR/history;
- old GitHub redirect migration documentation;
- old Homebrew tap identity;
- old Cargo wrapper/package identity;
- fixtures;
- compatibility installer filename;
- intentional migration diagnostics.

Everything else must converge.

**Required evidence:**

At minimum, sanitized output equivalent to:

```sh
git grep -nE \
  'jarimustonen/orchestratectl|github\.com/jarimustonen/orchestratectl|api\.github\.com/repos/jarimustonen/orchestratectl'
```

with every match classified. Also inspect encoded URLs and workflow repository fields that a plain grep may miss.

**Candidate CI:** **Blocks R9 acceptance.** Whether it blocks the actual CI run is unknown because the identity assertion implementation was not supplied.

---

## P1 — CI and runner proof is missing

### 7. No renamed-repository CI run proves the self-hosted macOS ARM64 leg

**Paths:**

- `issues/taskfleet-source-repository-rename/item.md`, “Required execution”
- `.github/workflows/ci.yml` — not supplied
- CI run receipts — not supplied

The precondition references old evidence:

- tested SHA `c3ef8b740ac531f12ce81c759ed209d178cf36bd`;
- CI run `33764612111`.

That does not satisfy the post-rename requirement. Repository renames can alter:

- self-hosted runner association and visibility;
- environment/repository allowlists;
- secret access;
- workflow identity checks;
- cache keys;
- status-check names and branch protection;
- API permissions.

The candidate must show a new run in `jarimustonen/taskfleet`, on the candidate SHA, whose macOS job actually executed on the intended ARM64 self-hosted runner. A green workflow where the job was skipped, substituted, or remained queued is insufficient.

**Required evidence:**

Record:

- canonical run URL and run ID;
- candidate commit SHA;
- event type;
- every job ID and conclusion;
- runner name/ID, labels, OS, and architecture for the macOS job;
- confirmation that Linux jobs also ran rather than being skipped;
- final exact-main push-run URL after merge before R9 closure.

Do not expose runner registration tokens or secret values.

**Candidate CI:** **Yes, directly blocks candidate CI.**

---

### 8. The runner selector is too broad to prove execution on the intended machine

**Paths:**

- `release/taskfleet-distribution.json:13`
- `issues/taskfleet-distribution-topology/plan.md`, “Current sealed topology”

The selector remains:

```json
"macos_runner": "macOS"
```

The plan acknowledges that this selects by the generic `macOS` label rather than a unique Taskfleet label. If more than one self-hosted macOS runner is visible, the job can run on the wrong architecture or machine.

The task specifically requires a self-hosted macOS ARM64 leg. A generic `macOS` label does not encode either `self-hosted` or `ARM64` in the manifest.

**Required fix:**

Use an explicit label set in the workflow, at least:

```yaml
runs-on: [self-hosted, macOS, ARM64]
```

Prefer a dedicated label such as:

```yaml
runs-on: [self-hosted, taskfleet-macos-arm64]
```

If cargo-dist only accepts a single runner string, document and assert the generated YAML’s complete selector rather than treating `"macOS"` as sufficient proof.

**Candidate CI:** **Blocks candidate CI** unless repository runner inventory proves `macOS` uniquely selects the intended ARM64 machine and the run receipt confirms the actual runner.

---

## P1 — Evidence and receipt requirements are not met

### 9. The R9 issue contains no immutable before/after evidence

**Path:** `issues/taskfleet-source-repository-rename/item.md`

The task explicitly requires evidence under the issue, but the supplied issue only contains planned work. It does not record:

- GitHub repository ID before and after;
- old and canonical full names;
- rename timestamp;
- default branch and visibility;
- rules/settings snapshots;
- Actions status;
- secret names;
- runner visibility;
- remote URLs;
- API receipts;
- canonical clone/fetch/push proof;
- redirect behavior;
- residual identity classification;
- renamed-repository CI runs.

The stable repository ID is especially important because it proves that `jarimustonen/taskfleet` is the renamed repository rather than a different repository created under the target name.

**Required fix:**

Add a sanitized evidence section or checked-in receipt file referencing immutable artifacts. Include command names and hashes where output cannot safely be committed.

**Candidate CI:** **Does not necessarily prevent jobs from running, but blocks R9 acceptance and merge under the stated task contract.**

---

### 10. No proof is supplied that the canonical repository name was unoccupied immediately before mutation or that the old name was not recreated

**Path:** `issues/taskfleet-source-repository-rename/item.md`, “Preconditions” and “Acceptance criteria”

The issue states these checks as requirements but provides no receipts. Post-rename verification must establish:

- canonical repository ID equals the old repository’s pre-rename ID;
- `jarimustonen/orchestratectl` is only a GitHub-managed redirect;
- no repository now occupies the old name;
- authenticated canonical API calls return the canonical full name without relying on redirect handling.

A successful browser URL or `git fetch` against the old URL is not proof; GitHub redirects make those tests pass.

**Candidate CI:** **No direct CI execution block**, but an R9 acceptance blocker.

---

## P2 — Contract quality and maintainability

### 11. The release topology duplicates authoritative repository identity across multiple files

**Paths:**

- `release/taskfleet-release.json`
- `release/taskfleet-distribution.json`
- `dist-workspace.toml`
- Cargo workspace metadata
- generated `.github/workflows/release.yml`
- release-wrapper checks

The repository identity is duplicated across at least two JSON manifests and, according to the plan, Cargo metadata, cargo-dist config, generated workflow URLs, and wrapper checks. R9 is exactly the kind of transaction where duplicated identity drifts.

**Required architectural change:**

Choose one machine-readable canonical topology file and derive/assert all other coordinates from it. Generated files should carry a provenance marker and CI should compare regeneration output byte-for-byte:

```sh
cargo dist generate
git diff --exit-code -- .github/workflows/release.yml
```

Also assert that no maintained generated URL points to the old source repository.

**Candidate CI:** **Not necessarily a current blocker**, but a missing regeneration diff or failed generated-file assertion must block.

---

### 12. The dated issue metadata is inconsistent with the supplied review date

**Path:** `issues/taskfleet-source-repository-rename/item.md:2-3`

The issue records:

```yaml
created: 2026-09-04
updated: 2026-09-04
```

The supplied review date is 2026-09-03. Unless repository tooling intentionally uses a future release-day date, this weakens the chronology of the claimed immutable evidence and mutation record.

**Required fix:**

Correct the dates or document the clock/time-zone/source responsible. Do not use future-dated issue metadata as evidence that a mutation already occurred.

**Candidate CI:** Usually **no**, unless issue validation rejects future dates. It is an evidence-quality defect.

## Candidate CI disposition

Based on the supplied material, the candidate is **not reviewable as ready to merge**.

The following must block the candidate run or its acceptance:

1. Full Linux CI and the actual self-hosted macOS ARM64 job must run on the candidate SHA in `jarimustonen/taskfleet`.
2. Generated cargo-dist workflow safety must be inspected and asserted with the restored tag trigger while activation remains blocked.
3. Active R9 documentation must stop instructing operators or assertions to set release state to `ready`.
4. Remaining old source-repository references must be inventoried and classified.
5. Immutable rename/API/runner receipts must be recorded.
6. Generated files must be regenerated and checked for drift.

## Context request

- **Kind:** artifact
  **Need:** Full uncommitted diff, preferably `git diff --stat`, `git diff --check`, and `git diff --find-renames -- . ':(exclude)history/**'`.
  **Why:** Required to identify incorrect replacements, missed source URLs, changes to intentional compatibility identities, and exact line-level regressions.

- **Kind:** artifact
  **Need:** `.github/workflows/release.yml`, `.github/workflows/taskfleet-release-gate.yml`, `.github/workflows/ci.yml`, `scripts/verify-release-activation.sh`, `dist-workspace.toml`, and all release-topology assertion tests.
  **Why:** Determines whether restored tag dispatch can bypass the blocked activation gate and whether generated cargo-dist output is consistent.

- **Kind:** artifact
  **Need:** Candidate CI run URL/JSON showing commit SHA, repository full name, every job conclusion, and sanitized runner labels/architecture for the macOS job.
  **Why:** Determines whether the mandatory renamed-repository Linux and self-hosted macOS ARM64 proof actually passed.

- **Kind:** artifact
  **Need:** Sanitized pre/post GitHub API receipts containing repository ID/full name/default branch/visibility, old-name behavior, Actions status, secret names, runner visibility, and canonical API/clone/fetch/push results.
  **Why:** Determines whether the one-way rename preserved repository identity and whether canonical operations avoid redirect dependence.

- **Kind:** artifact
  **Need:** Classified residual search output for old source coordinates and generated-file regeneration output from cargo-dist 0.28.2.
  **Why:** Determines whether old references are intentional compatibility/history or blocking maintained-source identity gaps.

---

## Model: claude-fable-5
[model:claude-fable-5] [thread_id:api_46e1eba0bebb4cb4bc52ee828981fe68]

# R9 identity-convergence review

The attached evidence is the two release JSONs, the R7 plan, the R9 issue contract, and ADR 0002. The actual uncommitted diff (workflows, `dist-workspace.toml`, gate/assertion scripts, Cargo metadata, receipts) is not attached, so several conclusions below are bounded and flagged. Findings in priority order; each marked for whether it blocks candidate CI.

## F1 — Contract conflict: R7 plan instructs R9 to set `ready` + live token; item.md and review constraints forbid it (BLOCKING until resolved)

`issues/taskfleet-distribution-topology/plan.md`, substitution table rows "cargo-dist trigger" and "R7 posture assertions" tell R9 to set "distribution trigger/activation to `tag-push`/`ready`" and update checks to require "live least-privilege proof and ready state". That directly contradicts:

- `issues/taskfleet-source-repository-rename/item.md` acceptance: "No crate publication, release tag… occurs. R10/release remain blocked; R9 does not authorize publication."
- The review constraint that `release/taskfleet-release.json` stays blocked while the tag trigger is restored behind the gate.

The attached JSONs show the safe posture was kept (`activation` blocked, `tap_secret_state: inert-blocked-r10`), so the diff apparently did **not** implement the table literally. But that creates a fork:

- If the Rust/shell posture assertions were updated per the table (require tag-push + live token + ready), they will fail against the attached JSONs → candidate CI red.
- If assertions still require the old R7 posture (dispatch-only + inert + blocked), they fail against `trigger: "tag-push"` in `release/taskfleet-distribution.json` → candidate CI red.
- Only a deliberately updated hybrid contract (tag-push + gate wired + blocked + inert) passes, and nothing attached proves that update happened.

Fix the plan table (split the `ready`/live-token rows out to R10) and show the assertion-script diff. Cannot verify without `scripts/verify-release-activation.sh` and the Rust posture checks.

## F2 — `release/taskfleet-release.json` activation label is stale and the "atomic" transaction is incomplete (likely CI-relevant)

`activation: "blocked-r8-r9-r10"` still names R8 as a blocker, but R8 is complete (evidence commit `488d6cab…` authorizes R9) and R9 is executing. Meanwhile `release/taskfleet-distribution.json` was changed (`trigger: "tag-push"`, `source_repository.current` updated) — so the identity/trigger transaction touched one JSON's posture fields but not the other's. The R7 plan requires the trigger flip and posture assertions to update **atomically**. Either:

- change to `activation: "blocked-r10"` in the same commit and update the gate's accepted-value list, or
- document why the composite label is intentionally retained through R9.

Fail-closed either way, so not a release-safety regression, but if the gate does exact string matching this interacts with F1 and can flip candidate CI.

## F3 — No proof the regenerated cargo-dist workflow preserved the safety topology (BLOCKING)

Restoring tag-push requires `dist-workspace.toml` edit (`dispatch-releases = false`) plus cargo-dist 0.28.2 `generate`, which **rewrites** `.github/workflows/release.yml`. R7's protections were injected into the generated workflow; regeneration can silently drop them. Must be verified in the diff:

1. The plan-job dependency on `taskfleet-release-gate.yml` survived regeneration and every build/host/publish job is downstream of it.
2. The `actions: write` cancellation fallback survived — critical because cargo-dist's host job accepts skipped build dependencies, so a gate-rejected tag run would otherwise still reach hosting.
3. The generated tag trigger pattern matches only intended release tags.
4. The repository still has no tag ruleset (receipted honestly in R7), so gate + blocked JSON is now the **sole** protection against a stray tag push starting a real release. With tag-push restored, this is a materially thinner safety margin than R7's dispatch-only/dry-run posture; the fail-closed behavior of the gate on `blocked-*` activation must be demonstrated (dry evidence run or unit test of `verify-release-activation.sh`), not asserted.
5. The macOS build job (`macOS` label, self-hosted ARM64) is downstream of the gate so unauthorized tags never schedule work onto self-hosted hardware.

None of this is verifiable from the attached files. Blocks candidate CI acceptance.

## F4 — Cargo metadata / generated-URL substitution unverified; failure here poisons generated artifacts (BLOCKING)

cargo-dist derives hosting owner/repo and installer/archive URLs from the workspace `repository` field. The plan table requires `Cargo.toml` `repository`/`homepage` → `https://github.com/jarimustonen/taskfleet`. If regeneration ran **before** the Cargo metadata edit, the regenerated workflow/plan embeds `orchestratectl` URLs — an incorrect residual that works only via GitHub redirect, which the ADR and item.md explicitly forbid relying on for maintained coordinates. Need: `Cargo.toml` diff, `dist plan` output showing hosting owner/repo `jarimustonen/taskfleet`, and generated installer URL inventory.

## F5 — No candidate CI / runner evidence exists (BLOCKING for acceptance by definition)

item.md requires: candidate run in the renamed repository exercising the self-hosted macOS runner and all Linux jobs, every job reviewed, and a fresh green `ci.yml` push run on the final merged main SHA. No run URLs, job lists, or snapshot-change review are attached, and no receipt shows the self-hosted runner accepted jobs under the renamed repository (runner registration is repo/URL-scoped in some configurations; the rename can orphan it). This is the core R9 proof and is currently absent.

## F6 — Required rename receipts and residual scan are missing (blocks R9 closure, not CI)

item.md mandates, none attached:

- Sanitized before-state (repo ID/name/default branch/visibility/rules/Actions/secret names/runner visibility/remotes/URL inventory).
- Immediately-pre-mutation revalidation receipt (old repo identity intact, `jarimustonen/taskfleet` unoccupied, no in-flight release/tag/tap action).
- Post-rename canonical clone/fetch/push/PR/API receipts proving no redirect dependence (e.g., API calls asserting no `301` follow).
- Maintained-surface scan with every residual classified as compatibility/protocol/history/convergence. Note the classifier must **preserve**: `compat/orchestratectl/` wrapper identity, `OCTL_*`, old-tap `jarimustonen/homebrew-orchestratectl` references, `orchestratectl-installer.sh` stub, 0.5.1 fixtures, ADR/history prose. Absent the scan output, over-replacement cannot be ruled out.

## F7 — Token-install ownership is ambiguous (needs clarification; not CI-blocking if F1 resolved)

Plan table post-R9 for the Homebrew credential: "install least-privilege token; prove through renamed-repo workflow before activation." `taskfleet-distribution.json` still says `tap_secret_state: "inert-blocked-r10"`. Either R9 skipped a required step, or token install is deliberately deferred to R10 — in which case the plan table row and the JSON should both say so explicitly, and the gate must not require live-token proof (ties back to F1).

## F8 — Degenerate/ambiguous fields in `release/taskfleet-distribution.json` (minor)

- `source_repository.current == source_repository.after_r9` — the field pair no longer encodes anything. If any assertion previously used the inequality as "rename pending" state, it's now broken; otherwise replace with a rename receipt reference (old repo ID → new full name, rename timestamp).
- `public_receipts.repository_id: 1355125556` is the **tap** repository ID but the key doesn't say so; trivially misread as the source repo. Rename to `homebrew_tap_receipts` or add a `repository` field. Also, the source repository's own ID (which is rename-stable and the strongest post-rename identity evidence) is recorded nowhere in the attached JSONs.

## F9 — Stub verification claim needs honest scoping (minor)

`stub_sha256` unchanged is expected (stub already printed the canonical taskfleet URL). But the plan's post-R9 "verify it resolves after rename" cannot pass yet — no release exists, so `releases/latest/download/orchestratectl-installer.sh` returns 404 regardless. Record this as "unverifiable until first canonical release," not as passed; otherwise the evidence overstates.

## F10 — `publish-crates.yml` posture unreviewed post-rename (verify in diff)

`release/taskfleet-release.json` references `publish-crates.yml`, but nothing attached shows its trigger/identity posture after the rename. It must not gain (or retain) any trigger that could publish on the restored tag-push, and any repository identity check inside it must use the canonical name.

---

## Context request

- Kind: artifact
  Need: Full uncommitted diff for `dist-workspace.toml`, `.github/workflows/release.yml`, `.github/workflows/taskfleet-release-gate.yml`, `.github/workflows/publish-crates.yml`, `.github/workflows/ci.yml`, `scripts/verify-release-activation.sh`, the Rust/shell R7 posture assertions, root `Cargo.toml` (workspace `repository`/`homepage`), and README/badges.
  Why: Determines F1 (assertion/JSON posture mismatch → CI red vs. safety regression), F3 (gate + cancel survival of cargo-dist regeneration), F4 (canonical URLs in generated artifacts), F10.
- Kind: artifact
  Need: `dist plan` output (cargo-dist 0.28.2) at the candidate SHA.
  Why: F3/F4 — proves hosting owner/repo is `jarimustonen/taskfleet` and artifact list contains only intended items.
- Kind: artifact
  Need: Evidence files under `issues/taskfleet-source-repository-rename/` — before-state capture, pre-mutation revalidation, post-rename canonical-operation receipts, residual scan classification, candidate CI run URLs including the self-hosted macOS job.
  Why: F5/F6 — R9 acceptance criteria are unverifiable without them; also rules out over-replacement of intentional legacy identities.
- Kind: clarification
  Need: Is least-privilege `HOMEBREW_TAP_TOKEN` installation owned by R9 (per plan table) or deferred to R10? And what is the intended post-R9 `activation` value in `release/taskfleet-release.json` (`blocked-r8-r9-r10` retained vs. `blocked-r10`)?
  Why: F1/F2/F7 — decides whether the attached JSONs are correct and whether the plan table must be amended, which in turn decides whether the posture assertions pass or fail candidate CI.

---

## Model: deepseek-v4-pro
[model:deepseek-v4-pro] [thread_id:api_a6cd68a62356403898b0c93711503268]

## Scope and assumption

I only have the four attached files. The full uncommitted diff, generated workflows, Cargo/dist manifests, and immutable receipts are not included. Findings that depend on absent files are marked as such.

I assume this review is after the one-way GitHub rename. If these canonical `jarimustonen/taskfleet` fields were committed before the actual GitHub rename, the diff is premature and should fail closed.

---

## Blocking findings

### BLOCK-1: Missing proof that generated `release.yml` actually has `tag-push` behind the gate

**Paths:**
- `release/taskfleet-distribution.json` line `"trigger": "tag-push"`
- `issues/taskfleet-distribution-topology/plan.md` exact R9 substitutions
- `.github/workflows/release.yml` — not provided

The plan’s pre-R9 state is explicitly dispatch-only with `dispatch-releases = true` and default `dry-run`. The attached distribution contract now claims `tag-push`, but there is no generated workflow diff or `cargo dist generate`/`plan` receipt proving:

1. `dispatch-releases = false` was actually applied.
2. The workflow now has a tag-push trigger.
3. `taskfleet-release-gate.yml` still runs before the `host` job, artifact builds, release creation, or `HOMEBREW_TAP_TOKEN` use.
4. The generated output was not hand-edited, which ADR 0002 forbids.

If `release.yml` is still dispatch-only, the contract is false. If it has `tag-push` but the gate dependency is not correctly ordered, a pushed tag could reach the host job without a release-activation check.

**Blocks candidate CI?**
Yes, as an acceptance blocker. The candidate CI may pass unit/check jobs, but it cannot validate the R9 tag-trigger posture without the generated workflow diff and plan receipt.

---

### BLOCK-2: Activation state has two conflicting sources, and the release-gate script is not shown

**Paths:**
- `release/taskfleet-release.json` line `"activation": "blocked-r8-r9-r10"`
- `release/taskfleet-distribution.json` line `"activation": "prepared-blocked-r10"`
- `scripts/verify-release-activation.sh` — not provided
- `.github/workflows/taskfleet-release-gate.yml` — not provided

The R7 plan says release execution is blocked by `release/taskfleet-release.json`. It also says R9 should move distribution trigger/activation toward `tag-push`/`ready`, while the task statement requires `release/taskfleet-release.json` to remain blocked.

That can be consistent only if the release gate reads **only** `release/taskfleet-release.json`. The attached files do not prove this.

Risk:

- If `verify-release-activation.sh` reads `release/taskfleet-distribution.json` and its state becomes `ready`, a tag push could publish despite the release file being blocked.
- If the script still expects the old `blocked-r8-r9-r10` token, then the R9 transition is incomplete or the new distribution state will fail the gate incorrectly.
- The two files already disagree: one lists R8/R9/R10 as blockers; the other lists only R10.

Required:

- Show the diff for `scripts/verify-release-activation.sh`.
- Define one authoritative release activation file and one non-authoritative distribution-readiness file.
- Keep release activation blocked, but update it to reflect active blockers, e.g. `blocked-r10`, if R8 and R9 are cleared. Do not set `release/taskfleet-release.json` to `ready`.
- Add a CI assertion that rejects a tag-push path when release activation is not `ready`.

**Blocks candidate CI?**
Yes. This is a release-safety blocker for restoring tag dispatch. The candidate cannot prove the tag trigger is behind the activation gate without the gate script.

---

### BLOCK-3: Homebrew tap secret is still marked inert, contradicting the R9 plan

**Path:**
- `release/taskfleet-distribution.json` line `"tap_secret_state": "inert-blocked-r10"`

The R7 plan explicitly states:

> R9 must install a least-privilege token and prove it through the renamed repository’s actual checkout/push workflow.

The attached contract still says `inert-blocked-r10`. Either:

1. The least-privilege token has not been installed, so R9 is incomplete; or
2. The token has been installed, but the contract is stale and cannot be used as evidence.

A tag-push workflow backed by an inert secret cannot prove the Homebrew leg. If the secret is later activated without a proof, that violates the plan.

Suggested state split:

```json
"tap_secret_state": "installed-r9",
"tap_secret_proof": "<run-url-or-receipt>",
"tap_secret_activation": "blocked-r10"
```

Do not mark the secret live/ready until the release activation gate allows it.

**Blocks candidate CI?**
Yes if CI is supposed to exercise the token-backed checkout/push path. Even if CI does not touch the secret, this blocks R9 acceptance because the plan requirement is unmet or the evidence is inconsistent.

---

### BLOCK-4: No diff for maintained source identity substitutions

**Paths expected but not provided:**
- `Cargo.toml`
- `crates/taskfleet/Cargo.toml`
- `crates/taskfleet-core/Cargo.toml`
- `compat/orchestratectl/Cargo.toml`
- `dist-workspace.toml`
- generated installer/archive URL sources
- `.github/workflows/*`
- README/badges/docs
- release-wrapper expected-repo checks
- local composite action references

The attached release contracts are canonical, but that alone does not prove the source-owned repository identity converged. R9 requires:

- Cargo workspace `repository` / `homepage`: `https://github.com/jarimustonen/taskfleet`
- cargo-dist owner/repo: `jarimustonen` / `taskfleet`
- generated installer/archive URLs: canonical repository
- `origin`/push/fetch URLs
- GitHub Actions references that previously pointed to `jarimustonen/orchestratectl`

A green CI run can pass while old URLs remain in metadata, docs, or generated artifacts. That would still fail R9 acceptance.

Run and classify:

```bash
grep -RIn "github.com/jarimustonen/orchestratectl" \
  --exclude-dir=.git \
  --exclude="ADR*" \
  --exclude="issues/*" \
  .
```

Every remaining old identity must be classified as intentional history/protocol or fixed.

**Blocks candidate CI?**
Not necessarily as a red CI job, but blocks R9-based acceptance. The candidate diff does not contain enough files to verify the core identity transaction.

---

### BLOCK-5: Tag trigger restored with no tag creation protection

**Path:**
- `issues/taskfleet-distribution-topology/plan.md`, paragraph: “The source repository currently has no tag ruleset (receipted honestly)…”

The plan admits there is no tag ruleset. The proposed safety layer is workflow/wrapper activation gates. Those gates can stop publishing, but they cannot prevent a permanent tag ref from being created.

A stray or malicious tag push would:

1. Create a permanent version tag.
2. Trigger the tag-push workflow.
3. Possibly be blocked before release publication.
4. Still consume that version, violating ADR 0002’s “never retag/reuse a version” rule.

This is a release-safety regression. Before R9 restores tag dispatch, add a GitHub tag ruleset restricting tag creation to the release wrapper/owner actor, or keep tag dispatch disabled until the release wrapper is the only tag creator.

**Blocks candidate CI?**
No, not the CI run itself, but blocks safe R9 closure if tag push is restored without tag protection.

---

## High/evidence findings

### HIGH-1: Immutable rename evidence is absent

**Required by:**
- `issues/taskfleet-source-repository-rename/item.md` preconditions, required execution, acceptance criteria

The attached `release/taskfleet-distribution.json` only contains receipts for the new Homebrew tap:

- repository id `1355125556`
- proof commit `db12bb1...`
- empty tree `4b825dc...`

There is no source-repository rename receipt:

- old repo ID/name/default branch/visibility
- new repo ID after rename
- redirect behavior proof
- no known release/tag/tap/publication in flight
- secret names only inventory
- runner visibility
- remotes
- exact URL inventory
- fresh green exact-main `ci.yml` push run URL
- self-hosted macOS ARM64 leg execution

R9 cannot be closed authoritatively on the four files alone.

**Blocks candidate CI?**
No, but blocks acceptance evidence.

---

### HIGH-2: Self-hosted macOS ARM64 proof not shown

**Path:**
- `issues/taskfleet-source-repository-rename/item.md`: “Trigger candidate CI in the renamed repository so the self-hosted macOS runner and all Linux jobs execute.”

No run IDs, job URLs, or snapshot receipts are attached. `release/taskfleet-distribution.json` says `"macos_runner": "macOS"`, but that is configuration, not evidence.

**Blocks candidate CI?**
The CI cannot be considered valid for R9 without the macOS runner execution receipt.

---

### HIGH-3: Stub installer URL/hash not revalidated after rename

**Paths:**
- `release/taskfleet-distribution.json` `"stub_sha256": "6d171a..."`
- `issues/taskfleet-distribution-topology/plan.md`: “verify it resolves after rename”

After rename, the old `releases/latest/download/orchestratectl-installer.sh` URL must resolve through the GitHub redirect and the stub must print the canonical installer URL, change nothing, and exit non-zero.

Required evidence:

1. HTTP request to the URL after rename.
2. Stub body hash equal to `6d171a...`.
3. Stub output exactly canonical installer URL and exit code `1`.

If the stub was regenerated, its hash must be recalculated.

**Blocks candidate CI?**
No, but blocks release/artifact readiness evidence.

---

## Medium findings

### MED-1: `release/taskfleet-release.json` still lists defeated gates

**Path:**
- `release/taskfleet-release.json` line `"activation": "blocked-r8-r9-r10"`

After R8 and R9 are accepted, this remains stale unless the token is intentionally frozen. Leaving defeated gates in the active blocker string makes state interpretation ambiguous.

Suggested update while keeping release blocked:

```json
"activation": "blocked-r10"
```

Do not set this to `ready` in R9.

---

### MED-2: `source_repository.current` and `after_r9` are identical

**Path:**
- `release/taskfleet-distribution.json`:
```json
"source_repository": {
  "current": "jarimustonen/taskfleet",
  "after_r9": "jarimustonen/taskfleet"
}
```

This may be correct as a post-rename current state, but it loses the transition. If this field is used as evidence, it should record:

```json
"source_repository": {
  "pre_r9": "jarimustonen/orchestratectl",
  "current": "jarimustonen/taskfleet",
  "rename_receipt": "<receipt-url>"
}
```

Otherwise the field provides no proof of the one-way transaction.

---

## Bottom line

- Do not merge/accept R9 on these four files.
- `release/taskfleet-release.json` can remain blocked, but the release-safety path is not proven.
- The generated `release.yml`, release gate, activation script, Cargo/dist manifests, and identity scan are missing.
- The Homebrew tap secret state contradicts the R9 plan.
- Tag-push without tag protection is a release-safety risk even when activation stays blocked.

## Context request

- **Kind:** artifact
  **Need:** full uncommitted diff, including `git diff --stat` and patch for `.github/workflows/release.yml`, `.github/workflows/taskfleet-release-gate.yml`, `.github/workflows/publish-crates.yml`, `.github/workflows/ci.yml`, `scripts/verify-release-activation.sh`, Rust/shell posture checks, `Cargo.toml`, crate manifests, `dist-workspace.toml`, and documentation/badges
  **Why:** needed to verify every R9 exact source identity substitution and the tag-trigger activation gate.

- **Kind:** artifact
  **Need:** cargo-dist 0.28.2 `generate` and `plan` output/manifest after R9 substitutions, plus generated `release.yml`
  **Why:** proves dispatch-releases setting, tag trigger, intended artifacts, and single canonical formula without hand-editing.

- **Kind:** artifact
  **Need:** source repository rename before/after receipts: old/new GitHub repo IDs, default branch, visibility, redirect proof, no-in-flight release/tag/tap/publication check, sanitized secret inventory, runner visibility, remotes, exact URL inventory
  **Why:** required by the R9 issue acceptance criteria and not present in the attached contracts.

- **Kind:** artifact
  **Need:** fresh green exact-main `ci.yml` run URL and self-hosted macOS ARM64 job receipt in the renamed repository, including job IDs and snapshot results
  **Why:** R9 requires all CI jobs, including the macOS runner, before final exact-main is authoritative.

- **Kind:** clarification
  **Need:** is `release/taskfleet-release.json` the sole source of truth for `verify-release-activation.sh`, or does `release/taskfleet-distribution.json.activation` also gate release?
  **Why:** determines whether a `ready` distribution state can bypass the release block.

- **Kind:** clarification
  **Need:** has `HOMEBREW_TAP_TOKEN` been replaced with a least-privilege token under the renamed repository, and if so, what proof run URL exists?
  **Why:** the attached distribution file still says `inert-blocked-r10`, contradicting the R7/R9 plan.

- **Kind:** clarification
  **Need:** is a GitHub tag ruleset available or planned before tag-push is restored?
  **Why:** workflow gates alone cannot prevent permanent tag creation and version consumption.

- **Kind:** artifact
  **Need:** stub installer URL response + output + hash after rename
  **Why:** confirms old installer URL compatibility without requiring release publication.
