[thread_id:group_81bedd5984804aaabbefbbdd450c6d88]

## Model: gemini-3.1-pro-preview
[model:gemini-3.1-pro-preview] [thread_id:api_22a5a63de6c64ce68d8720380af53f29]

### Review against Updated Context

The provided context explicitly addresses and resolves the critical flaws identified in the previous review. The inclusion of complete logs, expanded harness checks, tight file hashing, and full transparency over exploratory probes makes this a highly rigorous evidence package.

**How previous conclusions have changed:**
1.  **Hand-Forged Release Artifacts (Previous BLOCKER):** **RESOLVED.** The context clarifies this is a localized `0.5.1 -> 0.6.0` tap migration simulation, not the production artifact itself. The updated `verify-homebrew-prelive.sh` now enforces tight cryptographic bounds (`hashlib.sha256`) against the local archive and asserts exactly one text substitution. This safely isolates the simulation without violating the overarching cargo-dist release rules.
2.  **Untested Legacy Cargo Wrapper (Previous BLOCKER):** **RESOLVED.** `verify-install-channels.sh` now installs the legacy `--path "$repo_root/compat/orchestratectl"`, strictly verifies the `orchestratectl` executable is isolated (and `taskfleet` is absent), executes it, verifies the exact-SHA binding of the shared dispatch engine, and validates the stderr deprecation warning.
3.  **Skipped State Quiescence Stress Test (Previous BLOCKER):** **RESOLVED.** Acknowledged as a standard source-ignored test (`#[ignore]`) rather than a skipped R8 gate requirement. Standard `cargo test` behavior is accepted.
4.  **Dangerous "LEAK" Marker Ignored (Previous HIGH):** **RESOLVED.** `leak-disposition.json` correctly attributes the non-deterministic `LEAK` marker to process-free schema unit tests under scheduler load, differentiating it from true zombie child processes. Tracking this as a disclosed warning subject to final `residue.json` process-tree checks is the correct structural disposition.
5.  **Platform Hardcoding (Previous HIGH):** **RESOLVED.** Acknowledged that local artifacts are natively macOS ARM64 and that `cargo-dist generate --check` sufficiently enforces Linux musl target definitions for the R10 hosted-build boundary. The updated script securely verifies archive contents using `rglob`.
6.  **Untested Legacy Upgrade Alias (Previous MEDIUM):** **RESOLVED.** `verify-homebrew-prelive.sh` now explicitly executes `brew upgrade jarimustonen/orchestratectl/orchestratectl`. The attached `homebrew-acceptance.log` proves Homebrew natively intercepts this and successfully resolves it to `jarimustonen/taskfleet/taskfleet` via the `tap_migrations.json` mechanism.

### What is Genuinely Solid
*   **Absence Assertions:** The addition of the `assert_absent` bash function gracefully handling broken symlinks (`-L`) is an excellent defensive shell programming choice that prevents false positives during channel testing.
*   **Index Binding:** `verify-evidence-index.py` now cryptographically binds every R8 harness script, log, and JSON artifact to the exact tested commit. The Python logic enforcing the "pass" state machine before allowing `--write` to authorize R9 is rock solid.
*   **Command Parity Matrix:** The dynamic `jq` parsing of `structured --help --depth full` to map all 33 command surfaces leaves zero room for untracked CLI regressions.

### Remaining Risks / Minor Gaps
*   **None that block R9.** The remaining `pending-review-and-residue` states for `diff-residue` and `evidence-review` are expected pre-finalization markers. The execution of the evidence index script correctly sets `r9_authorized: false` until those final mechanical steps flip the manifest to `overall: pass`.

### Verdict
**PASS after mechanical finalization.**

The R8 integrated validation evidence is cryptographically sound, respects isolation boundaries, correctly models the Homebrew cross-tap migration, and proves the legacy Cargo bounded wrapper works as designed. Proceed to final process/worktree residue checks, commit the final index, and authorize R9.

---

## Model: gpt-5.6-sol
[model:gpt-5.6-sol] [thread_id:api_cfe072ccea804d2e961b7097350754bb]

# Revised review

## Conclusions that changed

The new evidence resolves several findings from the first review:

- Raw/sanitized logs now exist for the principal gates.
- The evidence index now covers the issue-local harness scripts and validation report.
- The old/unknown 0.5.1 coverage is now mapped to named tests.
- Homebrew is pinned to an exact Git commit and uses a private cache.
- Broken-symlink-safe absence assertions were added.
- Canonical and compatibility Cargo install flows are both exercised.
- The Homebrew receipt migration now checks `source.tap`, upgrades through the old fully qualified identity, and validates canonical resolution JSON.
- The formula rewrite now fails if the expected version, URL, or archive digest is absent.
- The inaccurate claim that the ignored stress test ran separately was removed.
- The delayed-exit marker is now supported by the complete nextest output and a specific disposition.

Those changes remove most of the original evidence gaps. They do not make the current package authorizable because the newly supplied artifacts expose a direct contradiction in the Homebrew evidence and defects in final authorization logic.

# Problems found

## BLOCKER

### 1. The attached candidate formula should fail during installation, contradicting the successful Homebrew log

`target/distrib/taskfleet.rb` defines `BINARY_ALIASES` with a string key:

```ruby
BINARY_ALIASES = {
  "aarch64-apple-darwin": {}
}
```

Despite the colon syntax, a quoted label is a string key in Ruby. The formula then looks it up using a symbol:

```ruby
BINARY_ALIASES[target_triple.to_sym].each do |source, dests|
```

For `target_triple == "aarch64-apple-darwin"`:

```ruby
BINARY_ALIASES[:"aarch64-apple-darwin"]
```

returns `nil`, so `.each` should raise `NoMethodError`. `install_binary_aliases!` is called unconditionally after installing the binary.

This formula is asserted to be the exact source for the local candidate:

- `verify-homebrew-prelive.sh` copies `$repo_root/target/distrib/taskfleet.rb`;
- it changes only the version and URL;
- it verifies the candidate archive SHA;
- `distribution-artifact-hashes.txt` pins `taskfleet.rb`;
- `homebrew-acceptance.log` claims the resulting formula installed successfully.

These facts cannot all be true under normal Ruby semantics. The candidate installation should fail before the success message.

This is not a documentation weakness. It means at least one of the following is wrong:

1. the attached formula is not the formula used by the passing run;
2. the log is not from the attached corrected harness/artifacts;
3. the formula shown was generated after or before the authoritative run;
4. the formula was modified in another unrecorded way;
5. the generated formula contains a real installation defect and the claimed test did not execute it as described.

The harness should retain and hash the actual rewritten candidate formula, not only the source formula. It should also perform a direct preflight assertion such as:

```ruby
abort "missing target alias map" unless BINARY_ALIASES.key?(target_triple)
```

The likely formula correction is one of:

```ruby
BINARY_ALIASES = {
  :"aarch64-apple-darwin" => {}
}
```

or, preferably:

```ruby
BINARY_ALIASES.fetch(target_triple).each do |source, dests|
```

using the existing string key and failing clearly for unsupported targets.

Because the currently attached Homebrew artifacts are internally inconsistent, the Homebrew acceptance leg is not credible. R8 cannot authorize R9 until the exact formula used by the run is reconciled and the affected distribution/Homebrew legs are rerun.

---

### 2. The “corrected” stripped-PATH run still reports that `xcrun` is unavailable

`validation.md` says the initial missing-`xcrun` setup was corrected and that only the corrected full run counts. However, the authoritative `evidence/logs/stripped-path-final.log` begins with:

```text
warning: invoking `"xcrun" "--sdk" "macosx" "--show-sdk-path"` to find MacOSX.sdk failed:
No such file or directory (os error 2)
```

It then reports that several test binaries generated that warning.

This directly contradicts:

- the description of the initial `xcrun` omission as superseded;
- the prior diagnostic disposition saying `xcrun` was explicitly declared in the acceptance tool directory;
- the characterization of this as the corrected stripped-PATH run.

The tests did run and all assertions passed, so this does not prove a product failure. It does prove that the isolation harness was not corrected in the manner claimed. It also leaves unclear whether the resulting binaries were built with the intended SDK metadata.

The evidence must either:

- rerun with a functional, explicitly resolved `xcrun`; or
- accurately classify the final run as intentionally excluding `xcrun` and explain why the SDK lookup warning is acceptable.

It cannot continue describing the missing helper as belonging only to superseded attempts.

Given that clean-PATH validation is an explicit acceptance leg, this contradiction blocks final authorization unless corrected.

## HIGH

### 3. Final authorization still accepts semantically wrong results for required commands

`verify-evidence-index.py` improves on the original version by validating required IDs and rejecting unknown result strings. It still uses one global set:

```python
PASS_RESULTS = {
    "pass",
    "pass-with-disclosed-warning",
    "pass-with-known-warnings",
    "expected-refusal",
}
```

Then every required command may use any value in that set:

```python
bad = {
    row["id"]: row.get("result")
    for row in commands
    if row.get("result") not in PASS_RESULTS
}
```

This would authorize nonsensical combinations such as:

```json
{"id":"rust-nextest","result":"expected-refusal"}
{"id":"ci-api","result":"pass-with-known-warnings"}
{"id":"evidence-review","result":"expected-refusal"}
```

The release-activation gate is the only command expected to refuse. Warning-bearing results should also be restricted to explicitly admitted command IDs.

Use per-command allowed outcomes:

```python
REQUIRED_RESULTS = {
    "ci-api": {"pass"},
    "rust-fmt": {"pass"},
    "rust-clippy": {"pass"},
    "rust-nextest": {"pass"},
    "rust-doctest": {"pass"},
    "rustdoc": {"pass"},
    "insta": {"pass"},
    "stripped-path": {"pass-with-disclosed-warning"},
    "wrapper-parity": {"pass"},
    "legacy-baseline": {"pass"},
    "legacy-current": {"pass"},
    "state-config-migration": {"pass"},
    "registry-protocol": {"pass"},
    "shipshape-protocol": {"pass"},
    "release-activation": {"expected-refusal"},
    "package": {"pass"},
    "install-channels": {"pass"},
    "cargo-dist-homebrew-resolution": {"pass"},
    "legacy-installer-stub": {"pass"},
    "homebrew-old-receipt": {"pass"},
    "shipshape-contract": {"pass"},
    "shipshape-plan": {"pass"},
    "public-facts": {"pass"},
    "identity-ledger": {"pass"},
    "issue-gates": {"pass-with-known-warnings"},
    "diff-residue": {"pass"},
    "evidence-review": {"pass"},
}
```

The verifier should also inspect `assessment.json` and require an allowed final assessment verdict rather than treating file existence as proof of review success.

---

### 4. The finalized index cannot be verified with the normal no-argument invocation

The verifier computes:

```python
manifest = validate_manifest(
    final=writing and
    json.loads(...).get("overall") == "pass"
)
```

When invoked without `--write`, `writing` is false, so `final=False`. `validate_manifest()` then contains:

```python
if not final and manifest.get("overall") == "pass":
    fail("manifest overall=pass but final validation was not requested")
```

Therefore:

1. `verify-evidence-index.py --write` can create a final passing index.
2. A later plain `verify-evidence-index.py` invocation against that final index always fails.

That defeats the stated purpose of a persistent verification command.

Separate “writing” from “final state”:

```python
manifest_data = json.loads((evidence / "command-manifest.json").read_text())
is_final = manifest_data.get("overall") == "pass"
manifest = validate_manifest(final=is_final)
```

Then use `writing` only to decide whether to rewrite `index.json`.

---

### 5. The command parity harness still does not literally execute every public command under both names

The new harness is materially stronger:

- all 33 visible structured-help command surfaces are inventoried;
- the complete structured command tree is compared;
- seven representative ordinary commands compare stdout, non-warning stderr, and exit status;
- wrapper-specific help and hidden self-exec behavior are checked.

However, ADR 0002 says:

> Every public command under canonical and wrapper names has identical stdout, JSON/JSONL, and exit codes.

The harness does not execute every public command under both names. It structurally compares their Clap definitions and executes seven representative commands. The acceptance matrix silently weakens the requirement into:

> every public command surface ... plus ordinary ... parity

Shared dispatch makes equivalence likely, but it does not prove there are no invocation-identity branches in command execution. The test suite even contains explicit invocation-identity behavior, so this is not a purely theoretical distinction.

Either:

- change the acceptance interpretation explicitly, with a reason that one shared dispatcher plus complete structure and representative execution is considered sufficient for R8; or
- generate a safe argument fixture for all 33 surfaces and compare at least their validation/help/dry-run behavior under both binaries.

The current statement “every required R8 leg passed” is stronger than the implemented parity check.

---

### 6. Structured-help normalization is overbroad and can conceal real identity defects

`verify-command-parity.sh` says:

> Invocation branding is the only normalized field

but actually applies this transformation to every string anywhere in the compatibility JSON:

```bash
jq -S '
  walk(
    if type=="string"
    then gsub("orchestratectl";"taskfleet")
    else .
    end
  )
'
```

This can hide unintended old-name occurrences in:

- descriptions;
- examples;
- environment-variable documentation;
- defaults;
- diagnostics;
- URLs;
- compatibility notes;
- unrelated string values.

It does not normalize only the invocation-brand field.

The comparison needs path-specific normalization, or it must record all changed JSON paths and verify they belong to an explicit allowlist. For example:

```bash
jq -S '
  .data.command = "taskfleet"
  | walk(
      if type == "object" and has("usage")
      then .usage |= sub("^orchestratectl"; "taskfleet")
      else .
      end
    )
'
```

The exact paths depend on the help schema. A blanket string replacement is not acceptable evidence for the claim that only documented branding differs.

## MEDIUM

### 7. Invalid-input parity does not compare non-deprecation stderr

For ordinary commands, the harness removes the compatibility deprecation line and compares the remaining stderr:

```bash
grep -vF ... "$tmp/compat.err" >"$tmp/compat.filtered.err"
cmp "$tmp/canon.err" "$tmp/compat.filtered.err"
```

For invalid input, it checks only:

- equal nonzero status;
- equal stdout;
- one compatibility warning.

It does not compare canonical stderr with compatibility stderr after removing the warning. An invocation-specific parse error could therefore pass.

Apply the same filtered-stderr comparison to invalid input.

---

### 8. The archive check is narrower than the acceptance-matrix wording

`acceptance-matrix.json` says install validation covers:

> all archive executable/member names

The script actually checks:

```python
executables == ["taskfleet"]
not any(p.name == "orchestratectl" ...)
```

and:

```bash
! tar -tf "$archive" | grep -Eq '(^|/)orchestratectl$'
```

This does not validate all member names against an allowlist. It validates:

- the executable set;
- absence of an exact `orchestratectl` basename.

That is probably enough for the old-command ownership requirement, but the matrix overstates it. Either change the wording to “all executable names and absence of an exact old-command member” or preserve and compare a full archive-member allowlist.

---

### 9. Network setup remains outside the credential-isolated environment

`verify-homebrew-prelive.sh` still performs:

```bash
git clone -q https://github.com/jarimustonen/homebrew-orchestratectl.git ...
```

outside `env -i`. It can consult ambient:

- Git configuration;
- credential helpers;
- proxy settings;
- URL rewrites;
- SSH/GitHub helper configuration.

The clone is read-only and public, so this is not equivalent to a public mutation. It does weaken the statement that the drill is credential-isolated.

Use a dedicated Git environment:

```bash
env -i \
  HOME="$tmp/git-home" \
  PATH=/usr/bin:/bin \
  GIT_CONFIG_NOSYSTEM=1 \
  git -c credential.helper= clone ...
```

This is not independently blocking once the two blocker contradictions are resolved, but the final report should avoid saying all setup was credential-free unless it is actually enforced.

---

### 10. Command execution records still do not uniformly preserve exit codes and timestamps

The new complete logs are a major improvement, but many are still plain combined command output. They do not independently record:

- start and end times;
- exact environment or environment digest;
- exit status per command;
- exact harness SHA at execution time.

For scripts using `set -e` and a final success line, a zero exit is reasonably inferable. For multi-command logs such as `full-rust-gate.log`, successful progression to the final section also provides practical evidence. This is adequate for bounded R8 review, but weaker than a structured execution receipt.

A future integrated run should produce a per-command receipt rather than relying on shell-flow inference.

---

### 11. The user-state incident remains an unresolved process violation

The revised report now correctly says:

- there was no pre-probe digest;
- complete restoration cannot be independently proved;
- mtimes changed;
- the probes are not passing gate evidence.

That is honest and removes the earlier false restoration implication.

It still means the overall R8 activity did not satisfy the literal requirement that every mutation destination be sandboxed. The authoritative replacement runs being isolated limits the evidentiary impact, but does not erase the incident.

This is not evidence of a Taskfleet state-integrity product defect, and it does not invalidate the exact-SHA test results by itself. It should remain a recorded exception requiring explicit owner disposition. The final report must not state that every command executed during the R8 effort was sandboxed; only every authoritative gate execution can be described that way.

## LOW

### 12. Public command inventory provenance is indirect

`public-command-inventory.json` contains only the resulting 33 command strings. It does not include:

- the raw structured-help digest;
- canonical and compatibility help digests;
- the list of normalization paths;
- the binary digests.

`command-parity.log` may cover some of this, but the inventory itself is not a sufficient standalone derivation receipt. This becomes more important once normalization is narrowed.

---

### 13. `validation.md` remains prematurely phrased as a pass

The report still says:

> **PASS, subject to the immutable evidence index and review below.**

The machine state remains pending, which is expected at this stage. Given the formula and stripped-PATH contradictions, however, this is no longer merely premature sequencing language. Until those contradictions are resolved, the correct status is “candidate evidence failed review.”

# Delayed-exit LEAK disposition

The supplied context materially changes the initial assessment of this warning.

The complete log shows:

```text
LEAK ... schema::tests::wire_names_match_serde_round_trip
...
Summary ... 1115 passed (3 slow, 1 leaky)
```

The disposition also records that another run attributed the marker to a different process-free unit test. The ordinary full release run had no leak marker, every assertion passed, and the marker moved under load.

That is credible evidence of nextest’s delayed test-binary exit attribution rather than a deterministic leaked child from either named test. A nextest `LEAK` advisory is not itself a failed assertion or nonzero gate result. Subject to final no-process residue, `pass-with-disclosed-warning` is a defensible disposition.

The LEAK marker does **not** block R8 on the supplied evidence. The missing-`xcrun` warning in the same log is a separate issue and must not be conflated with it.

# What is genuinely solid

- Exact source commit and tree identity are consistently stated.
- The full release run shows all 1,115 required executed tests passing.
- The named migration tests cover adoption, active/pending refusal, byte preservation, rollback, first-write closure, crash recovery, lock contention, symlink faults, and live-worker/pending-merge cases.
- The unknown compatibility case is now supported by named tests, including `removed_kinds_deserialize_to_unknown` and legacy removed-kind read-only behavior.
- The Homebrew topology correctly starts from a real 0.5.1 old formula and receipt before applying local migration metadata.
- Homebrew version identity is now exact rather than loosely described as a release tag.
- Private Homebrew and Cargo caches/targets significantly improve isolation.
- Failed exploratory attempts remain separated from acceptance evidence.
- The public command inventory and structural comparison are substantial improvements, despite the normalization flaw.
- R8 still explicitly refuses release authorization and limits any eventual authority to R9.
- Pending residue and review markers are correctly left pending for final mechanical steps.

# Risks that invalidate R9 authorization

R9 must remain unauthorized until:

1. the string-key/symbol-key formula contradiction is resolved and the exact corrected formula is rerun through the distribution and Homebrew acceptance legs;
2. the final stripped-PATH run either includes a functioning `xcrun` or accurately documents and admits the SDK warning;
3. `verify-evidence-index.py` uses per-command expected results;
4. the no-argument verifier works against a finalized passing index;
5. parity normalization is path-specific rather than replacing every old-name string;
6. final residue and assessment complete successfully.

The first two are not mechanical pending-state updates. They require corrected evidence and affected reruns.

# Verdict

**BLOCK R8**

The newly attached evidence is substantially better, and the nextest LEAK warning is not a blocker. However, the candidate Homebrew formula appears incapable of producing the attached successful run, and the authoritative stripped-PATH log contradicts the claimed `xcrun` correction. Those are direct evidence inconsistencies. R9 must not be authorized until they are reconciled and rerun.

---

## Model: claude-fable-5
[model:claude-fable-5] [thread_id:api_274dbce5f90846a48b591390383a45c1]

# Revised review — R8 integrated validation evidence (post-correction)

## Conclusions that change

- **H2 (CI job-count reduction) — WITHDRAWN.** `r8-ci-jobs-summary.json` shows all eight jobs: version-snapshots, msrv (1.85), cargo-deny, test (ubuntu-latest), test (macos-latest), rustfmt, clippy, docs. The earlier ~10-leg narrative counted bump-hook/release-wrapper as steps inside `version-snapshots`. No gate was dropped. This also removes my Linux-coverage concern for the previously failing fixture: the Ubuntu `version-snapshots` job that killed the first R8 attempt now passes at the exact SHA.
- **H5 (hand-editable authorization gate) — CLOSED.** `verify-evidence-index.py` now enforces the required command-ID set, refuses `overall=pass` with any non-`PASS_RESULTS` row, verifies every referenced output exists at finalization, indexes the harness scripts and `validation.md`, and hard-fails `release_authorized != false`. This is the fail-closed design I asked for. (One new bug in it, below.)
- **H4 (LEAK marker) — DOWNGRADED to accepted disclosed warning.** `leak-disposition.json` shows the marker moved between two provably process-free unit tests across runs (`cli::drain_cell_on_absent_guard_is_noop` vs `schema::wire_names_match_serde_round_trip`), which is the signature of scheduler-delayed binary exit, not a leaked child. The `@native-spawn-test-leaks` linkage is now made and correctly distinguished (real-worktree spawn validation, unrelated mechanism). Disposition `pass-with-disclosed-warning` is defensible, **conditional on the pending residue check finding no candidate process**. The stripped log confirms the marker on line 1064 exactly as disclosed.
- **H3 (real legacy-log mutation) — RECLASSIFIED from fixable gap to permanent caveat.** No pre-probe digest exists; complete byte restoration is unprovable by construction, and the report now says so honestly. This cannot be closed with more evidence. The acceptance clause "every mutation destination is sandboxed" was factually violated during exploration. This is now a **human sign-off item**, not an evidence request: an authorized human must explicitly accept the incident as a bounded, disclosed exploratory-probe breach before `overall` flips. Mitigating: only appended dispatch-log lines were involved (not events/projections/config), and `dual_name_resolver` tests independently pin that this class of write cannot corrupt adoption semantics.
- **M1 (hash-only logs) — LARGELY CLOSED.** Sanitized full logs are committed for the main gates and match the disclosed content (full nextest listing, LEAK line, Homebrew migration sequence, dual Cargo installs). Residual note: the `raw_sha256` values reference unsanitized originals, so committed-log bytes intentionally differ from those hashes; the chain is now (sanitized committed log) + (raw hash), which is acceptable but should be stated once in the index.
- **M2/M3 (Homebrew version guard, `/tmp` cache leak) — CLOSED.** Exact version **and** git commit pinned at both host and cloned prefix; cache is now `$tmp/cache` under the trap.
- **M4 (parity coverage) — CLOSED with one residual blind spot** (below). 33 surfaces structurally compared against a derived inventory; the 7-command byte comparison is retained; stateful parity is owned by `compatibility.rs`.
- **M5 (unknown-outcome adoption) — CLOSED.** `acceptance-matrix.json` maps it, and `legacy_removed_kind_run_is_read_only` plus `removed_kinds_deserialize_to_unknown` appear in the executed run.
- **M6 (artifact binding) — CLOSED.** Both channel scripts now `shasum -c` against `distribution-artifact-hashes.txt` before use; the formula rewrite asserts exactly-one substitution and archive digest match fail-closed. The rewritten formula is a local simulation input, correctly not represented as a release artifact.
- **M7 (macOS-only channels) — CLOSED as disclosure.** Now explicit in `validation.md`; Linux musl targets validated at plan level; hosted cross-platform artifacts correctly assigned to R10.
- **Prior LOW items** (perm-bit check, broken-symlink absence, shared target dir, old-name `brew info` content check) — all fixed in the reattached scripts.

## Remaining findings

### MEDIUM

**R1. Verify-mode of `verify-evidence-index.py` fails after finalization.**
`final` is computed as `writing and manifest.overall=="pass"`. In plain verify mode (`no --write`), `final=False`, and `validate_manifest` then executes:

```python
if not final and manifest.get("overall") == "pass":
    fail("manifest overall=pass but final validation was not requested")
```

So once the manifest is legitimately finalized to `pass`, every subsequent integrity re-verification of the committed evidence **fails** — exactly the run future auditors and R9/R10 will perform. Fix before finalization:

```python
final = json.loads((evidence / "command-manifest.json").read_text()).get("overall") == "pass"
# --write with a non-pass manifest still writes a pending index; verify mode
# derives final from the manifest itself.
```

(and keep the guard only for the write path if desired). One-line evidence-harness fix; must land before `--write` finalization or the index becomes self-invalidating.

**R2. `validation.md` Result section still asserts "PASS ... Every required R8 leg passed" while `diff-residue` and `evidence-review` are required IDs in `pending` state.** The hardened verifier now makes this mechanically harmless (it cannot mint `r9_authorized: true`), so this drops from HIGH to a wording defect — but it is still a factual overstatement at commit time. Either phrase it conditionally ("passes upon residue/review completion") or finalize the two legs before this file is treated as authoritative.

### LOW

**R3. Parity normalization blind spot.** `gsub("orchestratectl";"taskfleet")` on *all* wrapper strings would mask a defect where the wrapper wrongly brands something that must stay canonical even under the old name (e.g., a default home path or canonical URL rendered as `orchestratectl` in wrapper help where canonical says `taskfleet`). `invocation_identity_is_not_inferred_from_argv0_or_path` and the compatibility suite partially cover this, but the structural comparison itself cannot detect it. Worth one targeted assertion (e.g., wrapper help must not contain `~/.orchestratectl` outside documented deprecation text) in a future pass; not R9-blocking.

**R4. Potential stale-artifact inconsistency: Homebrew commit drift between artifacts.** `toolchain.json` and the script pin `6.0.21-70-g2316567`, but the originally attached `homebrew-acceptance.json` recorded `6.0.21-52-g27d05ae`. If the acceptance JSON was not regenerated by the corrected rerun, the evidence set is internally inconsistent. Mechanical check at finalization: assert `homebrew-acceptance.json.homebrew_version` equals the toolchain/script pin.

**R5. Confirm the stale "exercised separately in R8" sentence is actually gone from the *hashed* `outputs-summary.txt`.** The clarification says it was removed; the reattached `command-manifest.json` is consistent, but `outputs-summary.txt` was not reattached. Since it is an indexed, hashed artifact, the removal must be reflected in the final digest rows.

**R6. Inventory jq filter fragility.** `select(.hidden==false)` silently drops any command node lacking a `hidden` field. The count of 33 matches the committed inventory, so this is presently correct, but the filter should use `(.hidden // false) == false` to fail toward inclusion rather than exclusion.

**R7. Sanitized-vs-raw hash relationship undocumented.** State once (index or validation.md) that committed logs are sanitized transforms of the raw logs whose SHA-256s are recorded, so nobody flags the mismatch as tampering.

## Unresolved by design (not defects)

- `diff-residue` (`residue.json`) and `evidence-review` legs pending until after this review — correct sequencing; both are enforced by the hardened verifier at finalization.
- Homebrew evidence rests on a post-6.0.21 dev commit; exact-pinned and disclosed. Residual semantic-drift risk vs. released 6.0.21 is accepted and R10 re-proves against whatever Homebrew is then current.
- Legacy-log incident (H3) is permanently unverifiable; carried as a disclosed caveat requiring explicit human acceptance.

## What is solid

Unchanged from before, now stronger: exact-SHA binding enforced at runtime in every channel; superseded-failure disclosure discipline; correct migration-boundary modeling in the Homebrew drill (now with digest- and identity-fail-closed assertions); the acceptance matrix gives per-criterion → test/artifact traceability I previously had to reconstruct; and the eight-job CI summary fully substantiates the exact-SHA green claim.

## Verdict

**PASS after mechanical finalization**, with these finalization preconditions, all evidence-side and mechanically checkable:

1. Fix R1 in `verify-evidence-index.py` before running `--write` on a pass manifest.
2. Complete `diff-residue` (must confirm no candidate process, closing the LEAK disposition) and `evidence-review` legs; only then flip `overall` and reword/confirm R2.
3. Assert Homebrew-version consistency across `toolchain.json` / script / `homebrew-acceptance.json` (R4) and confirm the corrected `outputs-summary.txt` bytes are what the index hashes (R5).
4. Record explicit human acceptance of the disclosed, unprovable legacy-log restoration incident as a waived sandbox-clause breach.

Authorization scope remains R9-only; nothing here narrows or extends that.

---

## Model: deepseek-v4-pro
[model:deepseek-v4-pro] [thread_id:api_f0ee66622ad04a769762776caaaf2818]

## Revised verdict

**FIXABLE EVIDENCE GAPS.** Do not set `r9_authorized: true` until Finding 1 is reconciled and final residue/review/index finalization succeeds against the corrected evidence set.

This is not a repeat of the prior BLOCK R8. The major structural defects were fixed; the remaining items are bounded and can be resolved with localized evidence/triage changes.

---

## Conclusions that changed

| Prior finding | Current status |
|---|---|
| Finalization script could flip R9 from one manifest string | **Resolved.** `verify-evidence-index.py` now validates required command IDs, rejects `overall=pass` with any non-passing result, hashes scripts/validation/evidence, and refuses release authorization. |
| Homebrew formula URL rewritten without digest validation | **Resolved.** The script now computes the local archive SHA-256, asserts the generated formula contains it, and rewrites only the pre-live version label/URL. The digest in the attached formula matches `distribution-artifact-hashes.txt`. |
| Stress test “exercised separately” unsupported | **Resolved.** That sentence was removed. The manifest now states the source-marked expensive test is not a required executed leg. |
| Clean-PATH LEAK marker treated as silent pass | **Accepted** under the recorded project disposition. `leak-disposition.json` plus the final log support `pass-with-disclosed-warning`, but final no-process residue remains a required closing check. |
| Raw logs not preserved | **Resolved.** Sanitized complete logs are committed under `evidence/logs/` and iterated by the index. |
| Scripts/validation not hashed | **Resolved.** Root harness scripts and `validation.md` are now included in index artifact paths. |
| Mutations outside sandbox | **Resolved.** Private `$tmp/cargo-target`, `$tmp/cache`, and `$tmp`-scoped Homebrew/install roots are used. |
| Installer stub not explicitly manifest-bound | **Resolved.** `legacy-installer-stub` now has its own command entry. |

---

## Remaining findings

### HIGH

#### 1. `issue-dag.json` contains an untriaged issue that directly contradicts the Homebrew acceptance record

File: `issues/taskfleet-integrated-validation/evidence/issue-dag.json`

Under `unscheduled`:

```json
{
  "slug": "nominally-numberless-hand",
  "status": "untriaged",
  "title": "Old Homebrew receipt remains owned by legacy tap after migration"
}
```

This conflicts with:

- `evidence/homebrew-acceptance.json`:
  ```json
  "the migrated receipt source.tap was jarimustonen/taskfleet"
  ```
- `evidence/logs/homebrew-acceptance.log`:
  ```text
  ==> Migrating formula orchestratectl to taskfleet
  ```

`homebrew-diagnostics.json` records superseded attempts with the same symptom, so this untriaged issue may be merely an un-triaged artifact of those diagnostic failures. But the R8 package currently includes both an accepted assertion and an open issue stating the opposite.

Before R9 authorization, this issue must be:

- closed as superseded/duplicate, or
- explicitly classified as out-of-scope in the final review/assessment with rationale.

Silence is not acceptable. This is the only remaining material contradiction in the evidence set.

---

### MEDIUM

#### 2. Final `residue.json`/review outputs must be placed under `evidence/` or the finalizer must be extended

`verify-evidence-index.py` hashes:

```python
paths = [p for p in evidence.rglob("*") if p.is_file() and p != index_path]
paths += [
    root / "validation.md",
    root / "verify-command-parity.sh",
    root / "verify-install-channels.sh",
    root / "verify-homebrew-prelive.sh",
    root / "verify-evidence-index.py",
]
```

But the pending outputs are bare:

```json
{"id": "diff-residue", "output": "residue.json"}
{"id": "evidence-review", "output": "review.md, assessment.json, assessment.md"}
```

The final manifest validator only globs those patterns under `evidence/`.

If the review/residue workflow emits these files at issue root, finalization will fail, or they will not be hash-bound by the index. Conversely, if they land under `evidence/`, the script works as intended.

The finalization step must therefore either:

- explicitly emit `residue.json`, `review.md`, `assessment.json`, and `assessment.md` under `evidence/`, or
- add those root-relative paths to `artifact_paths()` and the output validation logic.

Fail-closed behavior here is safe, but the intended file locations must be fixed before running `--write` with `overall=pass`.

---

### LOW

#### 3. Homebrew clone identity is not reasserted after explicit `brew update`

`verify-homebrew-prelive.sh` verifies the cloned Homebrew head before `brew update`:

```bash
[[ "$(git -C "$tmp/prefix/Homebrew" rev-parse HEAD)" == "$homebrew_head" ]]
```

But it does not re-check that commit after:

```bash
"${brew_env[@]}" "$brew_bin" update >/dev/null
```

Explicit `brew update` can advance the Homebrew repository clone in some configurations. The recorded toolchain and acceptance log should be exact even after the migration boundary inside the disposable prefix. Add a post-update Homebrew head assertion or disable Homebrew self-update for the drilled prefix.

#### 4. `verify-evidence-index.py` does not cross-check `acceptance-matrix.json`

The machine finalizer trusts `command-manifest.json` only:

```python
manifest = json.loads((evidence / "command-manifest.json").read_text())
```

`evidence/acceptance-matrix.json` still contains its own pending results:

```json
{"criterion": "exact source identity and no production changes after testing", "result": "pending-final-residue"}
{"criterion": "multi-model review and assessed findings", "result": "pending-review"}
```

The requirements are mirrored in the command manifest, so finalization will normally fail if they are not updated. But there is no explicit consistency check. Before finalization, the script should also assert that every `criteria[].result` in `acceptance-matrix.json` is terminal.

#### 5. Stripped-path log still contains an `xcrun` SDK warning

`evidence/logs/stripped-path-final.log` begins:

```text
warning: invoking `"xcrun" "--sdk" "macosx" "--show-sdk-path"` to find MacOSX.sdk failed: No such file or directory (os error 2)
```

The run exited 0 and all 1,115 tests passed, so this did not block. But the acceptance narrative says the corrected run used a declared `xcrun` helper. This warning suggests an SDK lookup still failed in the final run. It should be explained in `isolation-diagnostics.json` or eliminated in a clean rerun.

---

## Genuinely solid

- Exact-SHA binding is now consistent across `source-identity.json`, install channels, Homebrew candidate checks, parity harness, and the finalizer.
- The full Rust gate log shows 1,115 tests passing, clippy/rustdoc/fmt/diff-check clean, and the ordinary release run has no LEAK marker.
- Install and Homebrew harnesses now use private cache/target paths and broken-symlink-safe absence assertions.
- The Homebrew formula substitution is fail-closed: exact-one formula URL/version occurrence, exact archive digest match, and post-install commit verification.
- The command parity script now compares the complete visible CLI tree rather than only several sample commands.
- The finalization script is materially stronger and fail-closed: it refuses final pass with non-passing commands and refuses any release authorization.
- DAG blockers for `taskfleet-integrated-validation` are empty; `blockers_open` and `blockers_missing` are both `[]`, and the issue is head-of-line.

Final R8 pass is close, but Finding 1 requires a deliberate human/triage outcome before the machine index may record `r9_authorized: true`.
