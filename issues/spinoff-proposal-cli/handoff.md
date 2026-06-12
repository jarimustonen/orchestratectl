# Handoff — DISCUSS items from /llm-review

Three product/semantic decisions surfaced during the multi-model review that aren't defects but need a human call. The triage details live in `history/assessment-spinoff-proposal-cli.json` (F7, F11, F17). Finnish write-ups in `/tmp/assess-findings.bnPSiC/issue-spinoff-*.md`.

## F7 — `--idempotency-key` semantics for `spinoff approve`

**The question:** Should `--idempotency-key` cover the external `issuectl new` call, or be honestly local-only?

Today it's local-only: the key is forwarded to `append_and_apply_unlocked`, which dedupes the event-log append. It does NOT prevent a duplicate `issuectl new` invocation on retry. The user-facing `--help` is silent on this distinction.

Options:
- **A.** Plumb a deterministic key (e.g. `<run-id>:<proposal-id>:materialize`) into `issuectl new`. Requires `issuectl new` to gain an idempotency-key flag (it doesn't have one today).
- **B.** Keep local-only, document the gap explicitly in `--help` and `AGENTS.md`. Steer users toward `--issue-slug` for retry-safety.
- **C.** Bundle with the `spinoff-issuectl-materialization-arch` redesign — the answer falls out of whichever architecture wins.

**Tentative recommendation:** B in the short term, revisit as part of the materialization-arch issue. Option A requires upstream work in `issuectl`.

## F11 — Re-approve with different `--issue-slug`

**The question:** What should `spinoff approve --issue-slug new-slug` do when the proposal was already approved with `old-slug`?

Today: silently returns `old-slug` with `idempotent_replay: true`. Test `approve_is_idempotent_on_reapproval` asserts this explicitly. Multiple reviewers flagged this as a UX trap — the caller may believe `new-slug` got attached.

Options:
- **A.** Return `proposal_already_approved` error when slugs differ.
- **B.** Introduce a separate `spinoff attach-issue` verb for slug-attach. Keeps `approve` as a one-shot terminal transition; allows attaching a slug post-hoc after a failed `issuectl new`.
- **C.** Keep current behavior, document as intentional in `--help`.

**Tentative recommendation:** A. Reverting silent ignores into an explicit error is cheap and matches AGENTS-AI-FIRST-CLI's strict-input ethos. B is nicer architecturally but is its own design.

## F17 — `proposed` vs `pending` vocabulary

**The question:** `SpinoffStatus::Proposed` on disk surfaces as `pending` in the CLI (`spinoff list --status pending`). Two terms for one concept.

Options:
- **A.** Migrate schema variant `Proposed → Pending`. One state-schema version bump (the dirs and projections rewrite — small footprint since MVP hasn't shipped). Reducer + tests update.
- **B.** Keep both; document the mapping prominently in `--help`, `design.md` §2.5, and `AGENTS.md`.

**Tentative recommendation:** A if this can happen before MVP locks (state-schema version still at 1). Otherwise B with a clear note.

---

Each of these has a staged `issuectl new` command in `history/assessment-spinoff-proposal-cli.md`. The user (or `/issue`) can file them once decisions are made.
