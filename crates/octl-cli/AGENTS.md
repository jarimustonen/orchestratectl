# crates/octl-cli

The orchestratectl CLI binary. Verb-noun structure (`run create`, `node list`, `event tail`, `skill install`, etc.) per `AGENTS-AI-FIRST-CLI.md`. Bundled SKILLs live under `skills/<name>/SKILL.template.md` and are embedded via `build.rs` + `include_str!`.

## `skill install` dual-homes into pi.dev (`~/.pi/agent/skills/`)

A default `skill install` (plain, `--force`, or `--agent all`, without `--dest`) writes each skill's `SKILL.md` to **two** locations: the Claude Code path `~/.claude/skills/<name>/SKILL.md` AND a pi.dev mirror at `~/.pi/agent/skills/<name>/SKILL.md`. pi discovers skills from a per-skill dir just like Claude and invokes them as `/skill:<name>`; bare `/name` cross-references resolve via pi's injected available-skills list, so **only the install target differs — no body/link rewrite**. The pi mirror is byte-identical to the Claude `SKILL.md`. Because pi uses a **per-skill directory like Claude** (not a flat prompts dir like codex), **companion resources mirror into pi too** — as plain siblings of the pi `SKILL.md` (e.g. `stint-start`'s `AGENTS-EXECUTION-DAG.md`), byte-identical and with no link rewrite (the body's `](AGENTS-EXECUTION-DAG.md)` sibling link and `stint-handoff`'s cross-skill `](../stint-start/…)` link resolve against the mirrored sibling exactly as under Claude). Mirroring the companion is what keeps a skill that STOPS on a missing companion (`stint-start`) from aborting under pi — issue `support-pi-dev`. The Claude write is unchanged, and the pi mirror **carries no in-dir provenance marker** (the pi corpus stays a pure body mirror; companions are tracked in the out-of-band record instead). Skipped for a custom `--dest` (caller-managed) and for `--agent codex` alone (codex is not a claude-format consumer). See `src/skill.rs` `cmd_install`.

### pi mirror lifecycle — out-of-band provenance (`pidev-pi-skill-lifecycle`)

The pi dir may hold no `.orchestratectl-managed` marker, so its lifecycle is keyed on a single **out-of-band** JSON record at `<ORCHESTRATECTL_HOME or ~/.orchestratectl>/state/pi-installed-skills.json` — `{ schema_version, skills: { <name>: { sha256, cli_version, companions: { <file>: sha256 } } } }`. The `companions` map (serde-default, so records written by an older binary stay readable — the schema stays v1) records each companion sibling we mirrored beside the skill's `SKILL.md`, keyed filename → content hash. On every pi write, the record is union-merged: SKILL.md writes refresh `sha256`/`cli_version` in place (preserving tracked companions), companion writes file under their owning skill's `companions` — so a targeted single-skill install never forgets the rest of the managed set. It is the SOLE authority for two decisions, both of which **never touch a pi dir we did not write** (a user's hand-authored pi skill is never recorded):

- **Prune** (gated on the same full-catalog `--force` as the Claude dir prune): a de-registered pi mirror the record names is deleted only if its on-disk bytes still hash to the recorded value (proof it is our unmodified copy). A **diverged** copy (user-edited since we wrote it) is left in place and dropped from the record (`pi_mirror_diverged`); a symlink or squatting dir is never followed/deleted. The `SKILL.md` we wrote is removed first, then each recorded **companion** sibling that still hashes to its recorded value (`pi_companion_pruned`; a diverged/user companion is left → `pi_companion_diverged`), then the now-empty per-skill dir best-effort (`remove_dir`, non-recursive — a user sibling, or a surviving diverged companion, is preserved).
- **`doctor` drift** — `skill.sync.<name>.pi` (older/newer/unparseable/edited via the recorded hash) + `skill.orphan.<name>.pi` (de-registered but still on disk), plus the **companion** arms `skill.sync.<name>.pi.<file>` (forward drift of each bundled companion vs the embedded body — content-identity in-sync signal, same as the codex companion check) and `skill.orphan.<name>.pi.<file>` (a companion the record tracks that the binary no longer bundles). All gated on the record being non-empty so a host that never dual-homed into pi stays 0-warn. ALL pi arms are **advisory — no autonomous `FixAction`** (unlike the Claude older-drift arm): the fix applier runs `skill install <name> --force`, which dual-homes and would force-overwrite the Claude copy too, so autofixing pi drift could silently downgrade a deliberately newer/edited Claude copy. Symmetric with the codex checks. Mirrors the Claude/codex checks in `src/doctor/checks/skill.rs`.

Integrity: the record is **loaded and validated before any file is written** (`load_pi_provenance_for_write`) — a corrupt or future-schema record **fails the install closed** (never silently laundered to empty and overwritten, which would erase tracking for every mirror). Record-sourced skill names are validated as single path components before any `join`→`remove_file` (`is_simple_skill_name`), and the empty-dir cleanup is bound to `<pi-root>/<name>/`. The record read-modify-write is **unlocked** (parity with the Claude/codex markers): concurrent `skill install` runs can lose one another's additions, so mutation commands are not meant to run concurrently.

## Insta snapshot test loop

Many integration tests in `tests/` use `insta` for envelope / help / skill-catalog snapshots. After any CLI surface change (added flag, renamed verb, new bundled skill, edited error message), running `cargo test -p octl-cli` produces `.snap.new` files for every diff. Accept them with:

```bash
find crates/octl-cli/tests/snapshots -name "*.new" -exec sh -c 'mv "$1" "${1%.new}"' _ {} \;
cargo test -p octl-cli
```

Often takes **2–3 rounds** because the first accept-pass reveals further drifts only visible once earlier snapshots settle. Re-run the loop until `cargo test -p octl-cli` is green.

**A workspace version bump is a snapshot change too.** The `version_*` snapshots (`envelope_snapshots__version_{text,json,jsonl}.snap`) bake in the literal crate version, so bumping `[workspace.package] version` in `Cargo.toml` stales them exactly like a CLI-surface edit — run the accept loop above (or `cargo insta test --accept -p orchestratectl`) after the bump, or `cargo test` goes red. `scripts/check-version-snapshots.sh` (also a CI job) fails loudly on a version/snapshot mismatch as a fast pre-publish guard; the release-mechanics obligation is in `OSS-RELEASE.md` alongside the CHANGELOG-finalize step.

## Skill catalog: edit pin-test explicitly

The bundled-skill list is hardcoded in `tests/skill.rs::skill_list_json_pins_catalog_shape`. When adding or removing a bundled skill, **also edit that test's `vec![...]` literal** — the snapshot loop above will NOT catch it (different test). Forgetting this gives a single failing test on the next run.

## Test-spawn hygiene

Integration tests that exercise `run create`'s production path spawn real `orchestratectl supervise` subprocesses. The shared `tests/common/mod.rs` `TestHome` fixture reaps them on Drop — use it (`let home = TestHome::new()`) for any new test that goes through `bin(&home)`. Tests that skip the fixture and use raw `TempDir` leak supervisor processes; the `supervise-test-teardown-leak` issue covers the recovery + reaper logic in detail.

After `cargo test -p octl-cli` finishes, `pgrep -lf "orchestratectl.*supervise"` from the workspace `target/debug/` path should return nothing. Any survivor is a missing-fixture bug in the new test.

## End-to-end spinoff harness

`tests/e2e_spinoff.rs` drives ONE full autonomous-spinoff round-trip on every run — `run create --kind spinoff --headless` (real detached supervisor) → live stub agent → `run merge` → supervisor rolls the run up to `done`, tears down, and exits — and asserts the canonical event sequence (`run.created`, `node.created`, `supervisor.started`, `node.report`, `run.status`, `supervisor.exited`) + terminal manifest. It is the CI gate for the merge / cleanup / supervisor-lifecycle paths.

It stubs the two shell-out boundaries through the production override hooks — `OCTL_CREATE_SH` (`run::spawn`) and `OCTL_MERGE_SH` (`run::merge`) — and points `TMUX_BIN`/`GIT_BIN` at nonexistent paths so the supervisor's tmux liveness probe reads `Unknown` (PID liveness governs → the live stub agent stays `Alive` until the merge terminalizes the node) and every teardown step is a lenient no-op. The stub agent (a `sleep`) is reaped by an `AgentGuard` on drop, panic-safe, alongside `TestHome`'s supervisor reaper. No new test-only hook was needed; adding lifecycle scenarios (failure path, concurrency, reattach) means extending this file.

## `run create --harness` (worker harness selection)

`run create --harness <name>` picks which code harness launches the worker in its
tmux pane — `claude` (default) | `pi` | `aider` | `claude-deepseek` (the
`harness::KNOWN_HARNESSES` registry; same names as `harness bakeoff`). The
mechanism is deliberately narrow: the resolved harness maps to a **workmux agent**
(`harness::workmux_agent`) forwarded to `create.sh` as `--agent <name>` (→
`workmux add -a`). `claude` maps to `None` — no `--agent` is passed, so a default
spawn's create.sh argv is byte-identical to before the flag existed. The
supervisor/merge/report path is harness-agnostic, so a non-claude worker rides the
exact same lifecycle.

Precedence (AGENTS-AI-FIRST-CLI §8), resolved per run in `harness::select`:
**flag `--harness` > env `ORCHESTRATECTL_HARNESS` > `config.toml` `[harness]`
(per-kind override, then section default) > built-in default (`claude`)**. The
config file (`config.toml` under the resolved home — `$ORCHESTRATECTL_HOME` or
`~/.orchestratectl`; `config.rs`) is the tool's first config-file layer; a user
points `[harness.per_kind] research = "pi"` while `code` stays claude (per-kind
keys are validated against the known run kinds at load, so a typo fails loudly). The resolved harness is folded onto `manifest.harness` (from
the `run.created` event, which also carries `harness_source` for provenance) and
surfaced on `run show` / `run list --json`. `harness::select::resolve_with` is the
pure, unit-tested resolver; `resolve` supplies the ambient config+env.

NOT wired: the `CodeHarness::run_chunk` in-process contract (bakeoff/pipeline) is a
*synchronous* seam and is unrelated to the detached-tmux worker launch — do not try
to route `run create` through it. `--harness pi` requires a `pi` agent configured
in workmux.

## pi worker-prompt translation shim (`harness::prompt`)

A worker's prompt is the `--task` brief materialized verbatim to
`<run-dir>/prompt.md` and handed to the agent via `create.sh` → `workmux add -P`.
Those briefs are Claude-Code-flavored (Skill/Agent tools, sub-agents, MCP,
`/worktree-*` / `/llm-*` slash commands) — none of which the `pi` agent has (pi is
AGENTS.md-native). `harness::prompt::worker_prompt_preamble(harness, kind, run_id)`
returns an optional operating-note **preamble** that `run create` (`create.rs`,
`resolve_prompt_file`) prepends before the brief when a harness needs the
translation. The preamble maps the Claude-only references to their bash/CLI
equivalent (the `/worktree-merge` close → the exact `orchestratectl run merge`
bash; `/llm-review` / sub-agents → skip) so a pi worker can complete the loop.
Because the preamble is generated in-process — unlike the static bundled SKILLs —
it templates the **exact run id** into the closing call (quoted heredoc), so the pi
worker runs a literal `orchestratectl run merge <run-id>` with no
`ls ~/.orchestratectl/runs | grep` discovery to get wrong.

Scope is deliberately **narrow: only `(pi, research)` is translated end-to-end**
(the issue's done bar — one autonomous kind working). Every other `(harness, kind)`
pair returns `None`, so the claude path is byte-identical and un-shimmed pi kinds
(spinoff, code, …) are left untranslated as an explicit follow-up — extending the
shim is a one-arm change in `worker_prompt_preamble`. A `--prompt-file` is used
as-is when there is no preamble (caller keeps ownership); with a preamble the
derived prompt is written into the run dir so the caller's file is never mutated.

## Shelling out to `claude -p` (pipeline spec/verify + harness adapters)

`claude -p --output-format json` does **not** emit a single result object — it
emits a **sequence** of JSON messages, and the FIRST is a `{"type":"system",
"subtype":"init", …}` banner (session_id, skills, tools, mcp_servers, model, …).
When parsing the model's answer, select the message whose **`type == "result"`**
and read its `.result` field; never take "the first JSON object" (that's the init
banner, and you'll parse the banner as the model's output). This cost three live
`pipeline run` iterations to diagnose — the spec stage kept reading the init banner
and the plan was always "missing acceptance". Fixed in
`src/pipeline/live/providers.rs` (`extract_result_text`); any new code that shells
to `claude -p` for a structured answer must apply the same selection rule.
