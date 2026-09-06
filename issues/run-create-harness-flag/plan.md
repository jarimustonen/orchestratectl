# plan — run create --harness

## Architecture finding (drives the design)

There are **two unrelated execution models** in the tree; the issue's "route the
worker launch through the CodeHarness adapter" conflates them:

1. **`CodeHarness` / `AgentLaunch` seam** (`crates/taskfleet-cli/src/harness/*`) — a
   *synchronous in-process* `run_chunk(req) -> ChunkResult` contract. Shells the
   tool blocking, reads git state, returns a structured result. Used **only** by
   `harness bakeoff` (+ the unfinished code-pipeline). It cannot BE the worker
   launch: the supervisor model needs a **detached PID in a tmux pane** it polls,
   not a blocking in-process call.

2. **`run create` worker launch** — fire-and-forget: `run/create.rs` →
   `spawn.rs::run_create_sh` → external `create.sh` (homebase) →
   `workmux add -a <agent>` launches the agent in a tmux pane; a separate
   `supervise` process watches its PID and consumes the terminal `node.report`.

`create.sh` **already** has an `--agent`/`-a` passthrough to `workmux`; `spawn.rs`
just never sets it (so workmux uses its default agent = claude). **The clean
wiring is: `--harness <name>` → forward `--agent <name>` to create.sh.** The
`CodeHarness` registry supplies the canonical valid-name list. The supervisor
path is already harness-agnostic, so "merge+report through the same supervisor
path" is satisfied for free.

## Scope landed here

1. `run create --harness <name>` (validated against the harness registry:
   `aider, claude, claude-deepseek, pi`).
2. Precedence **flag > env `TASKFLEET_HARNESS` > config file > built-in
   default (`claude`)**, with a per-kind config override (so config can default
   `research`/`spinoff` to `pi` while `code` stays `claude`). Built-in per-kind
   default is `claude` everywhere (safe rollout; claude stays default).
3. Persist resolved harness on the manifest (`Manifest.harness`, additive
   `#[serde(default)]`), fold from the `run.created` event, surface in
   `run show` / `run list --json` (`ManifestView` + `RunSummary`) and the event
   log (`harness`, `harness_source`).
4. Thread harness → `SpawnRequest.agent` → `create.sh --agent`. Map
   `claude → None` (byte-identical to today), every other harness → `Some(name)`.
5. Rollout: the forwarding is uniform + low-risk; claude stays the built-in
   default and the interactive driver. Any kind can be defaulted to `pi` via
   config.

## Deferred (follow-up issues)

- **Skill/Agent-tool translation shim (§4).** Making a *pi* worker actually
  complete a bundled Claude-flavored SKILL workflow is an AGENTS.md-native prompt
  translation — substantial, not an taskfleet-code change.
- **`config` subcommand** (`config path`, `config show --json`) per
  AGENTS-AI-FIRST-CLI §8 — useful but its own CLI surface + snapshot suite.
- **workmux `pi` agent preset** — `run create --harness pi` forwards `--agent pi`;
  workmux must have a `pi` agent configured in `.workmux.yaml` (homebase concern).

## Files

- new `crates/taskfleet-cli/src/config.rs` — config file load (`~/.taskfleet/config.toml`).
- new `crates/taskfleet-cli/src/harness/select.rs` — `resolve_harness`, `HarnessSource`, workmux-agent map.
- `harness/mod.rs` — `KNOWN_HARNESSES`, `DEFAULT_HARNESS`, `workmux_agent`, `pub mod select`.
- `run/mod.rs` — `--harness` flag + dispatch.
- `run/create.rs` — resolve, record in `run.created`, pass agent to spawn.
- `run/spawn.rs` — `SpawnRequest.agent` + `--agent` forwarding.
- `taskfleet-core/schema.rs` — `Manifest.harness`.
- `taskfleet-core/reducer.rs` — fold `harness`.
- `run/dto.rs` — surface harness + wire-shape tests.
- tests + insta snapshots.
</content>
