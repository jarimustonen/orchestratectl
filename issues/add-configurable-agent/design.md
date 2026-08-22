# Design: configurable agent profiles

Status: v2 historical baseline (2026-08-17) — **not approved for implementation**.
It was revised after a 4-role LLM panel (architect / security /
maintainability / test-strategist; synthesis in
`history/2026-08-17-panel-add-configurable-agent.md`, gitignored). Before any
slice below is implemented, revise this design after `worker-telemetry-protocol`
so autonomous eligibility, telemetry support, pi adapter requirements, and
interactive-only Claude operation are explicit. The combined result then stops
at `worker-control-plane-review` for Jari's approval.
Issue: `add-configurable-agent`.

## 1. Problem and constraints (told, not re-derived)

`run create` today selects only a *harness* (`claude | pi`) via
`harness::select` (flag > env > `[harness]` in `~/.orchestratectl/config.toml` >
built-in `claude`). There is no way to pick a **model** for one spawn; the pi
workaround mutates the global `~/.pi/agent/settings.json`, which is racy across
concurrent spawns and easy to leave dirty. The issue (plus Jari's steers in its
comments) fixes these constraints:

- **Capability names are the interface.** Callers say `expert` / `standard` /
  `fast` / `secure`-class names, never raw vendor model IDs. Raw `--model` /
  `--effort` flags are an OPEN question, explicitly deferred (build the
  capability layer first; add raw flags only if a need survives it).
- **Two axes, not four tiers.** Capability (fast < standard < expert) is one
  axis; **data residency** (`local` vs remote) is orthogonal. A local profile
  must NEVER fall back to a remote agent — exhausted fallbacks is the correct
  outcome. Automatic selection must be able to constrain residency
  independently of capability.
- **Ships useful defaults, not Jari's fleet.** The role set and a sensible
  mapping exist with no config file present; the model IDs in the issue's
  example set are illustrative user config only (see `audit-no-user-specifics`).
- **Per-run override is the primitive** and a legitimate first slice; the
  resolved choice must be recorded on the manifest and visible in `run show`.
- **No mutation of global harness settings, ever** — the design is only a
  solution if a per-spawn model choice leaves `~/.pi/agent/settings.json`
  (and every other global config) untouched.
- Backward compatibility: an installation that never opts into profiles keeps
  byte-identical behaviour (including create.sh argv).

## 2. Key mechanism fact (verified 2026-08-17)

The launch chain already transports a **full agent command line**:
`run create` → `create.sh --agent <agent-cmd>` → `workmux add -a "<cmd>"` →
pane command. workmux's `agent` value is a command string (its docs show
`"env … claude"`, `"claude --dangerously-skip-permissions"`), `-a` overrides
it, and built-in agents (`claude`, `pi`, …) are auto-detected *as leading
literal commands with args*, so prompt injection still works. Therefore a
profile can resolve to e.g. `pi --model "openai/gpt-5.6-sol:high"` and ride
the existing spawn path with **zero** global-settings mutation.

**Implementation starts with a smoke test decomposed into three orthogonal
assertions** (panel: one lucky spawn proves nothing):

- **(a) argv transport:** spawn with command `["claude", "--model", "haiku"]`
  and assert the pane's actual process argv (`ps -o args=` on the discovered
  pid) equals it exactly — not "contains haiku".
- **(b) prompt injection:** with an argument containing a space/quote, assert
  the brief still lands in the agent.
- **(c) PID discovery:** assert create.sh's walk populates `agent_pid_hint`
  for both a claude-family and a pi-family command.

If any assertion fails on a needed form, fall back to generating a temp
workmux config (`workmux add --config`) — but the command-string path is
expected to work.

Known sharp edge, per candidate command: create.sh's PID discovery walks for
`claude*|node*` process names. `claude`/`pi` both satisfy it today; a custom
local wrapper (e.g. `pi-gemma`) must too, or spawn fails
`agent-pid-undiscoverable`. Documented, not solved, here — and the failure
shape is **pinned by a test** (§9) so it stays loud and predictable. A
per-candidate `pid_pattern` field is the likely follow-up shape if local
wrappers multiply (panel/maintainability), not built now.

## 3. Model

### 3.1 Profile

A **profile** is a named, ordered list of launchable agent candidates plus
selection metadata. **Profile definitions live in exactly two places: the
built-in catalog and the USER config layer.** The repo layer selects, never
defines (§5 — this is the panel's central security resolution).

```toml
# user layer only: ~/.orchestratectl/config.toml
[profiles.standard]
description = "General-purpose high-capability work"
capability  = "standard"          # "fast" | "standard" | "expert" (ordered)
local       = false                # residency axis; default false
agents = [
  { harness = "pi",     command = ["pi", "--model", "openai/gpt-5.6-sol:high"] },
  { harness = "claude", command = ["claude", "--model", "opus"] },
]
```

- `agents[0]` is the preferred invocation; the rest are fallbacks, in order.
  (One ordered list, not preferred/fallback field pairs — simpler and covers
  "zero or more fallbacks" naturally.)
- `command` is **argv as a TOML array** — the only accepted form, never a raw
  shell string. orchestratectl joins it into the single `-a` value with the
  repo's existing POSIX single-quote helper (see `shell-quote-dedup` — this
  work is a third caller and should land on the deduped helper), in exactly
  one place in Rust. Property-tested round-trip (§9). **Profile commands must
  not contain secrets** (tokens belong in env / the agent's own config;
  workmux structured agents support `env`) — commands are recorded on
  manifests, events and `--json` surfaces unredacted, by design.
- `harness` must be one of `harness::KNOWN_HARNESSES` — it names the *protocol
  family* (which prompt-translation shim applies, `harness::prompt`), while
  `command[0]` is the executable. This is how `secure` can run `pi-gemma`
  under `harness = "pi"`.
- `capability` is an optional machine-readable rank so future escalation
  tooling ("retry one tier up", the terra→sol case) can order profiles without
  parsing names. It does not affect resolution of an explicitly named profile.
- `local = true` marks the residency class. **Honesty requirement (panel):
  `local` is a DECLARED residency class, not an egress sandbox — the binary
  cannot verify where a command sends data, and docs + JSON output must say
  so.** The user is the sole residency authority for commands they author
  (and they author all of them, since definitions are user-layer-only). What
  the binary DOES enforce, mechanically:
  - fallback never leaves the profile's own candidate list;
  - exhausted candidates on a local profile → hard error, never a remote
    retry, never fallthrough to another profile or the `[harness]` path;
  - supervisor retry re-launches only the recorded resolved command (§4.3);
  - automatic/agent-level selection never substitutes across profiles.

### 3.2 Built-in default catalog

With no config file, four profiles exist (names are the stable default role
vocabulary the issue asks for; all overridable/extensible in user config):

| name       | capability | local | default agents                        |
|------------|-----------|-------|----------------------------------------|
| `expert`   | expert    | no    | `["claude", "--model", "opus"]`        |
| `standard` | standard  | no    | `["claude"]` (user's default model)    |
| `fast`     | fast      | no    | `["claude", "--model", "haiku"]`       |
| `secure`   | (none)    | yes   | **empty** — resolving errors with "no local agent configured; define [profiles.secure] with a local command" |

Rationale: claude is already the tool's built-in default harness, so mapping
default tiers onto the claude model ladder adds no new vendor assumption and
uses model *aliases*, not dated IDs. `secure` ships as a defined name with an
intentionally empty candidate list because no universal local model exists —
failing with an actionable error is the correct out-of-box behaviour and keeps
the residency promise honest. The issue's `expert/standard/implementer/secure`
example maps as: implementer ≙ `fast`.

User-layer `[profiles.standard]` **replaces** the built-in `standard` wholesale
(per-name replace, no field-level merge — franken-profiles that mix layers are
undebuggable; panel consensus). Defining a new name adds to the catalog.

## 4. Selection and resolution

### 4.1 Which profile (per run)

1. flag `run create --profile <name>` (`conflicts_with = "harness"` — a
   profile pins the harness per candidate; snapshot the clap error, §9)
2. env `ORCHESTRATECTL_PROFILE` (empty/whitespace = unset, mirroring
   `ORCHESTRATECTL_HARNESS`). **Both env vars set non-empty is a hard error**
   naming both — loud beats guessing which mechanism the user meant.
3. config file layer, **specificity first, then mechanism** (panel/architect:
   adding a `[profile] default` must not silently void a deliberate per-kind
   harness choice):
   `profile.per_kind.<kind>` > `harness.per_kind.<kind>` >
   `profile.default` > `harness.default`.
   The repo layer sits above the user layer within each of the two profile
   keys (§5); `[harness]` remains user-layer-only.
4. nothing: built-in default harness path, byte-identical argv,
   `manifest.profile = None`.

When a profile is selected at any layer, profile resolution owns the launch.
Unknown profile name → hard error listing the catalog and naming the source
layer (same pattern as `invalid_harness`, including a repo-file-sourced name).

### 4.2 Which candidate (fallback)

Deterministic, observable, and narrow: walk `agents` in order; a candidate is
**available** iff `command[0]` resolves on the resolver process's PATH
(`which`-equivalent, injectable as a pure closure for tests). First available
candidate wins. Record, per run:

- `profile` + `profile_source` (flag/env/file/default)
- resolved `harness`, the full resolved command string
- `fallback_from`: the skipped candidates and per-candidate reason
  (`executable not found`) when index > 0.

All candidates unavailable → hard error naming every candidate and its reason
(for `secure`-class profiles this is the mandated fail-closed outcome).

**Honest scope (panel/architect):** PATH-presence covers *per-machine
installation differences* (the same synced config on hosts with different
fleets) — it does NOT cover provider/runtime failures (API quota, network,
upstream model errors). With both `pi` and `claude` installed, the fallback
never fires at create time; a preferred-agent runtime failure surfaces
through the existing supervisor death/attention paths. "Advance to the next
candidate / one tier up on worker death" is a named follow-up (§8), not
silently implied. Two documented gaps, both attributed loudly rather than
probed: the resolver's PATH may differ from the tmux pane's PATH (login-shell
rc), and a probe can pass at resolve time yet exec can fail at spawn time —
both land in the existing spawn failure (`agent-pid-undiscoverable` /
workmux error) and the error must name the candidate that was launched.

### 4.3 Supervisor retry

The retry path re-launches from the **manifest's recorded resolved command**
verbatim — it does not re-run resolution, even if config or PATH changed.
Same guarantee shape as today's "retry never silently drops back to claude";
it is also part of the residency guarantee (a `secure` run can never be
retried onto anything but the command it launched with). Tamper note
(panel/security, accepted): the runs dir is outside the worktree and
same-user; an agent able to write it can already execute arbitrary commands
as that user, so verbatim retry adds no new privilege.

### 4.4 Automatic selection (planning)

No planner lives in the binary. Automatic matching of work to tiers is an
agent/skill-level concern; the binary's obligations are: (a) expose the
catalog machine-readably (`profile list --json`: name, description,
capability, local, candidates, availability), (b) resolve deterministically,
(c) record the outcome. Skills (stint-start, worktree-spinoff, fan-out) then
choose a profile by description/capability and pass `--profile`; a
data-sensitive task selects a `local` profile explicitly. Agent discipline is
not a security control (panel/security) — a hard per-task residency
constraint (`run create --require-local`, refusing any non-local resolution)
is the named follow-up if/when automatic selection actually ships.

## 5. Config layering (user + repo)

**Panel resolution (defer-to-security): the repo layer may SELECT profiles,
never DEFINE them.** A `.orchestratectl.toml` that could define
`[profiles.*]` argv and auto-select it via `[profile] default` would turn
`run create` in any checked-out repo into arbitrary command execution — the
command runs before any agent/sandbox context, invisibly. Restricting the
repo layer to selection kills that class outright (no repo-authored command
exists to run, no residency redefinition is possible), needs no trust-grant
flow, and matches the real need: a repo declares which *tier* its work wants;
the model fleet is per-user/per-machine anyway (a repo-committed fleet would
also violate `audit-no-user-specifics`).

- **Paths:** user `~/.orchestratectl/config.toml` (existing, honors
  `$ORCHESTRATECTL_HOME`); repo `<repo-root>/.orchestratectl.toml` (single
  committable file; root = `git rev-parse --show-toplevel` of the CWD at
  `run create` time; no git repo → no repo layer).
- **Repo file schema v1 — selection only:** `[profile] default` and
  `[profile] per_kind.<kind>`, values are profile *names* resolved against
  built-ins + user layer at run time. Load-time hard errors: any
  `[profiles.*]` table, any `[harness]` section, unknown keys/kinds (same
  strictness pattern as the user file; unknown top-level sections tolerated
  for forward compat). A malformed repo file is a hard error naming the path.
- **Missing file at either layer = empty layer** (exact current behaviour of
  the user file).
- **Merge semantics:** repo `[profile]` selection keys sit above the user's
  `[profile]` keys within the file layer of §4.1 (scalar replace per key;
  `per_kind` merged by kind, repo entry wins). Profile *definitions* merge
  built-ins ⊕ user by name, wholesale replace.
- A repo-named profile that does not exist in built-ins + user layer fails at
  resolution with the catalog listed and the repo file named — it never
  falls through silently.
- `config path` grows a per-layer view; `config show` gains `profile.*` rows.
  **Coordination:** `config-show-layered-view` (lane `surface`, seq 10) is
  sequenced BEFORE this issue (seq 20) and introduces the layered/raw
  inspection schema; this design's `config show` additions ride that schema.
  If this lands first in calendar time, keep `config show` additions minimal
  (effective values only) and let that issue own the layered view.

## 6. Recording and surfaces

- `run.created` event data: `profile`, `profile_source`, resolved
  `agent_command` (single quoted string as launched), existing
  `harness`/`harness_source` (harness sourced `profile` when a profile chose
  it — extend `HarnessSource` with `Profile`; the naming stretch is accepted
  over a parallel enum, rename to `AgentSelectionSource` only if a third
  mechanism appears), and `agent_fallback_reason: Option<String>` (present
  only when a fallback was taken).
- `Manifest` (all `#[serde(default)]`, legacy-readable, **no schema-version
  bump** — consistent with how `harness` itself landed): `profile:
  Option<String>`, `agent_command: Option<String>`,
  `agent_fallback_reason: Option<String>`.
- `run show` / `run list --json` surface them; `run show` human output prints
  `profile (fallback: <reason>)` when set. JSON carries the *structured*
  resolution (harness + argv) separately from the launched string, so a
  quoting change touches one snapshot, not five (panel/test-strategist).
- New read-only noun verb: `orchestratectl profile list [--json]` — the
  effective catalog with per-profile source layer (`built-in | user`), the
  repo file's selection (if any), and per-candidate availability
  (computed at list time; it is a PATH snapshot, labeled as such). Emits
  empty-candidate profiles too (`candidates: []`). This is the dry-run
  observability surface the issue requires; its JSON schema is **frozen at
  v1 by a golden test the moment it lands** — skills will consume it.

## 7. Slicing (may land as separate worktrees, in this order)

1. **A — resolver + per-run override (MVP):** `ProfileCatalog` (built-ins
   only), `--profile` flag + env, candidate resolution (injectable PATH
   probe), shell-quoted `-a` passthrough on the deduped quote helper,
   manifest/event recording, `run show` surfacing, the §2 three-assertion
   smoke test. No config file changes. Lands the intake's primitive and kills
   the settings.json workaround.
2. **B — user-layer `[profiles]` + `[profile]` sections + `profile list`:**
   strict parsing/validation, specificity-first file precedence, per-kind
   defaults, frozen `profile list --json` v1.
3. **C — repo selection layer:** `.orchestratectl.toml` (selection-only
   schema), repo-name resolution errors, `config path`/`show` integration
   (coordinate with `config-show-layered-view`).
4. **D — skill/docs adoption:** bundled SKILL templates mention `--profile`
   where they mention `--harness`; AGENTS docs (incl. the "local is declared,
   not enforced" honesty note); file the follow-up issues from §8 that are
   still wanted.

Slice tests must not entangle: A must not depend on B's config files; B must
include a no-repo-layer scenario; C must include a no-git scenario. Slices B
and C touch `config/mod.rs` + `harness/select.rs` + `run/create.rs` (the
issue's collision warning: cross-cutting into lifecycle territory — do not
parallelize with lifecycle-lane units; run the integrated gate).

## 8. Explicitly out of scope / deferred (file as issues at slice D)

- Raw `--model` / `--effort` flags on `run create` (open question, default:
  not built; revisit only with a concrete surviving need).
- In-binary automatic planner/tier matcher (agent-level concern, §4.4).
- Escalation verb ("retry one tier up") and **candidate-advance on worker
  death** — the runtime-failure fallback the create-time probe deliberately
  does not provide (§4.2).
- `run create --require-local` hard residency constraint (§4.4).
- Per-candidate `pid_pattern` to generalize create.sh's `claude*|node*` PID
  walk (§2).
- Repo-layer profile *definitions* behind an explicit trust-grant flow
  (direnv-`allow`-style) — only if a concrete need survives the
  selection-only layer.
- Availability probes beyond PATH presence; secret redaction machinery for
  command strings (policy instead: no secrets in argv, §3.1).

## 9. Test plan (maps to acceptance criteria; layers per panel)

**Pure unit (no I/O; resolver takes injected env/config/PATH-probe):**
- Precedence: flag / env / file (specificity-first interleaving of §4.1,
  incl. profile.default vs harness.per_kind) / built-in; both-env-set error;
  empty/whitespace env unset; unknown name names its source layer;
  `--profile`+`--harness` clap conflict (snapshotted).
- Fallback walk: order, per-candidate reasons, exhausted-fallbacks error
  naming every candidate; empty candidate list (`secure`) errors actionably.
- Residency negatives, load + resolve: local profile exhaustion fails closed
  — no fallthrough to another profile, the `[harness]` path, or a built-in.
- Quoting: **property test** — argv → quoted string → `sh -c` round-trips to
  the original argv over a hostile alphabet (spaces, quotes, `$`, backticks,
  `;`, newlines, leading dashes, unicode).
- Merge: built-ins ⊕ user wholesale per-name replace; repo selection keys
  over user selection keys; repo file with `[profiles.*]` or `[harness]`
  rejected at load; malformed repo TOML hard error naming the path.

**Integration (temp HOME, temp git repo, spy on the spawn boundary):**
- **Byte-identical no-profile argv** asserted at the create.sh spy — exact
  argv diff vs today for identical inputs, not a manifest assertion.
- Profile-selected spawn: spy receives the exact quoted `-a` string.
- Concurrent spawns with distinct profiles: each spy call gets its own
  correct `-a` string; no global file is touched (the property the
  settings.json workaround lacked).
- Manifest + `run.created` recording end-to-end incl. `fallback_reason`.
- **Retry-after-crash:** run resolved via fallback, worker dies, supervisor
  re-launches `manifest.agent_command` verbatim even though the originally
  preferred candidate is now on PATH.
- Repo-root detection edges: submodule, worktree, CWD outside any repo.
- `profile list --json` golden schema v1; availability rows for present and
  absent executables; empty-candidate profiles emitted.

**e2e (real create.sh / workmux / tmux):**
- The §2 smoke test, three orthogonal assertions.
- `run create --profile fast` happy path: pane `ps` argv matches.
- `run create --profile secure` (unconfigured): fail-closed error.
- Pinned failure shape: a dummy `pi-gemma` binary whose tree never shows
  `claude*|node*` → `agent-pid-undiscoverable`, message snapshot.

**Snapshot discipline:** split snapshots by surface (help / JSON envelope /
human output) and scenario (no-profile / selected / fallback / exhausted);
slice A's `run create --help` snapshot must be a strict superset of today's.
