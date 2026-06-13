# mvp-polish-wave-2 — handoff

`/llm-review` (gemini-3.1-pro-preview + gpt-5.5 + claude-opus-4-7 +
deepseek-v4-pro, 1 round) was run on the 4 commits of this branch. Full
report at `history/review-mvp-polish-wave-2.md`. Findings were triaged
into FIX (applied this branch) and DISCUSS (deferred to follow-up
issues).

## FIX — applied in commit "review fixes"

- F1 `<none>` sentinel → `Value::Null` in `mismatch_error`'s structured
  `expected` field. Distinct human messages for the two paths.
- F2 Stronger `base32_lower_10` test fixtures: RFC 4648 §10 known vector
  (`foobar\0` → `mzxw6ytboi`) plus two asymmetric inputs that catch
  MSB/LSB swaps and `>>= 6` off-by-one.
- F3 Real fixture literal in `deterministic_id_formula_matches_design_md_1_4`
  (`d-a4ldwigubn`) — computed independently from the spec primitives.
- F4 `base32_lower_10` now takes `&[u8; 7]` (type-level length guarantee);
  call site slices `digest[..7]`.
- F5 Collision-rate comment corrected: ~3M → ~40M (50% birthday midpoint).
- F6 New integration test `approve_with_slug_after_approval_without_recorded_slug_errors`
  covering the `recorded=None + requested=Some` path; asserts
  `expected` is JSON `null`.

## DISCUSS — defer / spin off

- D1 **`item_kind` spec drift** (anthropic). Implementation hashes
  `child_run_id : child_node_id : report_seq : item_kind : item_index`;
  design.md §1.4 omits `item_kind` (prefix-only disambiguation). Either
  (a) update §1.4 to include `item_kind` as a normative axis, or
  (b) remove `item_kind` from the hash and rely on the `d-`/`s-` prefix.
  Validation.md V7 currently half-acknowledges this. Recommendation:
  decide before any external consumer derives IDs.

- D2 **Concurrent-approve test doesn't deterministically force both
  threads through the lock window** (gpt-5.5, deepseek). The current
  test can pass with only the pre-lock path firing. Real fix needs a
  barrier hook (env-gated) in `approve.rs` that releases both processes
  past preflight together. Test-infrastructure work; defer.

- D3 **Distinct error code `proposal_approved_with_different_slug`**
  (anthropic). Reusing `proposal_already_approved` conflates benign
  idempotent retries with real caller bugs. Cheap to add, but every
  consumer must learn the new code; defer to a CLI-error-taxonomy pass.

- D4 **Operational recovery for "approved without slug"** (anthropic,
  gpt-5.5). Once a proposal is approved with `accepted_as_issue_slug=null`
  (transient `issuectl` failure), there's no way to bind a slug later.
  Options: a `spinoff bind-issue` verb, or a relaxed `approve --issue-slug`
  that only fills a missing slug. Needs a design decision — defer.

- D5 **Rename `octl_core::SCHEMA_VERSION` → `ENVELOPE_SCHEMA_VERSION`**
  (anthropic). Naming parity with `STATE_SCHEMA_VERSION`. Trivial rename;
  defer to next CLI-touch PR to avoid noise here.

- D6 **B6 doc — retry safety needs a *stable* slug across retries**
  (anthropic, gpt-5.5). Under the new B5 contract, a caller using
  `uuidgen` per retry will hit `proposal_already_approved` on attempt 2.
  Add one sentence to the `--idempotency-key` doc clarifying this.
  Trivial doc tweak; can fold into next docs pass.

- D7 **No state-schema bump for hex → base32 migration**. Validation.md
  notes "before any external consumer locked the format"; that's enough
  for MVP, but if a hex-era `runs/` directory exists it will not dedup
  against a base32-era replay. Worth an explicit MVP-pre-release note in
  CHANGELOG when one exists.

## SKIP — out of scope or contradicting decisions doc

- Splitting envelope into `octl-protocol` crate (gpt-5.5). B4 is
  intentionally a minimal lift; the multi-crate split is a separate
  architectural decision.
- Replacing inline `base32_lower_10` with the `data-encoding` crate
  (anthropic). Dependency-add decision; the inline helper is ~20 lines
  and now has known-vector coverage.
- Wrapping `prefix: char` in an enum (gpt-5.5). Refactor outside scope.
- `#[deprecated]` shim for `octl_cli::error::SCHEMA_VERSION`. octl-cli
  is a binary; no external Rust consumers of its crate root.
