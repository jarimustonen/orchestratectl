# Design: configurable agent profiles

Status: DRAFT (2026-08-17) — pending multi-LLM review pass.
Issue: `add-configurable-agent`. Author: design session 2026-08-17.

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

Implementation must start with a smoke test of this (one real
`workmux add -a "claude --model haiku"` spawn); if prompt injection or agent
PID discovery breaks on any needed form, fall back to generating a temp
workmux config (`workmux add --config`) — but the command-string path is
expected to work.

Known sharp edge to verify per candidate command: create.sh's PID discovery
walks for `claude*|node*` process names. `claude`/`pi` both satisfy it today; a
custom local wrapper (e.g. `pi-gemma`) must too, or spawn fails
`agent-pid-undiscoverable`. Documented, not solved, here.

## 3. Model

### 3.1 Profile

A **profile** is a named, ordered list of launchable agent candidates plus
selection metadata:

```toml
# user layer: ~/.orchestratectl/config.toml   (repo layer: see §5)
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
- `command` is **argv as a TOML array** (strict, no shell parsing in config);
  orchestratectl shell-quotes it into the single `-a` string at spawn time.
- `harness` must be one of `harness::KNOWN_HARNESSES` — it names the *protocol
  family* (which prompt-translation shim applies, `harness::prompt`), while
  `command[0]` is the executable. This is how `secure` can run `pi-gemma`
  under `harness = "pi"`.
- `capability` is an optional machine-readable rank so future escalation
  tooling ("retry one tier up", the terra→sol case) can order profiles without
  parsing names. It does not affect resolution of an explicitly named profile.
- `local = true` marks the residency class. **Validation (load-time, hard
  error): every agent of a `local` profile must itself be local.** Since the
  binary cannot prove where a command sends data, `local` is a *declared*
  attribute — but the invariant we CAN enforce is structural: a `local`
  profile's candidate list is what the user wrote for it, and automatic tools
  never splice a remote candidate in, never fall through to another profile,
  and never fall back across the residency boundary. Exhausted candidates on
  a local profile → error, never a remote retry.

### 3.2 Built-in default catalog

With no config file, four profiles exist (names are the stable default role
vocabulary the issue asks for; all overridable/extensible in config):

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
example maps as: implementer ≙ `fast`. (Alias `implementer` → `fast`? No —
one name per role; users who want other names define their own profiles.)

User-layer `[profiles.standard]` **replaces** the built-in `standard` wholesale
(per-name replace, no field-level merge — franken-profiles that mix layers are
undebuggable). Defining a new name adds to the catalog.

## 4. Selection and resolution

### 4.1 Which profile (per run)

Mirrors the harness precedence exactly (AGENTS-AI-FIRST-CLI §8):

1. flag `run create --profile <name>`
2. env `ORCHESTRATECTL_PROFILE` (empty = unset)
3. config `[profile] per_kind.<kind>` then `[profile] default`
4. — nothing: **no profile**; the legacy `--harness` path runs unchanged
   (byte-identical argv, `manifest.profile = None`).

`--profile` conflicts with `--harness` (clap `conflicts_with`) — a profile
already pins the harness per candidate; letting both through invites the two
values to disagree. When *any* layer selects a profile, profile resolution
owns the launch and the `[harness]` section is not consulted (it remains the
no-profile default path). Unknown profile name → hard error listing the
catalog, with the source layer named (same pattern as `invalid_harness`).

### 4.2 Which candidate (fallback)

Deterministic, observable, and narrow: walk `agents` in order; a candidate is
**available** iff `command[0]` resolves on PATH (`which`-equivalent). First
available candidate wins. Record, per run:

- `profile` + `profile_source` (flag/env/file/default)
- resolved `harness`, the full resolved command string
- `fallback_from`: the skipped candidates and per-candidate reason
  (`executable not found`) when index > 0.

All candidates unavailable → hard error naming every candidate and its reason
(for `secure`-class profiles this is the mandated fail-closed outcome).
PATH-presence is deliberately the only availability probe: deeper probes
(API reachability, model validity) are non-deterministic at create time and
belong to the agent's own startup failure, which the existing
`agent-pid-undiscoverable` / supervisor paths already surface.

### 4.3 Supervisor retry

The retry path re-launches from the **manifest's recorded resolved command**
verbatim — it does not re-run resolution. Same guarantee shape as today's
"retry never silently drops back to claude": what ran once is what retries,
even if config drifted meanwhile.

### 4.4 Automatic selection (planning)

No planner lives in the binary. Automatic matching of work to tiers is an
agent/skill-level concern; the binary's obligations are: (a) expose the
catalog machine-readably (`profile list --json`: name, description,
capability, local, candidates, availability), (b) resolve deterministically,
(c) record the outcome. Skills (stint-start, worktree-spinoff, fan-out) then
choose a profile by description/capability and pass `--profile`; a
data-sensitive task selects a `local` profile explicitly. A later issue can
add `run create --require-local` as a residency *constraint* flag if agent
discipline proves insufficient — noted, not built now.

## 5. Config layering (user + repo)

The second deliverable: a repo-local layer above the user file.

- **Paths:** user `~/.orchestratectl/config.toml` (existing, honors
  `$ORCHESTRATECTL_HOME`); repo `<repo-root>/.orchestratectl.toml` (single
  committable file; root = `git rev-parse --show-toplevel` of the CWD at
  `run create` time; absent git repo → no repo layer).
- **Missing file at either layer = empty layer** (exact current behaviour of
  the user file).
- **Merge semantics, deterministic and documented:**
  - scalars (`[profile] default`, `[harness] default`): repo replaces user;
  - maps (`[profile.per_kind]`, `[harness.per_kind]`, `[profiles.*]`): merged
    by key, repo entry replaces the user entry **wholesale** (a repo
    `[profiles.standard]` is complete — no field inheritance from the user's).
  - built-ins sit below both layers, same per-name replace rule.
- **Both layers parse strictly** (same `deny_unknown_fields` + key validation
  as today; unknown top-level sections tolerated for forward compat). A
  malformed repo file is a hard error naming the path — never silently
  skipped.
- **Residency hardening rule:** the repo layer may not redefine a name the
  user layer (or built-ins) declares `local = true` into `local = false`
  (load-time error). A checked-out repo must not be able to silently turn the
  user's confined role into an exfiltrating one. (The repo layer is otherwise
  trusted — it is the same trust domain as the code the agent will execute.)
- `config path` grows a per-layer view; `config show` gains `profile.*` rows.
  **Coordination:** `config-show-layered-view` (lane `surface`, seq 10) is
  sequenced BEFORE this issue (seq 20) and introduces the layered/raw
  inspection schema; this design's `config show` additions ride that schema
  rather than inventing a second layering presentation. If this lands first
  in calendar time, keep `config show` additions minimal (effective values
  only) and let that issue own the layered view.

## 6. Recording and surfaces

- `run.created` event data: `profile`, `profile_source`, resolved
  `agent_command` (single quoted string as launched), existing
  `harness`/`harness_source` (harness now sourced `profile` when a profile
  chose it — extend `HarnessSource` with `Profile`), and
  `agent_fallback_reason: Option<String>` (human-readable; present only when
  a fallback was taken).
- `Manifest` (all `#[serde(default)]`, legacy-readable): `profile:
  Option<String>`, `agent_command: Option<String>`,
  `agent_fallback_reason: Option<String>`.
- `run show` / `run list --json` surface them; `run show` human output prints
  `profile (fallback: <reason>)` when set.
- New read-only noun verb: `orchestratectl profile list [--json]` — the
  effective catalog with per-profile source layer (`built-in | user | repo`)
  and per-candidate availability. This is the dry-run observability surface
  the issue requires (resolution without spawning); a separate
  `profile resolve <name>` verb is redundant with `profile list`'s
  availability columns and is omitted unless review argues otherwise.

## 7. Slicing (may land as separate worktrees, in this order)

1. **A — resolver + per-run override (MVP):** `ProfileCatalog` (built-ins
   only), `--profile` flag + env, candidate resolution, shell-quoted `-a`
   passthrough, manifest/event recording, `run show` surfacing, smoke test of
   the full-command `-a` path. No config file changes. Lands the intake's
   primitive and kills the settings.json workaround.
2. **B — user-layer `[profiles]` + `[profile]` sections + `profile list`:**
   strict parsing/validation (incl. residency rule), per-kind defaults.
3. **C — repo layer:** `.orchestratectl.toml`, merge semantics, residency
   hardening rule, `config path`/`show` integration (coordinate with
   `config-show-layered-view`).
4. **D — skill/docs adoption:** bundled SKILL templates mention `--profile`
   where they mention `--harness`; AGENTS docs; issue the optional
   `--require-local` / escalation follow-ups if still wanted.

Slices B and C touch `config/mod.rs` + `harness/select.rs` + `run/create.rs`
(the issue's collision warning: cross-cutting into lifecycle territory —
do not parallelize with lifecycle-lane units; run the integrated gate).

## 8. Explicitly out of scope / deferred

- Raw `--model` / `--effort` flags on `run create` (open question, default:
  not built; revisit only with a concrete surviving need).
- In-binary automatic planner/tier matcher (agent-level concern, §4.4).
- Escalation verb ("retry one tier up") — enabled by `capability` ordering,
  designed elsewhere.
- Availability probes beyond PATH presence.
- Generalizing create.sh's `claude*|node*` PID walk for arbitrary local
  wrappers (documented risk, §2).

## 9. Test plan (maps to acceptance criteria)

- Pure resolver unit tests: precedence (flag/env/file/default), unknown name,
  empty candidate lists, fallback order + reasons, exhausted fallbacks,
  residency validation (local profile w/ remote candidate rejected at load;
  repo demoting local rejected), conflicts_with harness flag.
- Config layering tests: merge-by-name wholesale replace, scalar override,
  missing files, malformed repo file hard-errors, built-in shadowing.
- No-profile byte-identical argv test (create.sh spawn request without
  profile has `agent: None` exactly as today).
- Snapshot loop: `run create --help`, `profile list` envelopes, `run show`
  with profile fields, `config show` rows.
- One e2e: `run create --profile fast` records profile+command on the
  manifest and the stub create.sh receives the quoted `-a` string.
