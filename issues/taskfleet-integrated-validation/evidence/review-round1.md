[thread_id:group_d96892c7efec458baa463b2e6120bc7b]

## Model: gemini-3.1-pro-preview
[model:gemini-3.1-pro-preview] [thread_id:api_22a5a63de6c64ce68d8720380af53f29]

### Problems Found

**BLOCKER: Hand-Forged Release Artifacts Violate ADR**
`verify-homebrew-prelive.sh` manually rewrites the generated Homebrew formula using Python (`s.replace('version "0.5.1"', 'version "0.6.0"')`). ADR 0002 explicitly mandates: *"Regenerate its workflow; do not hand-edit generated release identity."* You are not testing the actual 0.6.0 `cargo-dist` output; you are testing a hand-mangled 0.5.1 formula. This invalidates the Homebrew integration test.

**BLOCKER: Untested Legacy Cargo Wrapper**
ADR 0002 states: *"`cargo install orchestratectl` receives the bounded same-dispatch wrapper through 0.7.x."* However, `verify-install-channels.sh` ONLY installs the canonical crate (`cargo install ... --path "$repo_root/crates/taskfleet"`). It completely fails to test the legacy wrapper crate (e.g., `crates/octl-cli`). The R8 evidence does not prove the deprecated Cargo channel successfully isolates the executable or shares dispatch.

*Solution:* Add the wrapper validation to `verify-install-channels.sh`:
```bash
cargo install --locked --path "$repo_root/crates/octl-cli" --root "$tmp/cargo-legacy-root" >/dev/null
[[ -x "$tmp/cargo-legacy-root/bin/orchestratectl" && ! -e "$tmp/cargo-legacy-root/bin/taskfleet" ]]
env -i HOME="$tmp/home" PATH=/usr/bin:/bin ORCHESTRATECTL_HOME="$tmp/legacy-state" \
  "$tmp/cargo-legacy-root/bin/orchestratectl" version --output json | jq -e '.data.commit == "c3ef8b740ac531f12ce81c759ed209d178cf36bd"' >/dev/null
```

**BLOCKER: Skipped State Quiescence Stress Test**
`outputs-summary.txt` explicitly records `1 skipped: taskfleet_core::stress_tests::flock_stress_50_threads_1000_iters`. The prose claims this was "exercised separately in R8", but `command-manifest.json` contains no entry proving it was executed. State migration under ADR 0002 relies fundamentally on file-lock quiescence. Skipping the `flock` stress test in the integrated validation and failing to log its separate execution means concurrency safety is unverified.

**HIGH: Dangerous "LEAK" Marker Ignored**
The isolated test runner flagged a delayed-exit `LEAK` on `schema::wire_names_match_serde_round_trip`. Dismissing this because "the ordinary full gate had no leak marker" is unsafe. A pure schema serialization test should have zero background threads, child processes, or lingering descriptors. If this test leaks, your Serde `Drop` implementations or internal allocations are likely leaking OS resources, which could directly violate the strict quiescence/lock guarantees required by the ADR migration step.

**HIGH: Platform Hardcoding**
Both `verify-install-channels.sh` and `verify-homebrew-prelive.sh` explicitly hardcode `$repo_root/target/distrib/taskfleet-aarch64-apple-darwin.tar.xz`. There is zero verification of Linux or x86_64 artifacts. You must loop over all generated distribution artifacts to prove none of them accidentally bundle an `orchestratectl` binary.

*Solution:*
```bash
for archive in "$repo_root"/target/distrib/taskfleet-*.tar.xz; do
    tmp_arch="$(mktemp -d)"
    tar -xJf "$archive" -C "$tmp_arch"
    [[ "$(find "$tmp_arch" -type f -perm -111 -exec basename {} \; | LC_ALL=C sort)" == taskfleet ]]
    rm -rf "$tmp_arch"
done
```

**HIGH: State Tampering and Sandbox Failure**
The validation report admits that script development caused direct writes to the user's legacy log, and the author manually deleted the JSONL lines to "repair" the state. This is a severe sandbox failure. True R8 isolation means no ambient state is ever reachable. Manually tampering with `~/.orchestratectl` to cover up test pollution violates the immutable event byte contract.

**MEDIUM: Untested Legacy Upgrade Alias**
In `verify-homebrew-prelive.sh`, you invoke `brew upgrade taskfleet`. You failed to test `brew upgrade orchestratectl` (or a bare `brew upgrade`), which is the exact command a legacy user will run. You must prove that Homebrew's tap migration natively maps the old target command requested by the user to the new canonical identity without duplicate ownership errors.

**MEDIUM: Incomplete Identity Binding**
`source-identity.json` pins the root `Cargo.toml` and `Cargo.lock`, but omits the `Cargo.toml` manifests of the individual crates (`taskfleet`, `taskfleet-core`, `octl-cli`). The actual package dependencies and definitions are not cryptographically bound to the evidence record.

### Questionable Decisions & Risks
- **Testing pre-release state with a 0.5.1 cargo-dist payload:** By running the validation while `cargo-dist` still thinks the version is 0.5.1, you are fundamentally incapable of validating the real release graph. The ADR requires the release pipeline to be proven. If you have to write Python scripts to forge the formula version, you are proving your shell script works, not cargo-dist. The source repository must bump versions locally in a disposable commit to generate real 0.6.0 artifacts for testing, or `cargo-dist` must be run with environment overrides to force the 0.6.0 payload.

### What is Solid
- **Homebrew cross-tap topology:** Relying on Homebrew's native `tap_migrations.json` instead of attempting unsafe local symlinks or duplicate keg ownership is the correct architectural choice.
- **Evidence structure:** The immutable index generator (`verify-evidence-index.py`) properly detects residue, maps full `sha256` checksums, and accurately suppresses `r9_authorized` when the manifest isn't formally passed.

### Verdict
**BLOCK R8**

You cannot proceed to the R9 repository rename. The evidence contains hand-forged artifacts, skips the critical file-lock stress test without proof of secondary execution, fails to test the legacy Cargo wrapper entirely, hardcodes macOS ARM64 at the expense of Linux, and ignored a critical resource leak in a pure schema test. Re-run the R8 suite with true cross-platform loops, real cargo-dist outputs, and full execution logging.

---

## Model: gpt-5.6-sol
[model:gpt-5.6-sol] [thread_id:api_cfe072ccea804d2e961b7097350754bb]

# Problems found

## BLOCKER

### 1. The evidence package does not contain the command outputs it claims to make immutable

`evidence/command-manifest.json` generally points to `outputs-summary.txt`, which is a manually curated assertion summary, not command output. `outputs-summary.txt` then records hashes such as:

- `raw_sha256=3705...` for the Rust gate
- `stripped_authoritative_raw_sha256=5fd5...`
- `authoritative_raw_sha256=6144...` for Homebrew

The corresponding raw bytes are not indexed or attached. A digest of unavailable data is not verifiable immutable evidence. It cannot establish:

- the actual command and environment;
- exit status;
- complete test names and skipped tests;
- whether output was truncated or selectively summarized;
- whether warnings other than the disclosed ones occurred;
- whether the hash was computed from the claimed command output.

This directly falls short of the issue acceptance requirement to record “immutable command outputs, hashes” and prevents an adversarial review of most claimed passes.

**Required correction:** commit sanitized raw logs, or deterministic machine summaries produced directly from those logs, together with:

```json
{
  "command_id": "rust-nextest",
  "started_at": "...",
  "finished_at": "...",
  "exit_code": 0,
  "stdout_sha256": "...",
  "stderr_sha256": "...",
  "combined_log": "...",
  "sanitization": {
    "tool": "...",
    "rules_sha256": "...",
    "pre_sanitization_sha256": "..."
  }
}
```

If raw logs cannot be committed, their storage location and immutable artifact identifier must be recorded. Bare hashes are insufficient.

---

### 2. `verify-evidence-index.py` can authorize R9 without validating the evidence state

In `verify-evidence-index.py`, authorization is derived solely from:

```python
passed = manifest.get("overall") == "pass"
...
"r9_authorized": passed
```

The script does not verify that:

- every required command ID exists;
- no command remains `pending`;
- every command result is an allowed terminal result;
- `diff-residue` passed;
- review and assessment passed;
- referenced output files exist;
- the output files actually support their command;
- source identity and CI files agree;
- the release authorization remains false under all valid states.

A manual edit changing only `command-manifest.json.overall` to `pass` would cause the machine authority to set `r9_authorized: true`, even if individual commands still said `pending` or `fail`.

This is not adequate as the declared “machine-readable authority.”

**Required correction:** make finalization fail closed. At minimum:

```python
required = {
    "ci-api": {"pass"},
    "rust-fmt": {"pass"},
    # ...
    "release-activation": {"expected-refusal"},
    "diff-residue": {"pass"},
    "evidence-review": {"pass"},
}

commands = {row["id"]: row for row in manifest["commands"]}
assert set(required) <= set(commands)

for command_id, allowed in required.items():
    assert commands[command_id]["result"] in allowed, command_id

assert all(row["result"] not in {"pending", "fail"} for row in commands.values())
assert manifest["overall"] == "pass"
```

It should also validate the assessment verdict and residue schema rather than trusting their presence.

---

### 3. The executed evidence scripts are not covered by the evidence index or the tested source identity

The command manifest relies on scripts including:

- `verify-command-parity.sh`
- `verify-install-channels.sh`
- `verify-homebrew-prelive.sh`

The index covers files only below `evidence/`. The attached install and Homebrew scripts live outside that directory and are not listed in `index.json`. `source-identity.json` binds production source to the tested commit but explicitly permits later issue/evidence changes.

Consequently, neither the exact tested source commit nor the evidence index identifies the exact script bytes that produced the results. Those scripts could change after execution without invalidating the index.

The problem is especially serious for `verify-command-parity.sh`, which is not attached at all despite supporting one of the ADR’s central compatibility claims.

**Required correction:** add every executing script, sanitizer, fixture verifier, patch, and configuration file to the index, including its SHA-256. Also record the final evidence commit SHA. The authority must bind both:

1. production tree: `c3ef8b...`;
2. evidence harness tree/commit: the final committed evidence SHA.

---

### 4. Several mandatory ADR acceptance legs are asserted but not demonstrated

The ADR requires evidence for:

- every public command under both names;
- terminal, active, pending, and unknown 0.5.1 state;
- config and provenance adoption;
- exact preservation of persisted bytes and identifiers.

The supplied evidence only says:

- seven “ordinary” parity commands were checked;
- completed, non-terminal, and pending-merge cases were covered;
- broad test modules were included in a 1,115-test run.

There is no attached command inventory showing that seven commands cover every public command. There is also no explicit “unknown” fixture/result in `validation.md`, `command-manifest.json`, or `outputs-summary.txt`.

A broad test-suite pass cannot prove a particular mandatory scenario ran unless the exact test list and results are retained.

**Required correction:**

- Commit the parity script and a generated inventory of all public subcommands/output modes.
- Map every public command to its old/new comparison result.
- Add a requirement matrix mapping each ADR verification item to exact test names and output records.
- Explicitly identify the “unknown” state/outcome fixture and its assertions. If it did not run, rerun that leg.

Until that mapping exists, “every required R8 leg passed” is unsupported.

---

## HIGH

### 5. Homebrew stale-link assertions incorrectly permit broken `orchestratectl` symlinks

`verify-homebrew-prelive.sh` uses:

```bash
[[ -x "$tmp/prefix/bin/taskfleet" && ! -e "$tmp/prefix/bin/orchestratectl" ]]
```

For a broken symbolic link, `test -e` returns false. Therefore this assertion passes even if a stale `orchestratectl` symlink remains—the exact condition the report claims was excluded.

The same defect appears in `verify-install-channels.sh` for Cargo and shell installations. Its archive check only finds executable regular files:

```bash
find "$tmp/archive" -type f -perm -111
```

That does not detect an `orchestratectl` symlink or other unexpected archive member.

The claim in `outputs-summary.txt` that stale-link removal passed therefore outruns the script.

**Required correction and rerun:**

```bash
assert_absent() {
  [[ ! -e "$1" && ! -L "$1" ]]
}

assert_absent "$tmp/prefix/bin/orchestratectl"
```

For archives, inspect every member and link target, preferably using `tar -tvf` plus an allowlist. Also validate that no absolute or escaping symlink exists.

---

### 6. The Homebrew test does not actually pin Homebrew 6.0.21

The script accepts:

```bash
[[ "$(brew --version | head -1)" == Homebrew\ 6.0.21* ]]
```

The evidence records `6.0.21-52-g27d05ae`, which means behavior from commits after the 6.0.21 tag may be in use. The wildcard would also accept arbitrary suffixes.

Given that this test specifically depends on Homebrew migration behavior, exact tool identity matters.

**Required correction:** pin and record the Homebrew Git commit and verify it with:

```bash
test "$(git -C "$(brew --repository)" rev-parse HEAD)" = "$EXPECTED_HOMEBREW_SHA"
```

If `6.0.21-52-g27d05ae` is intentionally authoritative, state that exact commit instead of describing the test as Homebrew 6.0.21.

---

### 7. The Homebrew cache is shared, persistent, and outside the run-specific sandbox

`verify-homebrew-prelive.sh` uses:

```bash
mkdir -p ... <TMP_PATH>
...
HOMEBREW_CACHE=<TMP_PATH>
```

This path is neither run-unique nor removed by the trap. It permits:

- contamination from previous attempts;
- concurrent-run collisions;
- reuse of stale or malicious cached downloads;
- mutations surviving the supposedly disposable run.

This contradicts the issue requirement that every mutation destination be sandboxed.

**Required correction:** use `"$tmp/cache"` and remove it with the rest of the run. Hash downloaded legacy artifacts before installation and bind them to the known 0.5.1 fixture hashes.

---

### 8. The run made real writes to the user’s legacy state/log location

`validation.md`, “Limitations and residue,” discloses that four probes appended JSONL lines to the user’s legacy log. Removing the lines restored content bytes but not filesystem metadata, and the destination was not sandboxed.

This is an actual violation of the acceptance statement that “every mutation destination is sandboxed.” Classifying those probes as setup diagnostics does not make the mutation disappear. It also weakens the claim that the run performed no real-state mutation.

This cannot be repaired merely by changing the final pending markers. It requires one of:

1. explicit authorized disposition that the incident does not invalidate R8, with the acceptance language narrowed to authoritative commands; or
2. a clean replacement validation run whose process begins with isolation already enforced.

At minimum, the report must not imply that all run activity was sandboxed.

---

### 9. The separate execution of the ignored stress test is not evidenced

`outputs-summary.txt` says the source-ignored flock stress test was “exercised separately in R8.” `command-manifest.json` says one expensive test was skipped and “is not a required executed leg,” but it has no separate command entry or output.

These statements are inconsistent:

- either it was separately exercised, in which case its command/result must be recorded;
- or it was not required, in which case the report must stop claiming it was exercised.

This is likely a localized evidence correction, but currently the full-suite description is misleading.

---

### 10. The Homebrew old-name resolution assertion checks only exit success

The script runs:

```bash
brew info --json=v2 jarimustonen/orchestratectl/orchestratectl >/dev/null
```

It does not inspect the returned JSON. Success alone does not prove the claim that the old fully qualified identity resolved to the intended canonical formula through the intended migration record.

**Required correction:** assert the canonical formula identity, tap, old-name mapping, and installed receipt from the JSON. Preserve that JSON as evidence.

---

### 11. Exact-SHA checks do not fully protect against stale artifact generation

The install scripts consume preexisting files from `target/distrib`, and Cargo installation reuses the repository’s `target` directory:

```bash
CARGO_TARGET_DIR="$repo_root/target"
```

Runtime `.data.commit == c3ef8b...` checks are useful, but they do not prove:

- which command produced each archive;
- that the formula was generated from the tested tree;
- that no stale nonbinary files entered the archive;
- that the formula/archive relationship was generated in the same authoritative execution.

The exact artifact hashes help only if the generation logs and manifest are available.

**Required correction:** record the clean artifact-generation command, exit result, source tree status, artifact manifest, and producer tool SHA. Prefer a fresh run-specific target directory for authoritative packaging.

---

### 12. The formula rewrite is not fail-closed

`verify-homebrew-prelive.sh` performs plain string replacement:

```python
s = s.replace('version "0.5.1"', 'version "0.6.0"')
s = s.replace(old, archive.resolve().as_uri())
```

It does not assert that either old string existed exactly once. A changed generated formula could leave one or both substitutions unapplied while the script continues.

**Required correction:**

```python
assert s.count('version "0.5.1"') == 1
assert s.count(old) == 1
```

Then assert the resulting formula contains the expected local URI, version, SHA-256, formula name, and installed binary.

---

## MEDIUM

### 13. The delayed-exit `LEAK` marker is not a test failure, but its dismissal is under-supported

A nextest delayed-exit marker does not automatically mean an orphan process survived; therefore it need not block solely under a no-test-failure rule. The final process-residue check, if it passes, is relevant.

However:

- the raw nextest output is unavailable;
- the exact leak timeout and nextest version/config are not shown;
- there is no targeted repeat count for the affected test;
- “pure schema round-trip test” does not explain why the test process exited late;
- absence of a surviving process after the whole suite does not distinguish delayed teardown from a transient leaked child/thread.

This should not be silently normalized as a clean pass. Preserve it as `pass-with-disclosed-warning`, run the affected test repeatedly under the same stripped environment, and capture nextest’s exact diagnostic. It becomes blocking only if the project’s gate explicitly treats nextest leak warnings as failures or repetition shows nondeterministic process behavior.

---

### 14. Side-effect isolation is not fully credential-isolated during setup

Several commands execute outside `env -i`, including:

```bash
git clone https://github.com/jarimustonen/homebrew-orchestratectl.git
brew --version
brew --repository
```

The public clone is read-only, but it can still consult ambient Git configuration, credential helpers, proxy settings, and tracing configuration. That does not establish the broader “credential-free” characterization.

Run all network setup with a dedicated HOME and disabled credential helper:

```bash
env -i \
  HOME="$tmp/git-home" \
  PATH=/usr/bin:/bin \
  GIT_CONFIG_NOSYSTEM=1 \
  git -c credential.helper= clone ...
```

Also record which commands were networked and which were guaranteed local-only.

---

### 15. The index protects evidence-directory bytes, not semantic cross-file consistency

`verify-evidence-index.py` verifies file sizes and SHA-256 values but does not cross-check:

- tested SHA across all files;
- CI run head SHA;
- artifact commit identity;
- Shipshape plan head;
- public-fact query timestamps;
- command output references;
- `release_authorized == false`;
- report result versus manifest/index result.

Add a semantic verifier. Hash integrity alone prevents unnoticed byte changes; it does not prevent internally contradictory evidence from being committed consistently.

---

### 16. No committed sanitization report is shown

The evidence includes a 538 KB `identity-occurrences.tsv` and multiple GitHub/toolchain/public-fact summaries. The report says paths and runner names were sanitized, but there is no evidence of a secret scan or deterministic sanitization procedure.

Before finalization, scan every indexed artifact for:

- GitHub tokens and authorization headers;
- registry tokens;
- credential-helper output;
- user names and absolute home paths;
- runner labels/names;
- temporary paths that were intended to be private.

Record the scanner command, rules/version, and result. Manual statements that content was sanitized are inadequate.

---

### 17. “Current fresh facts” need per-query timestamps and response identity

`validation.md` says facts were observed “at the recorded query times,” but the attached materials do not expose those underlying responses. Candidate-name 404s and repository/tap heads are time-sensitive and can change independently of the tested commit.

The machine evidence should include per-query:

- timestamp;
- URL;
- status;
- selected response body hash;
- authenticated versus unauthenticated mode;
- API version where relevant.

These facts are not reservations, as the report correctly states, but they still need traceable receipts.

---

## LOW

### 18. Report wording is prematurely stronger than the machine authority

`validation.md` says:

> **PASS, subject to the immutable evidence index and review below.**

Meanwhile:

- `index.json.overall` is `pending-review-and-residue`;
- `r9_authorized` is false;
- manifest residue and evidence review are pending.

The qualification makes the sequencing understandable, and the user explicitly said not to treat the pending markers themselves as defects. Still, the final report should derive its wording from the finalized machine state. Before finalization, “candidate pass pending review and residue” would be less ambiguous.

---

### 19. The issue-doctor result is not fully clean

`issue-doctor.json` has:

- `agents_md_drift: true`;
- an unknown `deliverable` key.

These are disclosed and appear unrelated to product behavior. They are not R8 blockers unless repository policy defines `issuectl doctor` warnings as fatal. The command manifest should describe this as a successful diagnostic with known findings, not as an unqualified clean issue gate.

# Questionable decisions and hidden assumptions

1. **Seven parity commands are assumed to represent the entire CLI.** No generated command inventory proves this.
2. **A post-tag Homebrew implementation is treated as Homebrew 6.0.21.** Migration semantics may differ from the referenced release.
3. **Runtime commit output is treated as complete artifact provenance.** It binds the binary’s embedded string, not every archive/formula member or generation step.
4. **Exploratory failures are considered outside acceptance.** That is reasonable for setup errors only if they made no prohibited external mutation. The user-log writes crossed that boundary.
5. **A final no-process check is assumed to explain a nextest leak warning.** It establishes eventual cleanup, not the cause or repeatability of delayed exit.
6. **A successful `brew info` is assumed to prove correct migration resolution.** Its returned identity was discarded.
7. **Git commit immutability is implicitly relied on for scripts, while the machine index claims authority.** The final evidence commit has not yet been identified, and the current index does not cover the harness.
8. **Shared caches are assumed harmless.** They weaken both isolation and reproducibility.

# Risks that could invalidate R9 authorization

R9 must remain unauthorized until all of the following are resolved:

- command outputs or equivalent immutable execution records are available;
- the authority generator validates individual required legs rather than trusting one aggregate string;
- the parity and mandatory-state coverage matrix proves all ADR cases, including “unknown”;
- Homebrew and install-channel checks are rerun with broken-symlink-safe assertions;
- Homebrew is pinned to an exact commit and uses a run-private cache;
- the exact evidence harness bytes and final evidence commit are bound;
- the external user-log mutation receives explicit disposition;
- final residue proves no production-tree difference, leaked process, unexpected worktree change, or public mutation;
- evidence artifacts pass a committed sanitization/secret scan.

# What is genuinely solid

- The tested production SHA and tree are consistently repeated across the supplied report, manifest, index, and source identity.
- Release authorization is explicitly false, and the scope is repeatedly limited to R9.
- Failed setup attempts are separated from acceptance evidence rather than being silently omitted.
- The Homebrew flow does install a real old 0.5.1 formula before introducing local migration metadata, which is the correct basic topology.
- Runtime commit checks materially improve binding for Cargo/archive/shell/Homebrew candidate binaries.
- The evidence index provides useful byte-level integrity for files currently under `evidence/`.
- The pending review/residue state is explicit rather than falsely finalized.
- The nextest leak marker is disclosed rather than hidden.

# Verdict

**FIXABLE EVIDENCE GAPS**

R8 must not authorize R9 in its current state. The gaps require targeted evidence-harness corrections and reruns, especially for Homebrew/link assertions and mandatory coverage. This is not eligible for “PASS after mechanical finalization”; changing the pending fields alone would create an unsafe authorization.

## Context request

- **Kind:** artifact
  **Need:** Sanitized raw logs, or immutable CI/artifact references, for every command in `command-manifest.json`, including exit codes and timestamps.
  **Why:** Could establish that the summarized passes and disclosed warnings faithfully represent complete outputs.

- **Kind:** artifact
  **Need:** `verify-command-parity.sh`, generated CLI command inventory, and its raw comparison results.
  **Why:** Could establish whether seven commands actually satisfy the ADR requirement for every public command.

- **Kind:** artifact
  **Need:** Exact nextest test list/result output and the source/test names covering terminal, active, pending, unknown, config, and provenance adoption.
  **Why:** Could resolve whether mandatory acceptance cases, particularly “unknown,” actually ran.

- **Kind:** artifact
  **Need:** `r8-ci-summary.json`, `r8-ci-jobs-summary.json`, the exact CI run URL/API response, and relevant job logs.
  **Why:** Could verify exact-SHA CI identity, all required job conclusions, and absence of undisclosed warnings/failures.

- **Kind:** artifact
  **Need:** `toolchain.json`, package/archive member manifests, distribution hashes, generated formula, old-tap migration patch/manifest, and Shipshape sealed plan.
  **Why:** Could verify tool pinning, exact package topology, artifact provenance, and migration semantics.

- **Kind:** artifact
  **Need:** Final `residue.json`, review assessment files, final evidence commit SHA, and clean `git status`/production diff/worktree/process checks.
  **Why:** These are required for final authorization but are appropriately pending at this review stage.

- **Kind:** clarification
  **Need:** Whether an authorized owner waives the acceptance violation caused by writes to the real legacy log, or requires a replacement clean run.
  **Why:** This determines whether the disclosed unsandboxed mutation can be accepted or independently blocks R8.

---

## Model: claude-fable-5
[model:claude-fable-5] [thread_id:api_274dbce5f90846a48b591390383a45c1]

# R8 Integrated Validation Evidence — Adversarial Review

## 1. Problems found

### HIGH

**H1. Prose verdict outruns the machine authority.**
`validation.md` § Result declares "**PASS**, subject to the immutable evidence index" and names `evidence/index.json` as "the machine-readable authority." That authority says `"overall": "pending-review-and-residue"`, `"r9_authorized": false`. `command-manifest.json` still has two `"result": "pending"` rows (`diff-residue`, `evidence-review`) whose declared outputs (`residue.json`, `review.md`, `assessment.json`, `assessment.md`) do not exist in `index.json.artifacts`. The pending state itself is expected at this review stage; the defect is that the human-readable report already asserts PASS while its own cited authority does not. The Result section must be conditional-phrased or the finalization order documented, otherwise a reader of `validation.md` alone gets an unauthorized conclusion.

**H2. Unexplained CI job-count reduction between R8 attempts.**
`item.md` (2026-09-02 run at `fa04841`) enumerates passing legs: Ubuntu + macOS Rust tests, fmt, clippy, docs, cargo-deny, MSRV, snapshots, bump-hook, release-wrapper — ~10 jobs plus the failing `version-snapshots` job. `validation.md` for run `33764612111` at `c3ef8b7` says "**all eight required jobs succeeded**." Either jobs were consolidated in a `ci.yml` change between commits, or gates (cargo-deny? MSRV? one OS leg?) were dropped. The `ci.yml` sha256 is pinned in `source-identity.json` but no job-name list or workflow diff is attached to this review. A silently narrowed CI gate would undermine the "exact-SHA CI green on the full gate" claim. Must be resolved from `r8-ci-jobs-summary.json`.

**H3. Real user-state mutation breached the sandboxing acceptance; restoration is unverifiable.**
`item.md` acceptance: "every mutation destination is sandboxed." `validation.md` § Limitations discloses four probes appended JSONL dispatch lines to the **user's real legacy log**, then the lines were removed by hand. Two problems: (a) the acceptance criterion was factually violated, not merely risked; (b) "byte-restored" is asserted with zero attached evidence — no pre/post hash of the log file appears in `index.json`, `fixture-hashes.txt`, or anywhere else. Hand-editing a live legacy append-only-adjacent artifact is exactly the class of real-state mutation R8 promised never to perform. Disclosure is correct behavior; the claim of perfect restoration currently rests on the worker's word. Attach before/after digests or an explicit statement that they were not captured (which would leave this permanently unverifiable and should be recorded as such).

**H4. The nextest LEAK marker is not adequately dispositioned.**
`outputs-summary.txt` / `command-manifest.json` (`stripped-path`): one delayed-exit LEAK attributed to `schema::wire_names_match_serde_round_trip`, absent in the ordinary run. Issues:
- A pure serde round-trip test has no plausible child process to leak. Either the test is not pure (product/test defect) or nextest's leak attribution landed on the wrong test, meaning the *actual* leaking test is unknown — a materially worse situation than disclosed.
- `item.md` `blocked_by` includes `@native-spawn-test-leaks`. That blocker was presumably resolved before R8 started; a fresh leak marker in a spawn-adjacent suite may indicate the fix regressed or was incomplete. The evidence makes **no linkage** to that issue.
- Under the run's own no-flake standard, a marker present in one of two identical runs *is* environment-dependent nondeterminism. "Disclosed rather than normalized" is honest but is not root-cause analysis.
This does not have to block R9 (repo rename does not depend on it), but the disposition must reference `@native-spawn-test-leaks` and either explain attribution or file a follow-up before `overall` flips to pass.

**H5. The authorization gate in `verify-evidence-index.py` is a single hand-edited string.**
`--write` computes `passed = manifest.get("overall") == "pass"` and sets `r9_authorized` from it. Nothing checks per-command results. An operator can flip `overall` to `"pass"` while `pending`/`fail` rows remain and the index will assert `r9_authorized: true`. Fix (localized, evidence-side):

```python
ALLOWED = {"pass", "pass-with-disclosed-warning", "pass-with-known-warnings", "expected-refusal"}
bad = [c["id"] for c in manifest["commands"] if c["result"] not in ALLOWED]
passed = manifest.get("overall") == "pass" and not bad
if bad and manifest.get("overall") == "pass":
    raise SystemExit(f"overall=pass but non-passing commands: {bad}")
```

### MEDIUM

**M1. Raw logs exist only as hashes.** `outputs-summary.txt` records seven `raw_sha256` values (full gate, stripped run, parity, Homebrew acceptance, etc.) but the referent logs are not in `evidence/` and their storage location is never stated. `item.md` acceptance is technically satisfied ("output hashes"), but the chain is unverifiable by any reviewer: a hash of a log you cannot retrieve proves nothing. State where the raw logs are retained, or admit they were discarded.

**M2. Homebrew version claim vs. tested version.** `verify-homebrew-prelive.sh` guards `Homebrew\ 6.0.21*`, and `homebrew-acceptance.json` records `"6.0.21-52-g27d05ae"` — a dev snapshot 52 commits past the release whose `formulary.rb`/`migrator.rb` the ADR cites. Migration semantics could differ from released 6.0.21. The glob would also accept `6.0.211`. Tighten the guard or record explicitly that a post-6.0.21 snapshot was accepted and why.

**M3. Side-effect leak in the Homebrew script.** `verify-homebrew-prelive.sh` uses `mkdir -p ... <TMP_PATH>` — a fixed, world-shared path **outside** the `$tmp` sandbox and **not removed** by the `trap`. This (a) leaves residue contradicting "entirely disposable," (b) allows cross-run cache contamination (a stale cached archive could be served to a later run), and (c) is a predictable-path risk on shared machines. Should have been `HOMEBREW_CACHE="$tmp/cache"`.

**M4. Parity coverage vs. ADR requirement.** ADR 0002 Verification #2 demands identical output for "**every public command**." Evidence shows 7 ordinary commands plus invalid/help/hidden checks (`command-manifest.json` `wrapper-parity`) plus `compatibility.rs`. Nothing attached demonstrates the parity set is derived from an exhaustive command inventory rather than a sample.

**M5. No acceptance-token traceability.** `item.md` requires "terminal/active/pending/**unknown**/config/provenance adoption." `validation.md` § Coverage maps terminal, active, pending-merge, config, provenance — "unknown" outcome adoption is never explicitly evidenced. A per-token mapping table from acceptance clause → test/artifact would close this cheaply.

**M6. Artifact bytes not bound to recorded hashes at use time.** `verify-install-channels.sh` and `verify-homebrew-prelive.sh` consume mutable `target/distrib/*` files without checking them against the committed `distribution-artifact-hashes.txt`. The embedded-commit assertion (`.data.commit == c3ef...`) partially compensates for the binary, but the formula/installer/checksum files used could differ from the hashed ones. One `sha256sum -c` line would bind them.

**M7. Single-architecture channel evidence.** All archive/shell/Homebrew checks use `taskfleet-aarch64-apple-darwin.tar.xz` only. Given the *previous* R8 blocker was a Linux-only fixture failure, the macOS-only scope of the local channel drills deserves an explicit limitation entry in `validation.md` (it is currently only inferable from filenames).

**M8. Index scope.** `index.json` covers only `evidence/` files. The verification scripts (`verify-homebrew-prelive.sh`, `verify-install-channels.sh`, `verify-command-parity.sh`, `verify-evidence-index.py`) and the evidence commit SHA itself are not pinned in the index. Git provides immutability, but the "committed evidence index" then depends on unstated commit identity; record the evidence commit SHA at finalization.

### LOW

- `verify-install-channels.sh`: `find -type f -perm -111` would miss a stray binary with only `u+x`, silently passing the "only taskfleet is executable" check for that file.
- `verify-evidence-index.py` uses bare `assert` for all verification — silently disabled under `python -O`. Use explicit raises.
- `CARGO_TARGET_DIR="$repo_root/target"` reuses the shared repo target dir during "disposable" install checks; fingerprints plus the embedded-commit check mitigate, but it is not the clean-room the script's comment implies.
- `brew info --json=v2 jarimustonen/orchestratectl/orchestratectl >/dev/null` asserts old-identity resolution by exit code only; no content check that resolution went via the migration metadata.
- The Python-edited candidate formula (version relabel + local URI) is itself unhashed evidence.
- `command-manifest.json` `ci-api` uses ellipsized command text (`.../jobs; .../artifacts`), reducing reproducibility of the exact API calls.

## 2. Questionable decisions / hidden assumptions

- **0.6.0-labelled formula wrapping a 0.5.1-reporting binary** (`verify-homebrew-prelive.sh`): defensible to force the upgrade path, and the check correctly pivots to `.data.commit`, but it means no evidence exercises version-consistency between formula, receipt, and binary — deferred entirely to R10, which should be stated.
- **Assumption that nextest leak attribution is trustworthy** (H4) while simultaneously claiming the attributed test is pure — these two claims are in tension.
- **Assumption that `overall` in `command-manifest.json` is edited honestly** — the only mechanical gate (H5).
- Treating hash-only log references as "immutable command outputs" per the acceptance text — letter satisfied, spirit weak (M1).

## 3. Missing considerations

Covered above: sanitization of the 538 KB `identity-occurrences.tsv` (private path leakage unassessed), raw-log retention (M1), `/tmp` cache isolation (M3), archive-hash binding at use time (M6), Linux channel coverage (M7), evidence-commit pinning (M8).

## 4. Risks to R9 authorization

1. If H2 reveals a dropped CI gate, the "exact-SHA full gate green" premise fails → genuine block until re-run.
2. If the LEAK marker traces to a real spawn leak (regression of `@native-spawn-test-leaks`), the stripped-PATH leg's clean status is questionable — likely still not R9-relevant, but must be dispositioned before `overall: pass`.
3. If the legacy-log restoration cannot be evidenced, R8 permanently carries an unverifiable real-state-mutation asterisk; acceptable only with explicit sign-off, since the acceptance clause was violated.
4. H5 means the finalization step could mechanically mint `r9_authorized: true` on an inconsistent manifest — fix before finalization, not after.

## 5. Genuinely solid

- Superseded-failure disclosure discipline (`homebrew-diagnostics.json`, `isolation-diagnostics.json`) is exemplary: failures classified with cause and explicitly excluded from acceptance, matching the first R8 attempt's stop-on-CI-red behavior.
- Exact-SHA binding is enforced at runtime in every channel check (`jq -e '.data.commit == ...'`), not just asserted in prose.
- Scope discipline: R9-only authorization, explicit expiry on identity change, correct treatment of 404s as observations, and correct reading of the Shipshape "publication not detected" warning as expected pre-R9 state.
- Scripts fail loudly (`set -euo pipefail`, hard assertions, diagnostic dumps on the bin-link check).
- The Homebrew drill models the real transition boundary correctly (real published 0.5.1 receipt, `brew update` as the migration trigger, single migration-truth source after the double-migration diagnostic).

## Verdict

**FIXABLE EVIDENCE GAPS.** No attached evidence proves a product defect. Do not flip `overall`/`r9_authorized` until: H1 prose corrected, H2 job list reconciled, H3 restoration evidence attached or its absence recorded, H4 dispositioned against `@native-spawn-test-leaks`, H5 gate hardened; M1–M8 addressed or explicitly waived in `validation.md`.

## Context request

- Kind: artifact
  Need: `issues/taskfleet-integrated-validation/evidence/r8-ci-jobs-summary.json` (full job names/IDs) and the `ci.yml` diff between `fa04841ad74c...` and `c3ef8b740ac...`
  Why: resolves H2. If jobs were consolidated, finding downgrades to LOW; if gates were dropped, R8 must block.
- Kind: artifact
  Need: before/after SHA-256 of the user legacy JSONL dispatch log around the disclosed probe-line removal, or confirmation no digests were captured.
  Why: determines whether H3 is closable or a permanent unverifiable caveat on R8.
- Kind: artifact
  Need: `evidence/isolation-diagnostics.json` cross-reference to, and current status of, issue `@native-spawn-test-leaks`; the nextest output section showing the LEAK line.
  Why: H4 — decides whether the LEAK marker is a benign attribution artifact or a regression of a prerequisite blocker.
- Kind: artifact
  Need: `verify-command-parity.sh` plus the frozen public-command inventory from ADR Phase 0.
  Why: M4 — determines whether the 7-command parity set is exhaustive per ADR Verification #2 or a sample.
- Kind: clarification
  Need: Where are the raw logs referenced by the seven `raw_sha256` values in `outputs-summary.txt` retained, and for how long?
  Why: M1 — decides whether the hash chain is verifiable evidence or dead references.

---

## Model: deepseek-v4-pro
[model:deepseek-v4-pro] [thread_id:api_f0ee66622ad04a769762776caaaf2818]

## Verdict

**FIXABLE EVIDENCE GAPS — do not authorize R9 on the current record.** The machine index already refuses (`overall: pending-review-and-residue`, `r9_authorized: false`). Fixing this is not purely mechanical: several acceptance legs need additional evidence, safer scripts, or a clean rerun.

---

## BLOCKER / HIGH findings

### 1. Machine authority contradicts the report, and the finalization script is too weak to safely flip it

`validation.md` says:

> **PASS** ... The machine-readable authority is `evidence/index.json`

But `evidence/index.json` says:

```json
"overall": "pending-review-and-residue",
"r9_authorized": false
```

That contradiction is expected before final review, but `verify-evidence-index.py` is not strong enough to become the authority afterward:

```python
passed = manifest.get("overall") == "pass"
...
"r9_authorized": passed
```

It authorizes R9 based solely on one top-level string in `command-manifest.json`. It does not verify:

- that `diff-residue` and `evidence-review` have non-pending results,
- that their output files exist and are hashed,
- that no command still has `result: pending`.

A one-line edit to `command-manifest.json` would flip `r9_authorized` to `true`. The finalization script must require explicit residue/review artifacts and at least all required command results to be terminal before writing `pass`.

---

### 2. The authoritative clean-PATH run contains a nextest LEAK marker

`evidence/command-manifest.json` records `stripped-path` as:

```json
"result": "pass-with-disclosed-warning"
```

`evidence/outputs-summary.txt` discloses:

> nextest emitted one delayed-exit LEAK marker on pure schema::wire_names_match_serde_round_trip

Under the stated R8 rule that every leg must pass and that failures/flakes block, an authoritative run with a non-PASS status should not be treated as accepted. The claim that no leftover process was observed afterward does not prove the test exited cleanly; it only proves no residue was found later.

This should either:

- be rerun as an authoritative clean isolated run with zero LEAK markers, or
- be formally accepted by an ADR-level rule that explicitly allows `LEAK` as advisory for the exact nextest version used.

Disclosure alone is not acceptance evidence.

---

### 3. Homebrew pre-live script appears to rewrite the formula URL without rewriting the SHA

`verify-homebrew-prelive.sh` does:

```python
s = s.replace(old, archive.resolve().as_uri())
```

It replaces the download URL with a local `file://` archive but does not update `sha256`.

If `target/distrib/taskfleet.rb` contains a `sha256` for the old hosted archive, Homebrew should reject the local candidate archive with a checksum mismatch. The claimed PASS is therefore not credible on the evidence shown.

The acceptance record must include the actual formula content or raw Homebrew log proving one of:

- the formula has no `sha256`,
- the local archive's SHA was already correct in the formula, or
- the script rewrites the SHA elsewhere.

As presented, this is a potential fabricated/unsound passing leg.

---

### 4. “Expensive stress test exercised separately” is unsupported

`evidence/outputs-summary.txt` says:

> skipped: taskfleet_core::stress_tests::flock_stress_50_threads_1000_iters (source-marked expensive, exercised separately in R8)

But `evidence/command-manifest.json` has no separate stress-test command and instead says:

> one explicitly ignored expensive stress test is not a required executed leg

Those claims conflict. If the stress test was genuinely exercised separately, there must be a manifest entry and output hash for that run. If it was not, the summary must not say it was exercised. The current evidence cannot demonstrate coverage either way.

---

### 5. Residue and review outputs are not bound by the immutable evidence index

`verify-evidence-index.py` only hashes files under `evidence/`:

```python
evidence = root / "evidence"
actual = {p.relative_to(evidence).as_posix(): p for p in evidence.rglob("*") ...}
```

The pending command outputs are:

```json
{"id":"diff-residue", ... "output":"residue.json"}
{"id":"evidence-review", ... "output":"review.md, assessment.json, assessment.md"}
```

Those files are not under `evidence/` and are not in `index.json`. Therefore the final two required checks would not be part of the immutable index at all. The machine authority would say `pass` without hashing the residue check or the human review.

This violates the requirement that the evidence index record output hashes for the required legs.

---

## MEDIUM findings

### 6. Raw logs referenced by `raw_sha256` are not preserved

`evidence/outputs-summary.txt` references many raw logs only by hash:

```text
raw_sha256=3705cb...
stripped_authoritative_raw_sha256=5fd54e...
authoritative_raw_sha256=614483a...
```

But `index.json` does not list those raw logs as artifacts. The hashes cannot be verified, sanitization cannot be audited, and the evidence is non-replayable. If raw logs are intentionally discarded, the report must say bounded-hash-only evidence is used and should not describe those outputs as immutable command outputs.

---

### 7. The validation scripts and report are not part of the hash index

`verify-homebrew-prelive.sh`, `verify-install-channels.sh`, `verify-evidence-index.py`, and `validation.md` are all outside `evidence/` and are not hashed in `index.json`.

A later commit could change the acceptance script after evidence generation without invalidating the index. The scripts define the acceptance behavior, so they must be pinned by hash just like the outputs. At minimum, `command-manifest.json` should include script SHA-256 values.

---

### 8. Some mutations occur outside disposable sandboxes

`verify-homebrew-prelive.sh`:

```bash
mkdir -p "$tmp/prefix/bin" "$tmp/home" <TMP_PATH>
...
HOMEBREW_CACHE=<TMP_PATH>
```

The trap removes only `$tmp`:

```bash
trap 'rm -rf "$tmp"' EXIT
```

`<TMP_PATH>` persists and is shared across runs. This violates the stated “every mutation destination is sandboxed” requirement.

`verify-install-channels.sh`:

```bash
CARGO_TARGET_DIR="$repo_root/target" cargo install --locked --path ...
```

This writes build artifacts into the production worktree rather than into `$tmp`. Gitignore may hide it, but it is still a mutation outside the disposable root.

Both should use `$tmp/cache` and `$tmp/target`, and both should be removed by the trap.

---

### 9. Old latest-installer stub lacks an explicit command manifest entry

`validation.md` claims the generated old `releases/latest/download/orchestratectl-installer.sh` stub:

> exits 1, writes only the migration message to stderr, points at the canonical installer, and leaves its disposable home empty

But `command-manifest.json` has no dedicated `installer-stub` command. It may be buried inside cargo-dist topology checks, but the manifest and outputs summary do not show the assertion. This is required ADR verification item 12, so it must be an explicit leg with its own output hash.

---

### 10. Homebrew version/reproducibility is ambiguous

`homebrew-acceptance.json` records:

```json
"homebrew_version": "6.0.21-52-g27d05ae"
```

But the script clones the host Homebrew repo and then runs an explicit `brew update`. It is not clear whether the recorded version is the host version before cloning or the actual post-update disposable-prefix version.

The evidence should capture the in-prefix `brew --version` immediately after `brew update`, before the disposable operations.

---

### 11. DAG blocker status is not evidenced

The issue frontmatter still lists:

```yaml
blocked_by: ['@taskfleet-distribution-topology', '@publish-crates-fixture-symlink-chmod', ...]
```

No evidence shows those blockers are closed. `issuectl dag --json` is hashed in the index but not attached. R9 authorization should include current DAG state showing the R8 issue is no longer blocked, especially because the item itself says not to spawn before human disposition.

---

## LOW findings

- `source-identity.json` records `origin_main_at_start` but not `local_head_at_start`, despite `validation.md` claiming local HEAD equality.
- `cargo package --workspace --no-verify` creates package archives but does not build them from the `.crate` files. If package dry-run is a required proof, add a verified package build or confirm the wrapper does it separately.
- `homebrew-acceptance.json` says `"public_mutation": false`, but the script uses an explicit `brew update` on a cloned Homebrew prefix; network fetch behavior should be documented to support that claim.

---

## Genuinely solid

The exact-SHA binding is generally strong: the tested commit and tree are repeated consistently, the CI run is pinned to the same SHA in `source-identity.json`, and the disposable install scripts verify the candidate binary reports `c3ef8b740ac531f12ce81c759ed209d178cf36bd`.

The disclosure of failed diagnostic attempts in `homebrew-diagnostics.json` and `isolation-diagnostics.json` is transparent and mostly correctly excludes setup errors from acceptance evidence.

The separation between R8/R9/R10/R11 authority is explicit: `release_authorized` is always false, and `validation.md` does not authorize publication, tap activation, or release.

---

## Context request

| Kind | Need | Why |
|---|---|---|
| artifact | `target/distrib/taskfleet.rb` and the raw Homebrew acceptance log matching `authoritative_raw_sha256=614483a...` | Confirms whether the formula has a `sha256` and whether the file-URL replacement was valid. Could invalidate Finding 3. |
| artifact | `evidence/outputs-summary.txt` backing raw logs for `stripped_authoritative_raw_sha256` and ordinary full run | Needed to audit the LEAK marker, exact nextest output, and whether the run exited cleanly. |
| artifact | Separate stress-test run output/log if `flock_stress_50_threads_1000_iters` was actually exercised | Resolves the contradiction in Finding 4 and determines if test coverage is incomplete. |
| artifact | `residue.json`, `review.md`, `assessment.json`, `assessment.md` if already produced | Determines whether pending required checks can be finalized and whether they should be included in the index. |
| artifact | `issuectl dag --json` current output and blocker statuses | Confirms blocked_by issues are closed. Could change Finding 11. |
| clarification | Does R8 acceptance policy consider `LEAK` a pass-with-disclosed-warning or a blocking non-PASS? | Directly determines whether the authoritative clean-PATH run may count. Could change Finding 2. |
