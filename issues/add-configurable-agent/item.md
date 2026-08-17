---
created: 2026-08-17
updated: 2026-08-17
type: feature
reporter: jari
status: open
priority: normal
labels: [configuration, agents]
lane: surface
lane_seq: 20
---

# Add configurable agent profiles

## Description

## Goal

Add user-configurable agent profiles so `orchestratectl` can select named capability roles rather than hard-coding a single harness/model choice. Profiles describe the available model fleet and let orchestration planning choose an appropriate agent for each run.

## Configuration

Load a user-level configuration from the user's home-directory configuration location, with a repository-local configuration layered above it as an override. Define and document deterministic precedence, validation, missing-file behaviour, and whether profile definitions merge by role/name or replace the lower layer.

The configuration should define stable default role names that `orchestratectl` commands can accept directly. Each profile must include:

- a stable profile/role name;
- a human-readable description of intended capability and suitable work;
- one preferred executable invocation;
- zero or more fallback invocations;
- harness, model, and effort arguments needed to launch the agent.

## Example personal profile set

The initial design must support a configuration equivalent to:

- `expert`: a highly capable model: `claude --model fable --effort high`.
- `standard`: general-purpose high-capability work: preferred `pi --model gtp-5.6-sol --effort high`; fallback `claude --model opus5 --effort high`.
- `implementer`: routine implementation and basic work: preferred `pi --model gtp-5.6-terra --effort medium`; fallback `claude --model sonnet5 --effort medium`.
- `secure`: a security-conscious role: `pi-gemma`.

Use these as illustrative user configuration, not hard-coded defaults that force every installation to have the same models.

## Behaviour

- Commands should be able to request a profile by its default name.
- Planning and automatic model selection should use profile descriptions and availability, so work can be matched to the available capability tiers and fallbacks.
- Selection must be observable: dry-run/JSON/run metadata should state the requested profile and the resolved executable, harness, model, and fallback reason when applicable.
- Invalid profile definitions, unknown profile names, unavailable preferred agents, and exhausted fallbacks must fail with actionable errors.
- Preserve current configuration and CLI behaviour for installations that do not opt into profiles.

## Acceptance criteria

- Document the configuration schema, paths, precedence, role naming rules, and resolution algorithm.
- Add parsing and strict validation for user and repository configuration layers.
- Add profile-aware command input and resolver behaviour with deterministic fallback handling.
- Surface the selected profile and resolved agent details in machine-readable output and run metadata.
- Cover precedence, validation errors, explicit selection, automatic selection, fallback, and no-profile backward compatibility with tests.

## Comments

### 2026-08-17T08:14:16Z · @orchestrator

Laned to `surface` (seq 20) 2026-08-17, sequenced AFTER config-show-layered-view — both rework the config surface. COLLISION WARNING for whoever schedules this: despite the `surface` lane it is genuinely cross-cutting — it also touches harness::select (profile→executable resolution) and the run-create path (accepting a profile, recording the resolved profile/model/fallback in run metadata). Run-create is `lifecycle` territory. Do NOT run this in parallel with any lifecycle-lane unit. This is exactly the lane-misprediction shape that broke integrated main twice before (see TODO.md KEY LEARNING on disjoint lanes being a prediction, not a guarantee).

### 2026-08-17T08:37:42Z · @orchestrator

Config location, verified 2026-08-17 (Jari asked at handoff). The EXISTING user-level file is `~/.orchestratectl/config.toml`, redirected by $ORCHESTRATECTL_HOME when set; `orchestratectl config path` reports it (exists: true on Jari's machine today). It currently holds only a [harness] section with per-kind overrides, resolved by harness::select with precedence flag > env ORCHESTRATECTL_HARNESS > file per-kind > file section default > built-in 'claude'. IMPORTANT for the implementer: the REPO-LOCAL layer this issue asks for does NOT exist yet — there is exactly one config layer today. Do not assume a layering mechanism is already in place to extend; it has to be built, including the precedence, merge-vs-replace semantics, and missing-file behaviour the issue body calls for. Reuse harness::select's resolver rather than re-implementing resolution (config show already reuses it verbatim, which is why an env override honestly shadows every row).

### 2026-08-17T10:44:28Z · @orchestrator

DESIGN STEER FROM JARI (2026-08-17 handoff) — capability names are the interface, raw model IDs are at most an escape hatch.

Jari's preference, in his words: the system should work from a small set of capability-level names — roughly 'ultra-capable / capable / fast / security-conscious' — rather than callers naming raw models. Those names are configurable, AND both the role set and the mapping ship with sensible defaults, so the tool is useful out of the box with no config file present.

Implication for the profile design already described in this issue: the four example profiles (expert / standard / implementer / secure) should be read as CAPABILITY TIERS, not as Jari's personal fleet baked in. The model IDs in the example set are illustrative configuration, never built-in defaults that pin every installation to one vendor's line-up. (See also the sibling issue audit-no-user-specifics — shipping a personal fleet as a built-in default is exactly the leak class it exists to catch.)

OPEN QUESTION, deliberately NOT decided: whether to also expose raw `--model` and `--effort` flags on run create. Jari: 'voimme myös lisätä --model ja --effort liput, mutta en ole näistä varma.' Do not treat this as approved. Argument for: an escape hatch for a model that has no tier yet, and it is the smallest possible MVP. Argument against: it is a second, competing way to say the same thing, and it invites callers to hardcode vendor model IDs into scripts and briefs — precisely what the capability layer exists to prevent. Recommended resolution: build the capability layer first and add raw flags only if a concrete need survives it.

Note that the merged intake's own use case ARGUES FOR the capability framing rather than against it: escalating a stuck unit from gpt-5.6-terra to gpt-5.6-sol is 'retry one tier up', which a tier vocabulary expresses directly and portably, while a raw --model escalation hardcodes two vendor IDs into the orchestration logic.

### 2026-08-17T10:44:46Z · @orchestrator

MERGED IN from intake-feature-orchestratectl-d0c82ab27c9d (closed duplicate 2026-08-17, filed from ossctl stint #22). It adds three requirements this issue did not carry:

1. PER-RUN OVERRIDE IS THE PRIMITIVE. `run create` today exposes only `--harness claude|pi`, with no way to pick a worker model for ONE spawn. This is the mechanism the profile system needs anyway, and it is a legitimate MVP SLICE that may land FIRST — deliver the per-run selection + manifest recording before the full layered profile config, if that sequencing is useful. Do not treat the whole issue as all-or-nothing.

2. THE RESOLVED CHOICE MUST BE RECORDED AND VISIBLE. The selected profile and the resolved executable/harness/model (plus any fallback and its reason) go on the run manifest and are surfaced by `run show`. Without this, which model a worker ran on is unrecoverable after the fact — which is precisely what made the reported escalation hard to reason about.

3. WHAT IT REPLACES — the current workaround is genuinely unsafe. The pi harness reads its model from the GLOBAL ~/.pi/agent/settings.json, so per-spawn model choice today requires temporarily rewriting that file before `run create` and restoring it after the agent starts. Two named defects: a concurrent spawn inherits the wrong model (racy), and the restore is easy to forget (leaves the user's global settings mutated). Any design that still requires mutating global harness settings to select a model has NOT solved this.

Concrete syntax datum: pi accepts `--model "provider/id:<thinking>"` on its CLI, so passthrough is viable without touching settings.json.

Reported context (why this is an orchestration lever, not a nicety): a terra worker gave up twice on a large semantic seam; a sol worker finished it in one pass.

### 2026-08-17T10:48:17Z · @orchestrator

DESIGN CONSTRAINT (Jari, 2026-08-17) — `secure` is ORTHOGONAL to the capability ladder, not a rung on it. Read this before designing selection or fallback.

The other roles (ultra-capable / capable / fast) sit on ONE axis: how capable the model is. `secure` does not. Its defining property is that it **runs locally**, so the data never leaves the machine, which is what makes it safe for personal data, API key CONTENTS, credentials, and similar. Jari's note: it is deliberately LOW capability — that is an accepted cost, not a defect. Two axes, not four tiers:

  capability:   fast  <  capable  <  ultra-capable
  data residency: local (confined)  |  remote (leaves the machine)

WHY THIS MATTERS — the fallback rule, and it is a safety rule, not a preference:

**A `secure` profile MUST NOT fall back to a remote model, ever.** The whole point of selecting it is that the payload stays on the machine; a fallback that silently escalates to a cloud model would exfiltrate exactly the data the choice was protecting, and would do so at the moment things are already going wrong (the preferred agent was unavailable). Concretely:
- Fallback chains must be constrained to the same residency class. A local profile's fallbacks must themselves be local.
- If no local fallback is available, the correct behaviour is to **fail with an actionable error**, NOT to degrade to a remote agent. 'Exhausted fallbacks' is already in this issue's acceptance criteria — for `secure` it must be reached rather than escaped.
- 'Retry one tier up' escalation (the terra→sol case in the merged intake note) is a CAPABILITY-axis operation and must never cross the residency axis.

WHY THIS ALSO CHANGES AUTOMATIC SELECTION. This issue currently says planning should match work to 'capability tiers and fallbacks'. That is insufficient: capability is a property of the TASK'S DIFFICULTY, whereas residency is a property of the DATA THE TASK TOUCHES. A purely capability-driven planner will never pick `secure` correctly, because a task involving credentials is not thereby a hard task — it is often trivial. So the design needs a way to express task data-sensitivity (a flag on run create, a profile request, or a marker in the brief) that constrains the residency axis independently of the capability choice. Do not fold sensitivity into the capability ranking.

Naming implication: a role set presented as a single ordered ladder (expert/standard/implementer/secure) invites exactly the wrong mental model. Consider expressing residency as a separate attribute of a profile (e.g. `local: true` / a residency class) rather than leaving it implicit in one role's name, so the constraint is machine-checkable at fallback-resolution time instead of relying on the implementer remembering which role is special.

Example given: `secure` → `pi-gemma` (local).




