# crates/octl-cli

The orchestratectl CLI binary. Verb-noun structure (`run create`, `node list`, `event tail`, `skill install`, etc.) per `AGENTS-AI-FIRST-CLI.md`. Bundled SKILLs live under `skills/<name>/SKILL.template.md` and are embedded via `build.rs` + `include_str!`.

## `skill install` dual-homes into pi.dev (`~/.pi/agent/skills/`)

A default `skill install` (plain, `--force`, or `--agent all`, without `--dest`) writes each skill's `SKILL.md` to **two** locations: the Claude Code path `~/.claude/skills/<name>/SKILL.md` AND a pi.dev mirror at `~/.pi/agent/skills/<name>/SKILL.md`. pi discovers skills from a per-skill dir just like Claude and invokes them as `/skill:<name>`; bare `/name` cross-references resolve via pi's injected available-skills list, so **only the install target differs — no body/link rewrite**. The pi mirror is byte-identical to the Claude `SKILL.md`. Vendored filter: **only `SKILL.md` is mirrored** — companion resources (e.g. `stint-start`'s `AGENTS-EXECUTION-DAG.md`) stay Claude-only, matching homebase `dotfiles link`. The Claude write is unchanged, and the pi mirror carries no provenance marker and is not pruned (prune/marker logic stays Claude-scoped). Skipped for a custom `--dest` (caller-managed) and for `--agent codex` alone (codex is not a claude-format consumer). See `src/skill.rs` `cmd_install`.

## Insta snapshot test loop

Many integration tests in `tests/` use `insta` for envelope / help / skill-catalog snapshots. After any CLI surface change (added flag, renamed verb, new bundled skill, edited error message), running `cargo test -p octl-cli` produces `.snap.new` files for every diff. Accept them with:

```bash
find crates/octl-cli/tests/snapshots -name "*.new" -exec sh -c 'mv "$1" "${1%.new}"' _ {} \;
cargo test -p octl-cli
```

Often takes **2–3 rounds** because the first accept-pass reveals further drifts only visible once earlier snapshots settle. Re-run the loop until `cargo test -p octl-cli` is green.

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
in workmux; the Claude-flavored bundled-SKILL prompt translation for a pi worker is
a separate follow-up (`run create --harness` lands the launch selector only).

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
