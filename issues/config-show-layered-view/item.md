---
created: 2026-08-16
updated: 2026-08-17
type: improvement
status: open
priority: normal
epic: lifecycle-architecture-review
lane_seq: 10
lane: surface
---

# config show: layered/raw inspection that tolerates invalid values

## Description

## Description

Follow-up from the `/llm-review` + `/assess-findings` pass on `config-subcommand`
(3/3 reviewer consensus; see `history/review-config-subcommand.md` /
`history/assessment-config-subcommand.json`, findings F2/F3/F4).

`config show` today prints only the **effective resolved** harness value per key
with a single `source` (`env|file|default`). That satisfies AGENTS-AI-FIRST-CLI
§8's "effective resolved config" ask, but three related gaps make it a poor
*inspection/debugging* surface — exactly the job it exists for:

1. **Fails hard on an invalid file value (F2).** A typo like `[harness] default =
   "gpt"` makes `config show` exit with `invalid_harness` instead of showing the
   offending value. The tool dies precisely when the user runs it to debug the
   broken config. Precedent (`git config --list`, `kubectl config view`) shows raw
   invalid values.
2. **Env override hides file per-kind overrides (F3).** When
   `ORCHESTRATECTL_HARNESS` is set, every row reports `source: env`; a
   `[harness.per_kind] research = "claude"` override in the file is invisible, so a
   stale/shadowed config is undetectable and behavior silently changes when the env
   var is later unset.
3. **Invalid file value is laundered under env (F4).** Because harness-value
   validation lives in the resolver (which short-circuits on env), `config show`
   with `ORCHESTRATECTL_HARNESS=pi` and a bad file value *succeeds*, hiding the bad
   value — so `config show`'s strictness depends on ambient env state.

## Proposed direction

Decouple inspection from execution. Options to weigh in design:

- Add per-row `valid: bool` / `validation_error: Option<String>` and emit the raw
  value (with a `warnings[]` entry) instead of hard-failing — so `config show`
  never dies on the thing it inspects. Only unparseable TOML remains a hard error.
- Expose the layered stack per key (`configured_value`/`configured_source` +
  `effective_value`/`effective_source`, or a `layers`/`shadowed_by` list), so a
  caller sees both the file's `[harness]`/`[harness.per_kind]` contents AND what
  currently wins, and *why*.
- Make validity independent of ambient env (validate the file layer regardless of
  whether env shadows it).

This needs its own schema decision (bump `CONFIG_SCHEMA_VERSION`) and touches the
resolver/inspection boundary, so it is out of scope for the initial
`config-subcommand` landing.

## Also fold in (F6)

When the layered/warnings work lands, route the `--show-secrets` warning through
the JSON `warnings` envelope instead of the current `eprintln!` (currently
unreachable — no secret key exists — but the envelope-routing belongs with the
first real secret-valued key).

## Acceptance

- `config show` on a config with an invalid harness value shows the raw value +
  validity, not a hard error (TOML-parse errors may still fail).
- The file's `[harness.per_kind]` overrides are visible even when env shadows them.
- Validity of the file layer does not depend on whether env is set.
- Schema versioned; tests for each scenario incl. `ORCHESTRATECTL_HARNESS` set.
