# supervisor-dies-before-worker-node — analysis, review, and decision

## Root-cause analysis

The failure mode (3× on 2026-08-10 under FS/CPU saturation): a run left at
`status: pending`, `node_count == 0`, dead/absent supervisor, `updated_at ==
created_at`, an orphaned base-main worktree with 0 commits — a **stillborn** run.

Tracing the create path (`crates/octl-cli/src/run/create.rs`):

- For a **top-level worker** (e.g. `--kind spinoff`), `run create` runs
  synchronously: emit `run.created` → `create.sh` (worktree + tmux + agent, up
  to `--agent-startup-timeout`) → `node.created` (n-0001) → **then** spawn the
  supervisor (`spawn_supervisor_or_fail`, guarded by a readiness pipe). The
  supervisor is spawned **after** `node.created`; it never creates the first
  worker node itself. So the issue-reporter's mitigation (a) "supervisor
  retry/backoff on worker-node creation" is based on a misread of the
  architecture — no supervisor exists yet at that point.
- The stillborn signature (`node_count == 0`) therefore arises when `create.sh`
  **fails** (e.g. a `git index.lock` race or `workmux-add-failed` under
  saturation) and `run create` returns an error, leaving the top-level run on
  disk in `pending` for idempotent-key replay (create.rs:607-609), OR when the
  whole `run create` process is killed mid-flight before `node.created`.

**Key finding: detection already exists and is already wired.** The codebase has
`is_stillborn(...)` in `run/stalled.rs`, already consumed by `run wait` (settles
promptly, returns the structured error "supervisor died before creating any
worker node") and `run show` (folds into `stalled: true` + a human hint). So
mitigation (b) "run create fail-fast instead of a pending lie" is effectively
already satisfied — `run create` returns `supervisor_spawn_failed` /the create.sh
error, not a success.

**The one real gap:** `run list` did NOT surface stillborn — it computed
`stalled` only for `--kind orchestrate`. So a stillborn run appeared as an
ordinary `pending` row, indistinguishable from a healthy one. `run list` is
exactly the surface an operator / a `/stint` monitor sweeps, so this was the
"looks stuck until someone notices" silent block.

## Chosen fix (scoped, safe, read-only)

Wire the existing `is_stillborn` detector into `run list`, and give the wire DTO
a first-class `stillborn: bool`:

1. `run/list.rs` — compute `is_stillborn` per run from data already held under
   the shared lock (no extra I/O), **age-gated** (see below); keep the
   orchestrate `is_stalled`; `stalled = stalled_orchestrate || stillborn`
   (the umbrella, matching `show`/`wait`).
2. `run/dto.rs` — add `stillborn` + `with_stillborn`.
3. `run/show.rs` — also set `.with_stillborn` for cross-verb consistency.
4. Text output — `pending (stillborn)` marker, distinct from `(stalled)`.

Touches **no** event/reducer/schema/lock write path — all 5 state-integrity
invariants are not in play (pure read-time computed hints under the existing
shared lock).

## LLM review (`/llm-review`, gemini-3.1-pro / gpt-5.6-sol / deepseek-v4-pro)

### Applied (FIX-class)

- **Transient create-window false positive (all 3 reviewers, CONFIRMED).** For a
  top-level spinoff the supervisor spawns only after `node.created`, so during
  the whole `create.sh` window a healthy in-flight run presents the exact
  stillborn shape. Because `run list` sweeps every run — including ones another
  process is mid-`run create` on (the incident's own parallel-wave context) — a
  monitor over `run list --json` could flag/cancel a healthy run. **Fix:** an
  age gate in `list.rs` only (`STILLBORN_LIST_GRACE_SECS`, default 900s = the
  supervisor's own no-worker grace; `OCTL_STILLBORN_LIST_GRACE_SECS` override).
  `is_stillborn` itself stays grace-free — `run wait` must settle promptly (a
  grace would re-break `run-wait-stillborn-run-not-detected`), and `show`/`wait`
  are called on a specific run whose create already returned, so 0 nodes there
  means definitively stillborn. New regression test
  `list_within_grace_does_not_flag_stillborn`.
- **DTO "mutually exclusive" doc wording (deepseek).** Clarified: the underlying
  *detections* are exclusive, but `stalled` is the umbrella, so a stillborn run
  carries BOTH flags. Comment rewritten.
- **Test hardening (openai).** Assert `run list` (text) command success before
  reading stdout.
- **I/O micro-opt (gemini).** Skip the `n-0001` read once `stillborn` holds.

### Rejected (with reason)

- **"Don't overload `stalled`" (all 3).** The reviewers lacked `show.rs:146` /
  `wait.rs` context, where `stalled` ALREADY means "not progressing (either
  shape)". Making `list` disjoint would make it *disagree* with `show`/`wait`.
  The umbrella is the consistent choice; the doc fix resolves the confusion.
- **Wire version bump / `From` builder refactor (openai/gemini).** `stillborn`
  is additive; this matches exactly how `stalled` itself was added (same
  `with_*` builder + `summary_pins_wire_shape` pin). Consistency with the
  established convention wins.

### Deferred (→ spinoff proposals)

- **Auto-terminalize a stillborn run to `Failed`** rather than leaving it
  `pending` (gemini/openai/deepseek). Legitimate, but larger: a read path must
  stay read-only, and no live actor exists to append the terminal event (the
  supervisor never spawned). Needs a new write actor / a `run create` failure
  event; relates to the still-open `supervisor-spawn-fails-silently-at-run-create`.
- **`SupervisorState { NotRegistered, Alive, Dead }`** to distinguish
  "recorded-then-died" (immediate) from "never registered" (needs grace). The
  age gate achieves the practical goal without the refactor; the enum is a
  cleaner future model.
- **Move/rename `is_stillborn` out of the `stalled` module** (deepseek) —
  cosmetic, pre-existing.
