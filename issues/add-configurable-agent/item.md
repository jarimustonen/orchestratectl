---
created: 2026-08-17
updated: 2026-08-17
type: feature
reporter: jari
status: open
priority: normal
labels: [configuration, agents]
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
