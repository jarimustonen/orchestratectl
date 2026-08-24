# Design: configurable agent profiles

**Status:** approved product direction; design only
**Issue:** `add-configurable-agent`
**Depends on:** `worker-telemetry-protocol/design.md`

## 1. Product decision

A profile is a user-owned name for an ordered list of agent commands plus two
plain attributes: capability tier and data residency. The resolver records what
the caller requested, which candidate it selected, and why earlier candidates
were skipped.

Agents have full normal rights. V1 has no permission sets, operation sets,
`restricted-local` ceiling, tool filtering, command sandbox, trusted launch
composition, or mechanical no-spawn eligibility gate. The existing local
`secure` role is usable with normal rights. Its behavior depends on its
user-configured model and instructions; orchestratectl makes no no-spawn or
restricted-local guarantee. Stronger enforcement may be designed later if a
real need justifies it, but it is not a v1 requirement or claim.

The remaining hard boundaries are:

1. an explicit local request never falls back to remote;
2. autonomous operation initially requires pi with the telemetry adapter;
3. Claude remains explicit-interactive until it has a real adapter;
4. fallback never weakens residency or required telemetry;
5. executable profile definitions live only in user-owned configuration; and
6. requested and selected choices, plus a useful fallback reason, are visible.

Telemetry remains advisory as defined by the telemetry design. It may inform the
calling agent's judgment but cannot itself create success, failure, retry,
settlement, or teardown truth inside orchestratectl.

## 2. Configuration ownership

Executable profile definitions exist only in user config:

```text
$ORCHESTRATECTL_HOME/config.toml
# normally ~/.orchestratectl/config.toml
```

Repository config at `<repo-root>/.orchestratectl.toml` may select a profile by
name, including per-kind selection, but must reject profile definitions,
commands, argv fragments, adapter paths, and residency reclassification. This
prevents a checkout from defining what executable the user runs without adding a
separate trust/grant subsystem.

Direct CLI selection is explicit user authority. No launch mutates pi, Claude,
or other global harness settings. The selected command is invoked per spawn.

The user is the authority for a profile's residency declaration. `local` means
that the configured agent is intended to keep payloads on the machine;
orchestratectl enforces only that fallback does not cross to a `remote` profile
or candidate. V1 does not claim network confinement or verify model quality.

## 3. Profile model

A profile contains stable metadata and an ordered candidate list:

```toml
[profiles.capable]
description = "General-purpose high-capability work"
capability = "capable"
residency = "remote"
agents = [
  { harness = "pi", command = ["pi", "-e", "/Users/me/.pi/extensions/octl-telemetry.js", "--model", "example"], telemetry = "worker-v1" },
  { harness = "claude", command = ["claude", "--model", "example"] },
]

[profiles.secure]
description = "Local model for contained tasks"
capability = "fast"
residency = "local"
agents = [
  { harness = "pi", command = ["pi-gemma", "-e", "/Users/me/.pi/extensions/octl-telemetry.js"], telemetry = "worker-v1" },
]
```

Normative validation:

- profile names use one documented identifier grammar and are unique;
- `description`, `capability`, `residency`, and `agents` are required;
- capability is `fast | capable | ultra-capable`;
- residency is `local | remote`;
- `agents` is ordered, non-empty, and bounded to eight candidates;
- `harness` is `pi | claude` in v1;
- `command` is a bounded, non-empty argv array, never a shell string;
- `telemetry`, when present, is the known value `worker-v1` and is valid only
  for pi in v1; and
- unknown fields, invalid enums, duplicate candidates, and contradictory values
  fail before selection.

Candidate argv is ordinary user-visible launch metadata. V1 has no interpolation
or secret-reference feature, so users must keep credentials out of argv and use
existing environment or harness credential stores.

There is no built-in executable profile catalog. Documentation may recommend the
role names `ultra-capable`, `capable`, `fast`, and `secure`, but commands and
candidate definitions always come from user config. In this design, the existing
`secure` role means a user-owned profile of that name, such as the definition
above; prior illustrative mappings are not shipped executable definitions. An
operator may keep or copy that mapping into user config without gaining or
losing any enforced restriction.

## 4. Selection precedence

Selection uses the first non-empty source in this order:

1. CLI `--profile` (or the existing explicit legacy `--harness` path);
2. `ORCHESTRATECTL_PROFILE` (or legacy `ORCHESTRATECTL_HARNESS`);
3. repository `profile.per_kind.<kind>`;
4. user `profile.per_kind.<kind>` or legacy `harness.per_kind.<kind>` alias;
5. repository `profile.default`;
6. user `profile.default` or legacy `harness.default` alias; and
7. otherwise an actionable missing-profile error.

Legacy `--harness`, `ORCHESTRATECTL_HARNESS`, and an existing user
`harness.default` remain selection aliases only: each names a matching
user-owned profile definition and cannot synthesize built-in argv. A legacy
harness alias also requires every candidate in the named profile to use that
harness. New creates through these aliases obey the same autonomous eligibility
rule and requested/selected output as `--profile`; an alias that resolves to no
user definition errors with guidance to configure a profile. Existing stored
legacy runs remain readable, but no new create bypasses user-owned command
configuration.

At one precedence level, simultaneous profile and legacy harness inputs are an
error. An unknown selected profile errors; it does not fall through. Repository
selection needs no allowlist or trust-grant machinery because it can only name a
user-defined executable profile. The repository cannot supply or rewrite the
profile's commands or residency.

Interaction remains explicit lifecycle policy. `--interactive` selects
interactive; its absence keeps the current autonomous default. It is never
inferred from run kind, profile name, harness name, or telemetry state.

## 5. Resolution and fallback

Resolution is deterministic:

1. normalize the requested profile and interaction mode;
2. load and validate the named user profile;
3. inspect candidates in listed order;
4. skip a candidate whose executable is unavailable or whose static eligibility
   does not meet the requested interaction and telemetry condition;
5. select the first eligible candidate; or
6. fail with the bounded list of skipped reasons.

For autonomous use, v1 eligibility is exactly:

- `harness = "pi"`; and
- `telemetry = "worker-v1"`.

This declares that the user-configured pi command launches the approved adapter.
Orchestratectl does not add package attestation, trusted roots, probe executables,
ambient-extension suppression, operation restrictions, or a second launch
contract. If the command or adapter later fails, existing launch/worker failure
disclosure reports it.

For explicit-interactive use, pi and Claude candidates are eligible. Telemetry is
optional: a pi candidate declaring `worker-v1` gets the launcher identity needed
by the adapter; another candidate runs honestly without samples. Claude therefore
shows optional/unsupported telemetry until a real adapter exists.

Residency belongs to the selected profile and all its candidates. Fallback stays
inside that profile, so it cannot cross local to remote. Exactly one reason code
is recorded per skipped candidate, evaluated in this order:

1. `executable_missing`;
2. `autonomous_harness_unsupported` when an autonomous candidate is not pi; and
3. `telemetry_unsupported` when an autonomous pi candidate lacks `worker-v1`.

Thus autonomous Claude is `autonomous_harness_unsupported`. Post-selection
launch failure is not a skip reason and never appears in fallback; it uses the
existing agent failure disclosure unchanged.

Fallback occurs only before worker materialization. Runtime provider, model,
quota, adapter, or process failure uses existing failure handling. Retry reuses
the recorded selected candidate and does not reload config or advance fallback.

## 6. Requested and selected visibility

A successful create records a compact, versioned choice:

```json
{
  "profile": "capable",
  "selection_source": "cli",
  "interaction": "autonomous",
  "capability": "capable",
  "residency": "remote",
  "requested_harness": null,
  "selected": {
    "candidate_index": 0,
    "harness": "pi",
    "command": ["pi", "-e", "/Users/me/.pi/extensions/octl-telemetry.js", "--model", "example"],
    "telemetry": "worker-v1"
  },
  "fallback": []
}
```

If fallback occurs, each skipped row contains the candidate index, harness, and
one reason code. This is enough to explain “requested capable; selected its
second pi candidate because candidate 0 was missing.” A failed create returns the
same compact record with `selected: null` and a populated fallback list;
`profile`, `selection_source`, and `interaction` remain visible. V1 does not
record field-level derivation graphs, config digests, assurance labels, package
identities, shadowed-input trees, or launch-plan provenance.

The same compact shape appears in `run create --dry-run --output json`,
`run.created`/manifest data, and `run show`. Text output plainly states the
requested profile or harness, selected profile/candidate/harness, and fallback
reason where relevant. `run show` displays the recorded create-time choice; it
does not recompute against current config.

Dry-run validates config and executable availability but creates no run,
worktree, pane, or telemetry sample. It is a point-in-time preview. Profile list
shows user definitions and executable presence without claiming provider health,
model access, telemetry freshness, or local network confinement.

Legacy records remain readable as `selection: legacy-unrecorded`; no detailed
history is invented. The existing simple agent failure disclosure is accepted
and does not need additional provenance machinery.

## 7. Launch and telemetry flow

### Autonomous pi

1. Resolve the requested profile and preserve its residency.
2. Skip candidates that are not pi with `telemetry=worker-v1`.
3. Record the first eligible candidate and compact fallback reasons.
4. Launch its exact user-configured argv, adding only the existing run/node/
   attempt environment needed by the harness-neutral telemetry endpoint.
5. Show last told activity and freshness when samples arrive.
6. Keep canonical completion, failure, retry, and teardown on their existing
   explicit paths.

### Explicit-interactive Claude

1. Record explicit interactive selection and the chosen Claude candidate.
2. Launch its exact user-configured argv without fabricated adapter support.
3. Show `requirement=optional`, `support=unsupported`, `sample=absent`.
4. Wait for explicit `run merge` or `run cancel`; telemetry absence changes
   nothing.

### Local `secure`

The existing user-owned local `secure` role participates in the same rules and
is usable with normal rights. Autonomous use requires a pi candidate with the
adapter; explicit-interactive use does not. Orchestratectl makes no “restricted,”
“sandboxed,” or “cannot spawn” claim. Its behavior comes from the configured
model and instructions, not enforced policy.

## 8. Verification obligations

An eventual implementation should cover:

- strict user-definition parsing and repository-definition rejection;
- every selection-precedence pair and unknown-profile failure;
- argv round trips without shell parsing;
- local profiles never crossing to remote fallback;
- autonomous selection accepting only configured pi+adapter candidates;
- Claude succeeding only when explicit-interactive;
- fallback reasons matching the candidate actually skipped;
- requested/selected data matching across dry-run, manifest, and `run show`;
- retry pinning the recorded candidate despite config/PATH changes;
- no global harness-settings mutation; and
- all telemetry negative invariants from the telemetry design.

## 9. Non-goals and boundary

V1 does not add permission or operation models, restricted-local enforcement,
sandboxing, package trust machinery, adapter probes, repo trust grants,
automatic brief/data-sensitivity classification, runtime-failure fallback,
automatic tier escalation, model-quality verification, network confinement,
secret interpolation, or repository-defined commands.

This document does not implement production code, file profile implementation
issues, or schedule existing telemetry candidates. The proposed post-approval
split and assessment live in
`../worker-control-plane-review/integration-review.md`.
