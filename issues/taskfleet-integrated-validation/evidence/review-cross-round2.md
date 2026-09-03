[thread_id:group_9d46c10f9e64412a8757d1ec07c0cf1c]

## Model: gemini-3.1-pro-preview
[model:gemini-3.1-pro-preview] [thread_id:api_22a5a63de6c64ce68d8720380af53f29]

### Critique of Other AI Reviews

The other reviewers are either hallucinating or failing to read the updated code logic.

*   **OpenAI is hallucinating.** Their claim that the Homebrew formula `BINARY_ALIASES` syntax (`"aarch64-apple-darwin": {}`) is an invalid Ruby String key that will fail a Symbol lookup is factually incorrect. In Ruby 2.2+, `"string_key": value` evaluates directly to a Symbol (`:"string_key" => value`). Furthermore, they demand `env -i` Cargo installs, which you already implemented in the updated `verify-install-channels.sh`. Dismiss their assessment entirely.
*   **Anthropic and DeepSeek are lazy.** They are asking for "strict commit-bound residue" and "exact-once deprecation in all 33 paths". Both of these constraints were explicitly added to the code you just provided (`verify-evidence-index.py` strictly checks `residue.get("tested_commit") != TESTED_COMMIT`, and `verify-command-parity.sh` loops all 33 paths with `grep -cF ... == 1`). Their reviews demonstrate they did not read the updated indexer or parity loop.

The consensus to move to mechanical finalization is correct, but their reasoning is sloppy and their remaining demands are already satisfied.

### New Findings & Contradictions

The core constraints—absolute parity, complete path isolation, cryptographic binding of logs, and Homebrew tap transitions—are all rigorously proven. You successfully resolved the catch-22 matrix validation by properly configuring the Python indexer to evaluate the `final` state machine.

However, the validation harness still contains minor mechanical gaps:

**1. LOW: Glob Expansion Risk in Parity Check**
In `verify-command-parity.sh`, the command path is tokenized using `read -r -a path_args <<<"$suffix"`. Because `set -f` (noglob) is not active, if any current or future Clap subcommand alias contained glob characters (`*`, `?`), bash would silently attempt to expand it against the temporary directory contents before invoking the `$canonical` binary. While the current 33 paths are safe, a validation harness should never rely on passive glob safety.

**2. LOW: Incomplete Exception Binding**
In `verify-evidence-index.py`, you check the presence of the signed waivers:
```python
if exceptions.get("tested_commit") != TESTED_COMMIT or not exceptions.get("exceptions"):
    fail("disclosed exceptions lack recorded disposition")
```
This prevents someone from wiping `exceptions.json` to hide the disclosure, which is an excellent fail-closed design. However, it does not assert the *exact* exception IDs. If an operator manually removes the `stripped-path-advisories` exception but leaves the `exploratory-legacy-dispatch-log-write` exception, the array length is still > 0 and the script will blindly pass. The indexer must statically assert that both specific IDs are present and signed.

**3. LOW: Redundant Archive Assertions**
In `verify-install-channels.sh`, you validate the absence of the `orchestratectl` binary via an embedded Python script (`assert not any(p.name == "orchestratectl" for p in root.rglob("*"))`), and then immediately repeat the exact same check in bash (`! tar -tf "$archive" | grep -Eq '(^|/)orchestratectl$'`). This is harmless but messy.

### Top 3 Remaining Issues

There are no remaining Blockers, High, or Medium risks. The evidence package is exceptionally tight. The top issues are purely hygiene:

1.  **Exception Signature Verification:** `verify-evidence-index.py` must assert the exact presence of the two specific exception IDs, not just a non-empty list.
2.  **Glob Expansion Hazard:** Add `set -f` before `read -r -a path_args` in `verify-command-parity.sh` to guarantee precise argv passthrough.
3.  **Mechanical Finalization:** Execute the final DAG regeneration, complete the residue and assessment checks, update the acceptance matrix, and run `--write` to authorize R9.

### Verdict

**PASS AFTER MECHANICAL FINALIZATION**

The R8 integrated validation evidence is cryptographically sound, properly handles the Homebrew cross-tap migration locally, accurately maps the bounds of the legacy Cargo wrapper, and correctly utilizes the Python indexer to block R9 authorization until the final residue asserts zero zombie processes or untracked paths. Proceed to finalization.

---

## Model: gpt-5.6-sol
[model:gpt-5.6-sol] [thread_id:api_cfe072ccea804d2e961b7097350754bb]

# Round 2 assessment

## What is genuinely blocking

### HIGH — Passing evidence is attached to superseded harnesses

The caller explicitly says:

> Cargo installs now use `env -i` ... **corrected rerun pending**

Yet both machine authority inputs already say the leg passed:

- `evidence/command-manifest.json`, `install-channels.result`: `"pass"`
- `evidence/acceptance-matrix.json`, disposable channel criterion: `"pass"`

The attached `logs/install-channels.log` came from the earlier harness execution. It cannot support the newly modified `verify-install-channels.sh`. The same source commit does not make a result transferable across evidence-harness changes.

This is a direct contradiction, not mechanical finalization. The corrected Cargo leg must run successfully, and then all affected records and hashes must be regenerated. Until then, the manifest and matrix must not describe the corrected harness as passing.

The same rule applies to other post-run harness changes:

- `verify-command-parity.sh` added all-33-path invocation and exact-once warning assertions.
- `verify-homebrew-prelive.sh` added the post-`brew update` Homebrew commit assertion.
- `verify-evidence-index.py` changed final authorization semantics.

The verifier itself need not rerun a product leg, but parity and Homebrew must be rerun if their current success logs predate those assertions. A later assertion cannot retroactively strengthen an earlier execution.

**Required outcome before authorization:**

```text
corrected install-channel run = pass
corrected parity run = pass
corrected Homebrew run = pass if its log predates the post-update assertion
```

Then update the logs, summaries, manifest, matrix, and index together.

---

### HIGH — The evidence still does not satisfy the ADR’s literal all-command parity requirement

`verify-command-parity.sh` now:

1. compares the complete structured command tree;
2. drives all 33 public paths into parser errors with `--r8-invalid`;
3. compares seven valid state-independent commands.

That is good parser and surface coverage. It is not:

> Every public command under canonical and wrapper names has identical stdout, JSON/JSONL, and exit codes.

For the remaining commands, the harness proves only that both entry points select an equivalent parser path when given a deliberately invalid flag. It never reaches command execution. It therefore does not cover invocation-identity divergence inside handlers for commands such as:

- `state migrate`
- `state rollback`
- `run create`
- `run merge`
- `run reattach`
- `run wait`
- `node report`
- `event create`
- `skill install`
- `supervise`

The shared dispatcher lowers the risk but does not eliminate it. Invocation identity is deliberately passed into the implementation for branding and warning behavior, so handler-level divergence is structurally possible.

The acceptance matrix accurately calls this “every public command surface,” but the ADR requires command behavior. That is a weaker criterion.

Resolve this in one of two ways:

1. Generate a safe success/refusal/dry-run fixture for every leaf command and compare stdout, filtered stderr, and exit status under both entry points; or
2. Record an explicit authorized interpretation that full structural parity, all-path parser parity, representative valid output parity, and one shared dispatcher satisfy R8’s “every public command” requirement.

Without one of those, “every required R8 leg passed” remains overstated.

---

### MEDIUM — The sanitizer is a path replacer, not a sanitization or secret-verification gate

`sanitize-evidence.py` replaces a small set of path patterns:

```python
<USER_PATH>...
<TMP_PATH>
<TMP_PATH>
<TMP_PATH>
```

It does not detect or reject:

- GitHub tokens;
- crates.io tokens;
- `Authorization` headers;
- credential-helper output;
- arbitrary user home paths;
- email addresses;
- runner names or labels;
- secrets encoded outside those path patterns;
- environment dumps.

It also uses:

```python
source.read_text(errors="replace")
```

which silently replaces undecodable bytes. That means the committed sanitized log is not necessarily a lossless transform of the original output, and no source-log digest or transformation receipt is shown.

Calling this file a sanitizer overstates its guarantees. At finalization, run a fail-closed secret/privacy scan over every indexed artifact and retain the result. At minimum, reject known token patterns and remaining absolute private paths.

This is mechanical only if the scan passes. If it finds sensitive data, remediation and index regeneration are required.

# Mechanical finalization items

These are not current product/evidence failures, provided the generated files pass the strict verifier.

## Final residue

The following intentionally remain pending:

- `command-manifest.json`: `diff-residue`
- `acceptance-matrix.json`: exact source/no-production-change criterion
- `residue.json`

The updated verifier correctly requires:

- matching tested commit;
- no production diff;
- no unexpected candidate processes;
- no unexpected tracked paths;
- no public or tag mutation;
- no unrelated-worktree mutation.

That is adequate if the generated residue file is based on actual commands and not manually asserted.

## Final assessment

The review criterion and assessment files are intentionally pending. The verifier now requires:

- matching tested commit;
- four models;
- findings;
- overall pass.

The check remains shallow—it does not require unique model identities or verify the finding dispositions—but it is sufficient for this bounded workflow if the human-readable assessment is indexed and reviewed.

## DAG refresh

The stale obsolete Homebrew issue in `issue-dag.json` must be refreshed after its disposition. This is mechanical and does not invalidate the successful Homebrew simulation.

## Hash and index regeneration

Logs, summaries, matrix, assessment, residue, and harness files must be finalized before the index is written. A subsequent normal invocation of `verify-evidence-index.py` must pass without rewriting anything.

# Weak consensus that should be challenged

## “All command parity is resolved”

It is not resolved literally. All 33 command **paths** are reached, but only through parser rejection. Reviewers treating that as exhaustive command behavior are collapsing parser parity into execution parity.

This may be acceptable as an explicitly bounded R8 interpretation because the wrapper links the shared dispatcher. It is not equivalent to the ADR’s stated verification requirement unless that interpretation is authorized.

## “The exploratory user-log incident is irrelevant once disclosed”

It is not irrelevant. The run violated the literal condition that every mutation destination be sandboxed. The record now handles it honestly:

- no false claim of provable byte restoration;
- mtime mutation acknowledged;
- attempts excluded from acceptance;
- clean authoritative replacements required;
- task-owner disposition recorded in `exceptions.json`.

Given that explicit disposition, it need not block R8. But final prose must limit the sandbox guarantee to **authoritative gate executions**. It cannot truthfully claim that every command executed during R8 development was sandboxed.

## “The retained `xcrun` warning proves clean-PATH failure”

That dismissal is also wrong. The warning is unexplained and should be fixed in R10, but this R8 run:

- compiled all release test binaries;
- ran all 1,115 tests;
- exited zero;
- used an isolated environment;
- had an ordinary exact-SHA build without the SDK warning.

The warning does not invalidate the clean-PATH behavioral test. It is correctly a disclosed advisory, not a blocker.

## “The moving LEAK marker is a product defect”

The evidence does not support that claim. Attribution moved between process-free unit tests, all assertions passed, nextest exited zero, and the ordinary run had no marker. Final no-process residue remains mandatory. `pass-with-disclosed-warning` is defensible.

# New contradictions and defects

## MEDIUM — Validation prose is stale about compatibility Cargo installation

`validation.md`, “Packages and install channels,” says:

> Disposable Cargo-prefix, raw-archive, and locally redirected generated shell installer checks install/run only `taskfleet`

The current `verify-install-channels.sh` also installs and runs the bounded `orchestratectl` Cargo wrapper.

The intended statement should distinguish channels:

- canonical Cargo/archive/shell artifacts install only `taskfleet`;
- the separate compatibility Cargo package installs only `orchestratectl`.

As written, the report contradicts the harness.

---

## MEDIUM — The parity manifest and matrix omit the strongest new check

`command-manifest.json` still describes parity as:

> 33 public structured-help command surfaces, 7 ordinary ... commands

`acceptance-matrix.json` likewise mentions structural parity and seven ordinary outputs, but not that all 33 paths are now invoked under both names with forced invalid input.

That omission matters because the all-path loop is the main evidence narrowing the gap between surface parity and behavioral parity. Update both records after the corrected rerun.

---

## MEDIUM — Final authorization does not verify that the index’s `overall` agrees with the manifest

`verify-evidence-index.py` checks:

```python
if index.get("r9_authorized") != (manifest.get("overall") == "pass"):
```

but it does not check:

```python
index["overall"] == manifest["overall"]
```

A malformed index could contain:

```json
{
  "overall": "pending-review-and-residue",
  "r9_authorized": true
}
```

and still pass if the manifest is `"pass"`.

`r9_authorized` is the operative field, so this is not a release-safety blocker, but the machine authority should be internally consistent:

```python
expected_overall = (
    "pass" if manifest.get("overall") == "pass"
    else "pending-review-and-residue"
)
if index.get("overall") != expected_overall:
    fail("index overall disagrees with manifest")
```

---

## LOW — The final assessment schema allows duplicate or unnamed models

The verifier only requires:

```python
len(assessment.get("models", [])) == 4
```

It accepts four duplicate entries or empty model records. If multi-model review is a formal acceptance criterion, require unique nonempty model identifiers and per-model verdicts.

---

## LOW — Homebrew’s public clone still inherits ambient Git configuration

This remains:

```bash
git clone -q https://github.com/jarimustonen/homebrew-orchestratectl.git
```

outside `env -i`. It may use ambient credential helpers, proxy settings, or URL rewrites. It is a public read and cannot mutate the remote, so it does not block R8. R10 should use a private Git HOME and disabled credential helper.

# Final top 3

1. **HIGH:** Corrected channel/parity/Homebrew harnesses cannot inherit pass results from earlier executions. At least the Cargo corrected rerun is explicitly pending.
2. **HIGH:** All-path invalid-input parity is not the ADR’s literal valid execution parity for every public command. It needs broader fixtures or an explicit accepted interpretation.
3. **MEDIUM:** `sanitize-evidence.py` does not establish that indexed logs are secret-free; a fail-closed final artifact scan is still needed.

# Verdict

**FIXABLE EVIDENCE GAPS**

This is not ready for “PASS after mechanical finalization” yet because a corrected Cargo rerun is explicitly pending, and any other modified execution harness must also be rerun. Those are substantive evidence-generation steps, not field updates.

After the affected reruns pass, the parity interpretation is explicitly resolved, the artifact scan passes, and the intentionally pending residue/review/DAG/index files are generated and verified, R8 can become a pass authorizing only R9. The LEAK marker, retained `xcrun` warning, and disclosed exploratory log incident do not independently block that outcome.

---

## Model: claude-fable-5
[model:claude-fable-5] [thread_id:api_274dbce5f90846a48b591390383a45c1]

# Final round 2 — R8 evidence review

## 1. Blocking vs. mechanical

**Genuinely blocking (must complete/pass before `--write`, but all are already-scheduled work):**

- **B1. The env-i Cargo install rerun is pending.** The corrected `verify-install-channels.sh` (now `env -i` with private HOME/CARGO_HOME/target, RUSTUP_HOME passthrough) has **not yet produced an authoritative pass**. The committed `logs/install-channels.log` was generated by the previous harness revision. Until the rerun lands and the log is replaced, the `install-channels` row's `pass` refers to a superseded harness — precisely the class of stale-evidence coupling this run has repeatedly disciplined itself about. The `residue`/`evidence-review` legs are pending by design; this one is pending by *correction*, which is different.
- **B2. Residue leg must actually pass** with the now-strict schema (7 required falsy fields + commit binding). This is the load-bearing gate for the LEAK and legacy-log dispositions; everything conditional ("contingent on final no-process residue") resolves here.

**Mechanical finalization only:**
- Regenerate `issue-dag.json` after dispositioning `nominally-numberless-hand`; rehash `outputs-summary.txt` / acceptance matrix; produce `review.md`/`assessment.json`/`assessment.md`; flip manifest `overall`; run `--write`. The verifier now enforces all of it fail-closed, commit-bound, with exception records. None of these can silently mint authorization anymore.

## 2. Challenging consensus and dismissals

**W1. The consensus over-credits the residue gate's process check.** Everyone (including my prior round) treats "final residue finds no candidate process" as closing the LEAK disposition. Be precise about what it proves: a point-in-time process scan **after** the run cannot retro-confirm that the LEAK marker in the stripped run was benign — a leaked child that exited hours ago leaves no residue. The actual justification for benignity is the disposition's *attribution-mobility* argument (marker moved between two process-free tests) plus the ordinary run's clean marker. That argument is sound; the residue check is corroboration, not proof. `exceptions.json` phrasing "contingent on final no-process residue" slightly overstates what residue can establish. Accept it, but don't pretend the residue check is the evidentiary core.

**W2. OpenAI's "Homebrew identity refresh" ask was already stale when made** — `toolchain.json` and `homebrew-acceptance.json` both carried `6.0.21-70-g2316567`/`2316567ba9...` and the verifier cross-checks them. Consensus should have marked this closed rather than re-listing it; re-listing satisfied asks inflates the apparent open-issue count and dilutes attention from B1.

**W3. DeepSeek's exact-once deprecation ask, now landed, has a subtle scope limit nobody flagged.** The 33-path loop's exact-one assertion runs against `--r8-invalid` error paths only. A command that emits the deprecation warning *twice* only on its **success** path would pass: the byte-compared ordinary set covers just 7 commands' success paths. The `compatibility.rs` suite plausibly covers "at most once per process" generically, but that claim is asserted in `validation.md`, not demonstrated in the attached evidence. Not blocking (the wrapper is a single dispatch shim; a per-command double-warn is architecturally implausible), but the consensus wording "exactly one warning each" claims more than the harness tests.

**W4. Correct dismissal to reaffirm:** the Ruby Symbol claim stays dead. Quoted-label hash keys are Symbols; the formula executed end-to-end in the drill. No re-litigation warranted.

**W5. One consensus gap: `REQUIRED_RESULTS` now hard-pins historical warning states.** `stripped-path` *must* be `pass-with-disclosed-warning`; a hypothetical clean rerun recording `pass` fails verification. As a frozen-record pin this is defensible, but no one documented that it is intentional. One comment line; do it at finalization.

## 3. New evidence contradictions

**C1. Verifier/manifest output-reference mismatch — will fail final validation as written.** The final-mode check resolves every manifest `output` pattern via `evidence.glob(pattern)`. But manifest rows reference `distribution-artifact-hashes.txt` (exists in `evidence/` — fine) **and** the `install-channels` row references `logs/install-channels.log`, which must be the *rerun's* log (B1). More importantly: `evidence-review` row lists `review.md, assessment.json, assessment.md` and `diff-residue` lists `residue.json` — these must be created **inside `evidence/`** or the glob fails. Planned, but note the coupling: if `review.md` is filed at issue root (a natural place), final `--write` hard-fails. Put all four under `evidence/`.

**C2. Indexed-artifact set vs. `artifact_paths()` drift risk.** `artifact_paths()` hardcodes six root-level files. `verify-command-parity.sh` writes its inventory to `${R8_PARITY_INVENTORY:-$tmp/...}` — the committed `public-command-inventory.json` in `evidence/` is only regenerated if the env var was set on the authoritative run. If the 33-path-loop harness revision changed the inventory shape/count and the rerun didn't export `R8_PARITY_INVENTORY`, the committed inventory silently describes an older harness output. Same class as B1: confirm the committed inventory came from the final harness revision at rehash time.

**C3. Sanitizer path-boundary nit (LOW).** `sanitize-evidence.py`'s `<USER_PATH>(?:/...)*` rule replaces the bare home dir but the repo-path rule runs *first* with `Path.cwd()` — correct only if the sanitizer is always invoked from the repo root. Invoked from elsewhere, repo paths degrade to `<USER_PATH>` remnants with trailing components intact... actually no: the `<USER_PATH>(?:/[^\s:'\"`]+)*` rule would swallow the whole path to `<USER_PATH>`, losing the `<REPO>` distinction but leaking nothing. Cosmetic inconsistency risk only; not a leak vector. No action required beyond invoking from repo root.

No contradictions found between committed logs, JSON evidence, and prose in this round. The Homebrew identity triple (script pin, toolchain, acceptance) is internally consistent; the diagnostics file now explains the earlier `-52-g27d05ae` discrepancy as superseded stale metadata.

## 4. Final top 3 and verdict

1. **B1 — land the corrected env-i install-channels rerun and replace its log before rehash.** The only leg whose current `pass` predates its current harness. Same discipline applies to `public-command-inventory.json` (C2).
2. **C1 — file `residue.json`, `review.md`, `assessment.json`, `assessment.md` under `evidence/`,** or final-mode output-reference validation self-fails. Trivial but sequencing-critical.
3. **B2 + W1 — complete the residue leg and, in the assessment, state the LEAK-benignity rationale as attribution-mobility corroborated (not proven) by residue.** One sentence of accuracy in the permanent record.

**Verdict: PASS after mechanical finalization.** The authorization machinery is now genuinely fail-closed: commit-bound residue/assessment schemas, per-command expected results, exception records with owner authority, indexed harness scripts and sanitizer, cross-checked Homebrew identity, and a verifier that works in both write and steady-state verify modes. No product defect was demonstrated at any point in three rounds; every finding was evidence-harness quality. Remaining work is executing the already-corrected reruns and the intentionally-deferred final legs in the right order: install-channels rerun → inventory/DAG/matrix regeneration → residue → review/assessment (all under `evidence/`) → manifest flip → `--write`. Scope unchanged: authorizes R9 source-repository rename only; expires on repository identity change; nothing for R10/R11.

---

## Model: deepseek-v4-pro
[model:deepseek-v4-pro] [thread_id:api_f0ee66622ad04a769762776caaaf2818]

## Verdict

**FIXABLE EVIDENCE GAPS — do not grant R9 authorization yet.** The finalizer and channel scripts are much stronger, but one side-effect isolation problem remains, and the final evidence bundle is still pending. Those are fixable, but they are not all mechanical.

---

## 1. Genuinely blocking vs. mechanical finalization

### Genuine residual gap: host `RUSTUP_HOME` and ambient PATH in Cargo channel checks

File: `verify-install-channels.sh`

```bash
tool_path="$PATH"
rustup_home="${RUSTUP_HOME:-$HOME/.rustup}"
...
env -i HOME="$tmp/home" PATH="$tool_path" CARGO_HOME="$tmp/cargo-home" RUSTUP_HOME="$rustup_home" \
  CARGO_TARGET_DIR="$tmp/cargo-target" cargo install --locked ...
```

`HOME`, `CARGO_HOME`, and `CARGO_TARGET_DIR` are private, but `RUSTUP_HOME` is copied from the host environment. `cargo install` through the rustup proxy can read that directory and can plausibly write proxy/toolchain state. This violates the R8 requirement that every mutation destination be sandboxed unless a no-mutation proof exists.

The finalizer does not close this gap. `verify-evidence-index.py` requires residue fields:

```python
required_residue = {
    ...,
    "unexpected_tracked_paths",
    "public_mutation",
    "tag_mutation",
    "unrelated_worktree_touched",
}
```

None of those checks host `$HOME/.rustup`, `$CARGO_HOME`, or `$TMPDIR` mutation. A host rustup write would not fail finalization.

Required correction:

- run the corrected Cargo channel with private `RUSTUP_HOME` and a bounded toolchain, or
- add a residue check recording host `$HOME/.rustup` mtimes/content hashes before and after, and verify no change.

The corrected rerun is still listed as pending. This is a real fix, not mere evidence regeneration.

---

### Mechanical but mandatory before final PASS

**Final review and residue artifacts are still absent by design.**

`command-manifest.json`:

```json
{"id": "diff-residue", "result": "pending", "output": "residue.json"},
{"id": "evidence-review", "result": "pending", "output": "review.md, assessment.json, assessment.md"}
```

`acceptance-matrix.json`:

```json
{"criterion": "exact source identity ...", "result": "pending-final-residue"},
{"criterion": "multi-model review ...", "result": "pending"}
```

These are mechanical, but the finalizer must be run after they are emitted and updated. The verifier now fail-closes on missing or non-passing results, so this should work.

**Stale DAG must be regenerated.**

The earlier hashed `issue-dag.json` contained:

```json
{
  "slug": "nominally-numberless-hand",
  "status": "untriaged",
  "title": "Old Homebrew receipt remains owned by legacy tap after migration"
}
```

while `homebrew-acceptance.json` asserts the migrated receipt source moved to `jarimustonen/taskfleet`. That contradiction must not remain in the final indexed DAG. Regenerating the DAG is mechanical, but it must happen before final index generation.

**Stale stress-test wording.** The source-ignored `flock_stress_50_threads_1000_iters` is still described as “not a required executed leg”; this is now accurate. No further action.

---

## 2. Weak consensus and incorrect dismissals

### The Ruby Block argument was correctly rejected

OpenAI’s claim that:

```ruby
BINARY_ALIASES = {
  "aarch64-apple-darwin": {}
}
```

produces a String key is wrong. That syntax creates a Symbol key, so:

```ruby
BINARY_ALIASES[target_triple.to_sym]
```

is correct. The executed Homebrew log confirms the formula installed and upgraded. No issue.

### The Cargo “private roots” consensus is too permissive

Anthropic, Gemini, and OpenAI broadly accepted that Cargo installs now use private homes and targets. The current script does use private `HOME`, `CARGO_HOME`, and target, but it still passes host `RUSTUP_HOME` and the full ambient `PATH`. That is not a fully isolated install environment. The consensus missed this because the corrected script was not examined closely enough.

### The parity loop now lives up to its claim

`verify-command-parity.sh` now asserts exactly once for the deprecation warning in every one of the 33 command-path invalid invocations:

```bash
[[ "$(grep -cF '`orchestratectl` is deprecated' "$tmp/all-compat.err" || true)" == 1 ]]
```

and the normalization is bounded to the `command` object field. This addresses the earlier criticism. The other reviewers were right to accept this after the correction.

---

## 3. New evidence contradictions

No new direct contradiction beyond the already-known stale DAG issue. The main new problem is not a contradiction but an unproven isolation boundary in the install channel evidence.

`verify-evidence-index.py` also still does not check `acceptance-matrix.json` criterion text against the actual command result; it only checks that each criterion result is `"pass"`. That is acceptable fail-closed if the manifest is correct, but it means the final matrix must be manually updated truthfully.

---

## Top 3 remaining issues

1. **Host `RUSTUP_HOME` and ambient PATH in `verify-install-channels.sh` are not sandboxed.** Use a private `RUSTUP_HOME`/toolchain copy or record host rustup state before/after and include that in `residue.json`. Current finalizer will not catch host rustup mutation.

2. **Final residue/review/assessment/index/DAG artifacts are still pending.** They must be emitted under `evidence/`, satisfy the new fail-closed schema, and be rehashed only after the Homebrew/DAG inconsistency is removed.

3. **The stale `issue-dag.json` contradiction must be resolved before final index generation.** If finalization happens on the current DAG, the indexed evidence would contain an open issue claiming the Homebrew receipt migration failed while `homebrew-acceptance.json` says it passed.

---

Bottom line: the evidence harness is now mostly sound, the finalizer is fail-closed, and the assessed warnings are honestly disclosed. But the install-channel sandbox is still incomplete, and the final bundle is not closed. Fix those two concrete things and run finalization strictly.
