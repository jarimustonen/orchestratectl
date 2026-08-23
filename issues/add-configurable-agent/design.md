# Design: configurable agent profiles and effective worker policy

**Status:** proposed for joint human review; design only, not implementation authorization
**Issue:** `add-configurable-agent`
**Supersedes:** the 2026-08-17 v2 implementation-ready wording
**Depends on:** `worker-telemetry-protocol/design.md`

This revision joins profile selection to the telemetry protocol's worker-control
boundary. It stops at `worker-control-plane-review`. The historical slices A–D,
production profile or telemetry changes, adapter work, and new implementation
issues are not authorized by this document.

## 1. Decisions and invariants

The control plane resolves one requested policy into one effective launch policy.
The control plane keeps these dimensions separately represented; policy may
tighten one from another, but never substitutes one for another. Names such as
`secure` are catalog vocabulary, not evidence:

- **capability tier:** declared `fast < capable < ultra-capable` (the label
  `fast` denotes the lowest product tier, not measured latency);
- **data residency:** user/built-in declared `local | remote`;
- **interaction mode:** explicit `interactive` or the current `autonomous`
  lifecycle default;
- **worker permission requirement:** a named, closed operation set, with a
  `restricted-local` ceiling derived from local residency;
- **telemetry requirement:** derived from interaction mode; and
- **support evidence:** separately probed telemetry support and separately
  verified permission-enforcement support.

Binding rules:

1. Callers select capability names/profiles, not vendor model IDs. Raw `--model`
   and `--effort` remain an unresolved escape-hatch question.
2. Residency is orthogonal to capability. Selection, fallback, and escalation
   never cross an explicit residency constraint. A request whose effective
   residency is `local` never falls back to remote, regardless of profile name.
3. The initially expected local profile is weak. Local profiles in protocol v1
   receive the restricted worker-permission ceiling, including no worker or
   worktree spawning. This is launch policy, not prompt advice.
4. A repository may select a profile by name but may never define executable
   profiles. Executable definitions are built-in or user-authored only.
5. No launch mutates `~/.pi/agent/settings.json`, Claude settings, or any other
   global harness configuration. Model and adapter selection are per-spawn.
6. Autonomous eligibility depends on mechanically checked launch evidence, not
   a harness-name assumption or user assertion. Telemetry support must be
   advertised by a trusted, compatible adapter and pass the telemetry design's
   bounded probe. Permission enforcement is distinct evidence from a trusted
   harness launch-composition capability; it is not advertised by the telemetry
   adapter. Pi plus the approved telemetry adapter is the only expected initial
   autonomous combination. Claude has no adapter and is explicit-interactive
   only. Capability tier and residency remain declarations, not proofs of model
   quality or network confinement.
7. Telemetry is diagnostic evidence. Missing, stale, invalid, settled, or
   shutdown telemetry cannot imply success, failure, progress, wedging, retry,
   terminalization, `run wait` satisfaction, or teardown. `run merge` remains
   the only success truth; told `worker.exited` and the existing confirmed-dead
   grace remain crash paths.
8. Fallback is deterministic, constraint-preserving, and create-time only.
   Retry reuses the recorded effective candidate; it never silently advances a
   fallback or tier.
9. Every decision is explainable as requested policy, effective policy, and a
   structured provenance/decision trace. Candidate argv is public user data;
   telemetry bearer secrets, capability paths, and private probe diagnostics are
   never part of that public record.
10. With no profile selection, legacy harness precedence and base candidate
    argv remain unchanged wherever the resolved interaction is eligible. The telemetry
    migration still makes unsupported Claude autonomous creation fail; that is
    an explicit control-plane transition, not profile fallback or an invisible
    compatibility shim (§10).

## 2. Trust boundaries

### 2.1 Definition and selection ownership

Profile definitions contain executable launch material and therefore live only
in:

1. the built-in catalog; and
2. the user config at `~/.orchestratectl/config.toml`, redirected by
   `$ORCHESTRATECTL_HOME`.

The repository file `<repo-root>/.orchestratectl.toml` may contain selection
keys only. Reject `[profiles.*]`, `[harness]`, executable argv, adapter paths,
permission grants, residency reclassification, and telemetry-support assertions
in that file. Otherwise a checkout could arrange arbitrary same-user execution
before the worker or any sandbox exists.

The user is the authority for a candidate's declared residency. `local` means a
user declaration that payloads remain on the machine, not an egress sandbox or
binary-verified fact. Orchestratectl truthfully enforces non-crossing selection;
it cannot prove that arbitrary user-authored software does not use a network.

### 2.2 Adapter and telemetry ownership

The harness-neutral protocol, capability issue/revocation, attempt fencing,
bounded sample, read views, and eligibility checks belong to orchestratectl.
The pinned pi package owns translation from documented pi lifecycle events. A
profile points to an adapter requirement; it does not make the adapter trusted.
Trust comes from the probe, immutable package/version/integrity identity,
operator-configured trusted root, explicit extension launch, and probe-to-launch
checks specified by the telemetry design.

Profiles do not declare telemetry requirement in v1: autonomous derives
`required`, interactive derives `optional`. A profile cannot assert
`support = "available"`. Support is recorded evidence returned by the resolved
adapter probe. Static `harness = "pi"`, an existing sample, or an argv string
containing an extension name is never proof.

Adapter references resolve only through a user/operator-owned registry under
`$ORCHESTRATECTL_HOME`; repository config cannot define or redirect it. Each
entry pins the harness family, probe executable identity, trusted package root,
exact package version/integrity, and extension entry identity. The registry
entry and probe executable must be authenticated against that trust policy
*before* the probe executes; probe output negotiates compatibility but cannot
establish the trustworthiness of the executable that produced it. The exact
registry schema and the meaning of a writable trusted package root require the
joint cross-design decision in §11.

### 2.3 Restricted local workers

Protocol v1 assigns every `residency = "local"` profile the restricted permission
ceiling. At minimum it denies `spawn_workers`, covering `run create`, child
fan-out/spinoff workflows, and equivalent worktree-launch facilities. The exact
closed operation set is not yet approved; v1 does not expose arbitrary per-profile
permission subsets.

Permission enforcement belongs to trusted harness-specific launch composition
owned by orchestratectl's control plane, not to the diagnostic telemetry adapter
or its protocol probe. A conforming launch capability must construct and bind the
canonical harness executable, final argv, ambient-extension policy, and
model-visible tool/skill set so worker spawning and any trivial process escape
are absent. Trusted extension-internal execution used only for the fixed
telemetry endpoint is distinct from model-visible process capability. Merely
omitting instructions, setting an environment marker, asking a weak model not to
spawn, or identifying an opaque wrapper artifact is not enforcement.

No harness currently has accepted evidence for that permission capability.
Therefore every candidate requiring `restricted-local` is ineligible in this
design, interactive or autonomous, until the joint review defines a useful
operation set and accepts a harness launch mechanism that enforces it. The
built-in local role is consequently a fail-closed placeholder today, not a
shippable security feature. This is a capability restriction, not an OS sandbox;
same-user arbitrary code remains outside the threat claim.

## 3. Data model

### 3.1 Profile definition

A profile is a stable capability role and an ordered candidate list:

```toml
# user layer only
[profiles.capable]
description = "General-purpose high-capability work"
capability = "capable"
residency = "remote"
permission_set = "standard"
agents = [
  { harness = "pi", command = ["pi", "--model", "openai/example:high"], adapter = "pi-telemetry-v1" },
  { harness = "claude", command = ["claude", "--model", "example"] },
]

[profiles.secure]
description = "Small, unambiguous work whose payload must remain local"
capability = "fast"
residency = "local"
permission_set = "restricted-local"
agents = [
  # Illustrative desired mapping; currently ineligible because no accepted
  # restricted-local launch capability exists.
  { harness = "pi", command = ["pi-gemma"], adapter = "pi-telemetry-v1" },
]
```

Normative validation:

- profile names use a documented stable identifier grammar and are unique;
- `description`, capability, residency, and permission set are required;
- local profiles must use `restricted-local`; remote profiles may also request
  that stricter set;
- `agents` is ordered, contains at most eight candidates, and may be empty only
  for an actionable built-in placeholder;
- `harness` names a known protocol family; `command` is a bounded, non-empty
  **base candidate argv** array, never a shell string; and adapter references
  name operator-approved registry entries, not repository paths;
- capability and residency belong to the profile and are inherited by every
  candidate; a candidate probe does not independently verify either declaration;
- unknown fields/enums, contradictory permission claims, duplicate candidate
  identities, and invalid adapter references fail before selection.

`command[0]` may differ from the harness name (`pi-gemma` under the pi protocol),
but an opaque wrapper is not eligible for autonomous or restricted operation
unless a trusted harness launch capability can bind its behavior, not merely its
file identity. Per-spawn base argv is transported through the existing launch
chain using one property-tested quoting boundary; the final control-plane launch
plan may add adapter and restriction material (§6). Candidate argv and the final
launch plan are public metadata. V1 performs no secret detection/interpolation;
credentials remain in inherited environment or existing harness-owned credential
stores, which orchestratectl does not mutate. Users must not put credentials in
argv. A future profile-specific secret-reference surface requires separate
review.

### 3.2 Built-in catalog

The stable vocabulary is `ultra-capable`, `capable`, `fast`, and `secure`.
Built-ins must be useful without copying Jari's personal fleet or pinning dated
vendor IDs. The exact built-in candidate mapping is a joint-review decision.
The safe minimum is:

- remote tiers may map to the current default harness/model aliases;
- `secure` exists as local + fast + `restricted-local`, but may have an empty
  candidate list and fail with “configure a trusted local candidate” because
  there is no universal local model or restriction-capable adapter;
- a new custom name adds a wholesale definition; custom definitions never merge
  field-by-field with another layer.

Stable built-in names that carry a promise have fixed semantic metadata. User
configuration may wholesale-replace their candidate list and description but
may not change capability, residency, or permission requirement. Thus a role named
`secure` cannot be redefined as remote or unrestricted. Custom user role names
remain wholesale definitions. The exact reserved-name set and aliases such as
historical `expert`, `standard`, and `implementer` are joint-review decisions;
the machine capability rank remains canonical.

### 3.3 Requested policy

The resolver first creates a normalized `RequestedWorkerPolicy` without choosing
an executable:

```text
profile_request: explicit/configured name | legacy harness path
interaction: autonomous | interactive
profile_constraints (when named): declared capability and residency,
  derived permission requirement
telemetry_requirement: required | optional
```

Interaction remains explicit lifecycle policy: `--interactive` has CLI
provenance; its absence produces the current `autonomous` lifecycle default with
`lifecycle-default` provenance. It is never inferred from run kind, profile,
harness, or telemetry. Autonomous derives telemetry `required`; interactive
derives `optional`. Selecting a local profile derives the `restricted-local`
ceiling; contradictory inputs error rather than winning by precedence.

A named profile supplies the requested capability/residency/permission
constraints; provenance retains the definition source and derivation. V1 performs
no constraint-based automatic profile matching and does not infer data
sensitivity from brief text. Skills may choose a named role explicitly, but the
binary receives and records that name. Automatic tie-breaking and tier
escalation require a later reviewed policy.

### 3.4 Effective policy

A successful resolution produces immutable launch policy:

```text
policy DTO version; profile name, normalized definition digest and catalog
  identity (or legacy-none)
resolved declared capability/residency and derived permission requirement,
  each with an assurance label
interaction and telemetry requirement
stable candidate identity and index; harness; base argv; canonical executable
  path/content identity
final launch-plan identity, argv/environment additions and model-visible tool set
adapter protocol/version/package/integrity/entry identity, or unsupported
eligibility result and mechanically checked evidence
ordered fallback decisions
```

The telemetry capability-file path/secret is attempt-scoped launch material, not
stored in this policy's public representation. Manifest projections may store
only the telemetry control hash/generation defined by the telemetry protocol.

## 4. Configuration and command precedence

### 4.1 Definition merge

1. load built-ins;
2. parse the user file strictly and wholesale-replace/add profiles by name;
3. parse repo selection only.

Missing files are empty layers. Malformed present files hard-fail with path and
field. New profile/repo sections carry an explicit config schema version and
bounded counts/string/argv sizes; old files containing only existing `[harness]`
remain valid. Repo root is `git rev-parse --show-toplevel` from create CWD;
outside git there is no repo layer. Unknown top-level sections may follow the
repository's forward-compatibility convention, but unknown keys inside owned
sections fail. User config and adapter-registry ownership/symlink/permission
requirements are part of the trust decision in §11; warnings must not be
misrepresented as confinement of arbitrary same-user code.

### 4.2 Selection authority and precedence

Repository selection is a policy influence even though it cannot introduce new
argv: it can move payloads remote, raise the permission ceiling, or increase
cost. Therefore a repo selection is considered only when user-owned policy has
authorized that profile for repository selection. The joint review must choose
whether authorization is a global allowlist, a per-repository grant, or stricter
residency/permission ceilings. Until that choice exists, repo selection fails
closed as `repo_selection_not_authorized`; it cannot silently override user
policy. Direct CLI selection is the user's explicit authority and is unaffected.

After an authorized repo input is admitted, specificity-first candidate order is:

1. CLI `--profile` or legacy `--harness`;
2. non-empty `ORCHESTRATECTL_PROFILE` or `ORCHESTRATECTL_HARNESS`;
3. admitted repo `profile.per_kind.<kind>`;
4. user `profile.per_kind.<kind>` or `harness.per_kind.<kind>`;
5. admitted repo `profile.default`;
6. user `profile.default` or `harness.default`;
7. existing built-in harness default.

Within one source and specificity (`CLI`, environment, user per-kind, or user
default), non-empty profile and harness mechanisms conflict. Across precedence
levels, the higher input wins and the lower one is retained in the decision
trace as shadowed; an unknown higher profile errors and never falls through.
Empty/whitespace environment values are absent. Repo config cannot contain
harness keys. The joint review must confirm both repository authority and the
specificity-first ordering; authority is the security decision, not merely the
position of `repo default` in this table.

## 5. Deterministic resolution and fallback

Resolution is a pure ordered decision followed by bounded probes:

1. normalize requested policy and reject contradictions;
2. choose the configured/named profile or legacy harness path;
3. validate the selected profile's declared constraints and derived permission
   ceiling once; reserved-name semantic mismatch rejects the profile;
4. inspect at most eight candidates in declared order, cheap static checks first
   (shape, registered trust identity, executable resolution), external probes
   last, memoized per adapter/executable identity for this resolution;
5. reject each candidate with structured reason(s) if its canonical executable
   is absent/unbindable, adapter support fails, or required permission
   enforcement is unavailable;
6. select the first candidate satisfying every hard constraint;
7. if none qualifies or the bounded aggregate probe budget is exhausted, fail
   before worker materialization and report every tried/untried decision.

Every resolution has a finite candidate, probe-count, output-size, and aggregate
wall-clock budget. Exact numeric bounds are part of the public contract and must
be settled with the telemetry probe budget before implementation; no unbounded
candidate tail is silently ignored.

For `autonomous`, candidate eligibility requires all of:

- a harness telemetry adapter that truthfully advertises `worker_telemetry`;
- separately verified harness launch-composition support for every required
  permission restriction;
- the non-interactive, closed-stdin, time/output-bounded probe and protocol
  negotiation from the telemetry design;
- a trusted, pinned, unchanged package/entry identity and explicit launch plan;
- satisfaction of residency and permission constraints.

For `interactive`, telemetry is optional. V1 deterministically enables it when
the selected candidate has a trusted compatible adapter: it issues attempt
authority and launches the adapter exactly as for required telemetry. If the
adapter is absent/incompatible, that fact does not reject an otherwise
unrestricted interactive candidate; it launches without telemetry and records
`support=unsupported|incompatible, sample=absent`. Claude therefore qualifies
only when interaction is explicitly interactive. An autonomous chain
`pi → claude` rejects Claude as `telemetry_unsupported`; it never weakens the
requirement. A local chain never reaches a remote candidate. No fallback crosses
profiles.

PATH presence remains only one availability fact. It does not prove provider
health, credentials, model access, restriction enforcement, or telemetry
support. Runtime provider/network/quota failure is not create-time fallback and
uses existing told worker-exit behavior. It never derives a retry decision from
a missing or stale lease.

Fallback output is a structured ordered list: candidate index and stable
candidate identity, decision (`selected | skipped | untried`), and precise reason
codes such as `executable_missing`, `launch_unbindable`,
`telemetry_unsupported`, `adapter_not_configured`, `adapter_untrusted`,
`adapter_incompatible`, `protocol_incompatible`, `probe_timeout`,
`probe_output_invalid`, `permissions_unenforceable`, or
`probe_budget_exhausted`. Capability/residency mismatch is a profile-level
constraint error, not a candidate probe result. Human messages derive from the
same DTO and distinguish unsupported, untrusted, incompatible, and transient
probe failure.

## 6. Launch, telemetry identity, and retry

### 6.1 Launch transaction

A successful create freezes effective policy before materializing the worker.
Dry-run runs only authenticated, bounded external probes and mutates no
orchestratectl run, worktree, capability, or pane. It is a point-in-time,
non-binding eligibility snapshot: external probe code is executed and cannot be
promised globally side-effect-free; a later create re-resolves and can observe
drift.

Real launch then:

1. constructs and records a control-plane launch plan from base candidate argv,
   canonical executable, pinned adapter entry, ambient-extension policy,
   permission tool plan, and environment additions;
2. immediately revalidates the identities in that plan; drift fails before
   attempt authority is created;
3. when a compatible adapter was selected (required or optional), creates the
   owner-only, no-symlink attempt capability outside the worktree exactly as
   specified by the telemetry design;
4. launches the final plan, including `OCTL_TELEMETRY_CAPABILITY` only for that
   attempt, and records launch/worker-exit through existing told paths;
5. on failure after capability creation but before a viable worker exists,
   revokes/removes that capability and records a typed launch-aborted phase.

The executable/adapter checked must be those actually launched. Canonical launch
identity covers resolved executable path/content, symlink/script/interpreter or
shim identity as applicable, adapter entry/integrity, and the composed plan; a
PATH token plus a snapshot is not pinning. There remains a same-user TOCTOU
window between final check and process execution; the design does not claim to
eliminate it. Create errors name candidate and phase and never try a later
candidate after partial materialization.

### 6.2 Attempt/incarnation fencing

Profiles do not duplicate telemetry identity. Each retry increments the node's
absolute attempt, revokes/removes the old capability, and issues a new secret and
generation. Within an attempt, pi session reload/new/resume/fork uses the
telemetry design's random client-instance open, lease epoch, and strictly
increasing client sequence. A new incarnation atomically makes the previous
sample ineligible until its first update. Old attempt/generation/epoch writes
are rejected under the run lock.

The hybrid lease remains 30-second refresh / 90-second currentness (values
returned by `open`). These are diagnostic freshness bounds, never fallback,
stall, health, or retry timers.

### 6.3 Retry policy

Retry copies the recorded requested policy, effective candidate, and final
launch-plan identity. It does not reload definitions, re-run catalog selection,
advance candidates, cross capability/residency, or change interaction or
permissions. Identity drift intentionally makes an old run non-retryable; an
ordinary retry never adopts an updated package. A future explicit operator
re-resolution would create a new requested-policy decision and provenance event,
never hide migration inside `node.retry`.

Retry uses a prepare/commit boundary aligned with existing reducer invariants:

1. without mutating attempt state or holding the run lock across a subprocess,
   revalidate the recorded executable, launch-composition, adapter, protocol,
   and permission evidence;
2. reacquire the exclusive run lock and compare the node/attempt/status to the
   preflight snapshot; a mismatch restarts or fails preflight without mutation;
3. if preflight failed, preserve current authoritative attempt/work and record no
   new telemetry authority;
4. if it succeeded, atomically record the new absolute attempt/generation and
   revoke old telemetry authority before any new-attempt sample can qualify,
   then issue the fresh capability and spawn from the recorded final plan;
5. capability/spawn failure after that transition is a typed failed launch
   attempt with work preserved, never a rollback that revives old authority.

Immutable create policy, immutable bounded per-attempt revalidation/launch
evidence, and disposable advisory sample are separate records. Pre-enforcement
`legacy-unrecorded` runs keep the retry rule recorded by their legacy state; the
migration gate applies to new creates and does not retroactively invent adapter
identity. The joint review must confirm that grandfathering and exact typed
retry transitions against current reducer semantics.

“Try the next candidate after runtime failure” and “retry one capability tier
up” remain separate future policy decisions. They require explicit new requested
policy and provenance, never telemetry inference.

## 7. Provenance and observability contract

The bounded, explicitly versioned policy DTO uses field-level provenance rather
than one lossy `profile_source`. Public origins use normalized source class and
key, not an absolute home/config path:

```text
cli flag | environment variable name | repo config key+repo identity |
user config key+config identity | built-in catalog+version |
lifecycle default | derived constraint
```

Each field records selected normalized value, origin, derivation chain, and
relevant conflicting/shadowed input. The resolver also records normalized
profile-definition digest, stable candidate/launch-plan identity, candidate
index as display context, bounded fallback decisions, and timestamped probe
evidence identity. Assurance labels distinguish `declared_by_profile`,
`declared_by_user_or_builtin`, `derived_policy`, `resolved_identity`,
`probed_support`, and `verified_launch_capability`; the DTO never presents model
quality or local egress as proved.

The same versioned policy DTO flows through:

- **config inspection:** raw/layered selection values and effective catalog
  definitions, clearly separating selection from executable definition;
- **`run create --dry-run --output json`:** requested policy, normalized hard
  constraints, effective candidate, eligibility/probe result, launch argv, and
  full fallback trace, labeled as a no-run snapshot;
- **launch:** an immutable internal launch plan derived from that DTO;
- **`run.created` / manifest:** requested and effective policy plus provenance;
  legacy records lacking the versioned fields remain readable, with no bearer
  secret or capability path;
- **`run show`:** the recorded create-time policy (never recomputed from current
  config/PATH) beside current telemetry dimensions `requirement`, `support`, and
  `sample`;
- **retry:** original policy plus attempt number and fresh telemetry generation;
  revalidation results append attempt evidence but do not rewrite create-time
  provenance.

`run show` keeps immutable policy, immutable per-attempt evidence, and current
telemetry separate. `support` means a trusted probe advertised the protocol and
the recorded launch plan attached that entry; it does not mean samples are
flowing. For example, autonomous pi can show `support=available, sample=stale`
while status is unchanged; interactive Claude shows `requirement=optional,
support=unsupported, sample=absent`. Deleting an adapter later does not rewrite
launch-time support; retry evidence reports revalidation separately. Text must
not call `agent_active` “progress”, `tool_running` healthy, or `settled`
complete. Policy rendering includes profile, declared capability/residency,
permission requirement, interaction, candidate/fallback, and provenance; legacy
JSON uses an explicit versioned `legacy-unrecorded` variant rather than invented
fields.

`profile list --output json` is static catalog inspection: definitions, source,
attributes, and ordered candidates, with executable presence clearly labeled as
a point-in-time hint. It does not run adapter probes by default or imply
eligibility. Any future explicit live-probe mode is bounded and named. Only
create dry-run applies command/config constraints and probes eligibility.

Base/final argv is observable and unredacted public user data; v1 performs no
secret detection or safety label. Capability paths, bearer secrets, secret
hashes, absolute private config paths, and private probe diagnostics are
excluded from public policy DTOs. Profile, candidate, argv, description,
fallback, derivation, and total persisted-policy sizes all have finite schema
limits so config cannot create unbounded events/manifests.

## 8. No-profile compatibility and migration

Profile adoption alone must not alter legacy selection:

- absent profile input/config follows the existing flag > env > user per-kind >
  user default > built-in Claude harness precedence;
- the legacy **base candidate argv** and prompt behavior are preserved;
  telemetry-capable autonomous pi necessarily receives separately recorded
  control-plane additions in its final launch plan;
- legacy manifests remain readable and display `policy: legacy-unrecorded`
  rather than invented provenance.

The telemetry design adds a separate unavoidable eligibility rule: after the
human-approved enforcement gate, every autonomous launch requires truthful
telemetry support. The current no-profile default may resolve to Claude and must
then fail before materialization with an actionable instruction to configure a
telemetry-capable profile or choose explicit interactive mode. It must not
silently add `--interactive`, pretend telemetry exists, or fall back by harness.
Explicit interactive Claude preserves today's base command path. Legacy
`--harness pi` autonomous creation also requires an approved adapter after the
gate; profile absence is not a telemetry exemption.

No production release may ship a half-state that requires an adapter package not
yet available through the approved operator installation. The joint review must
choose one atomic migration gate (release boundary or explicit temporary feature
gate), its diagnostics, and rollout timing. A temporary gate may delay
*enforcement* but must not label unsupported workers autonomous-eligible.

## 9. Verification obligations for an eventual implementation

These are acceptance tests for later approved work, not slices or authorization.

### Pure resolver and config

- every selection precedence pair, source path/key, empty env, both-env conflict,
  and unknown profile source;
- wholesale profile replacement and repo-definition rejection;
- all capability × residency × interaction × permission × telemetry combinations,
  especially local→remote, every currently unenforceable restricted candidate
  (interactive and autonomous), and autonomous→unsupported negatives;
- deterministic ordered fallback reasons and exhausted errors;
- local permission ceiling cannot be loosened by user/repo/CLI;
- no-profile legacy resolution remains exact;
- argv hostile-character round trip through the sole quoting boundary.

### Boundary/integration

- spawn spy proves exact argv, exact pinned adapter entry, ambient extension
  behavior, permission tool set, and no global settings mutation;
- concurrent different-profile spawns remain isolated;
- dry-run creates no run/worktree/pane/capability, respects aggregate probe
  bounds, and matches create only under an unchanged dependency snapshot;
- manifest/event/show carry identical requested/effective policy and provenance;
- interactive Claude succeeds with honest unsupported telemetry; interactive pi
  with a compatible selected adapter receives optional attempt authority;
- autonomous Claude and unadapted pi fail before materialization;
- local restricted worker cannot access worker/worktree spawning through any
  exposed tool/skill/process route claimed by the enforcement boundary;
- fallback-resolved retry pins the same candidate even if PATH/config changes;
- retry rotates attempt capability and rejects stale attempt/incarnation writes.

### Telemetry negative invariants

Reuse the telemetry design's full failure-injection matrix. For every absent,
stale, corrupt, clock-unreliable, settled, shutdown, sequence-conflict,
incarnation-takeover, and endpoint-failure case, assert no synthetic report,
status change, retry, fallback, cleanup, merge, or run-wait completion. Test with
stripped ambient PATH and fake clocks/endpoints where applicable.

### Launch-chain smoke test

Separately prove exact base-argv transport, final control-plane additions,
prompt injection under hostile quoting, and PID discovery for every supported
candidate family. A lucky spawn is not all four. Test PATH/symlink/interpreter
identity drift between resolution, final check, spawn, and retry. Custom wrappers
that cannot satisfy launch binding or PID discovery remain loud failures unless
a later reviewed mechanism replaces those limitations.

## 10. Ownership and non-goals

Orchestratectl owns policy/config resolution, eligibility, immutable launch plan,
recorded provenance, telemetry control, and read surfaces. The external pi
adapter owns truthful public-event translation, probe identity, restriction-aware
launch integration, and lease sender. Stint/end-to-end work owns deliberate
adapter installation, rollout, and integrated autonomous validation. Workers do
not install packages or mutate global tools.

Not authorized or in scope here:

- production implementation of profiles, telemetry, adapters, or skills;
- filing or scheduling replacement slices A–D or new telemetry candidates;
  telemetry candidates already filed by the completed telemetry design remain
  untriaged and unauthorized, and this review does not authorize adding
  profile-dependent launch/permission work to them;
- automatic brief classification/data-sensitivity inference;
- runtime-failure fallback, automatic tier escalation, or telemetry-based retry;
- repo-defined commands or trust-grant machinery;
- treating `local` as verified network confinement;
- importing pi internals, EventBus, process managers, session logs, or private
  extension state;
- raw model/effort flags without the joint human decision.

The v2 sections saying “implementation starts”, “may land”, or “file at slice D”
are superseded. The only next step is `worker-control-plane-review`.

## 11. Human decisions for `worker-control-plane-review`

The combined review must explicitly decide the blockers first:

1. **Restricted-local operation set and feasibility:** What exact model-visible
   tools/skills remain? Does pi provide a per-spawn launch boundary that removes
   shell/process and worker-spawn escape without mutating global settings while
   leaving useful editing ability? Until yes, every restricted/local candidate
   remains ineligible, not merely autonomous-local.
2. **Launch-composition ownership:** Confirm telemetry translation/probing and
   permission enforcement as separate capabilities. Decide whether trusted
   harness-specific composition lives in orchestratectl or a separately
   versioned launch contract; do not silently extend the telemetry probe.
3. **Adapter registry/trust:** Define its user/operator-owned location, write
   authority, trusted-root and file-permission semantics, trust-before-probe,
   exact package/integrity identity, and binding to the resolved harness runtime.
4. **Repository authority:** May repo selection move a user's payload remote,
   raise its permission ceiling, or select any user-defined executable profile?
   Choose global allowlist, per-repo grant, or non-weakenable user constraints,
   then confirm specificity-first precedence.
5. **Reserved role semantics:** Which built-in names have fixed
   capability/residency/permission metadata, which candidates can users replace,
   and are historical aliases retained?
6. **Optional telemetry:** Confirm the proposed deterministic rule: interactive
   candidates use telemetry when a trusted compatible adapter is selected, and
   otherwise launch honestly without it.
7. **Retry and migration transaction:** Confirm hard pin/fail-on-drift, the
   prepare/commit attempt ordering, legacy/pre-gate grandfathering, the future
   explicit re-resolution escape hatch, and the one atomic release/feature gate
   that makes telemetry mandatory for new autonomous creates.
8. **Public launch metadata and credentials:** Confirm that base/final argv and
   package integrity are public durable metadata, capability material is not,
   and v1 relies on inherited/harness-owned credentials rather than profile
   secret interpolation.
9. **Built-in fleet mapping:** Choose useful vendor-neutral defaults without
   personal fleet leakage and accept that local remains a fail-closed placeholder
   until restriction enforcement exists.
10. **Raw escape hatch:** Reject `--model`/`--effort` for v1 (recommended), or
    require equivalent policy/provenance rules.

Automatic matching, brief/data-sensitivity inference, runtime fallback, and tier
escalation are deferred rather than human blockers for v1. The review must also
record any amendment needed to the telemetry design—notably trust-before-probe,
legacy requirement rendering, and confirmation that its adapter probe gains no
permission-broker role—so the two documents cannot be approved inconsistently.
Only after explicit approval may the review define and file production work in
dependency order.

## 12. Acceptance check for this design phase

- Capability, residency, interaction, permissions, and telemetry remain separate
  dimensions with explicit assurance: declarations are not presented as proofs,
  and derived constraints are recorded.
- Autonomous eligibility depends on truthful adapter evidence; Claude is
  explicit-interactive only.
- Local fallback cannot become remote; every weak/local candidate fails
  eligibility until mechanical restriction evidence is accepted.
- Attempt/generation and incarnation/epoch/sequence fencing are reused without
  duplicating telemetry truth in profiles.
- Requested/effective policy and provenance are specified from config and CLI
  through resolver, dry-run, launch, manifest, show, and retry.
- Missing/stale telemetry has no outcome, progress, retry, fallback, or teardown
  authority.
- Legacy profile-free selection remains compatible except for the explicit,
  human-gated telemetry eligibility transition.
- Unresolved product and enforceability decisions are named for joint review.
- This document grants no production or issue-slicing authorization.
