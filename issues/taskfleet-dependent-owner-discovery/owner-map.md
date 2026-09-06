# Taskfleet dependent-owner map (ADR 0002 E1)

_Evidence captured 2026-09-06. Source checkout: `jarimustonen/taskfleet` at `c05238c1b4ad6341d17c9e15192c11892674a805`._

## TL;DR

The active convergence has **eight repository owners**, with Homebase split into two serialized slices because it owns both the package fleet and the intake-facing workflow text:

1. **Homebase** owns the `orchestratectl` fleet package unit, old Homebrew/release channel, `~/.orchestratectl/config.toml` link, binary-owned skill installation, Taskfleet source checkout/tmux identity, and machine convergence on `gertrud`, `hauis`, `haapa`, and currently-unreachable `brunhild`.
2. **intakectl** owns the Haapa intake subprocess integration, the `orchestratectl` executable default/override, run-home diagnostics, target-repository key and clone resolution, and the deployed `intakectl.service` / `intakectl-drainer.service` pair.
3. **issuectl** owns the shipped `/issue-intake` templates which still declare `orchestratectl` as the prerequisite for `/worktree-bug-analysis`; its generated copies must land before downstream copies are refreshed.
4. **3dbear-monorepo**, **blog**, **deutschpad**, and **project-canon** own tracked active workflow or generated-skill copies containing the old command/path/repository identity.
5. **Shipshape (`ossctl` source coordinate)** owns one current operating-policy reference to the old fleet-repository name. Its old-package release fixtures and `octl-core` architecture analogies are intentional fixtures/history, not dependencies.

There is **no external Cargo dependency on `octl-core` or `orchestratectl`** in the maintained source set. There is also **no installed or source-present production pi worker-telemetry adapter**: Taskfleet owns only the public contract and endpoint. The contract id `orchestratectl.worker-telemetry-adapter` and `OCTL_*` worker/notify protocol variables are stable and must not be renamed.

Taskfleet 0.6.1 and its canonical channels are live: crates.io carries `taskfleet-core`, `taskfleet`, and the bounded `orchestratectl` wrapper at 0.6.1; GitHub Release `v0.6.1` carries Taskfleet-only binaries plus the non-installing old-installer stub; `jarimustonen/homebrew-taskfleet` carries `taskfleet` 0.6.1; and the old tap contains only `tap_migrations.json` mapping `orchestratectl` to `jarimustonen/taskfleet/taskfleet`. No E1 command changed a repository, machine, state root, installation, skill, service, secret, tap, or public ref.

## Search inventory and method

### Maintained-source boundary

The inventory enumerated every immediate Git repository under `/Users/jari/Sources` with:

```sh
find /Users/jari/Sources -mindepth 1 -maxdepth 2 -type d -name .git -prune
```

For each repository, the search used `git grep` (tracked files only) for:

```text
orchestratectl | taskfleet | octl-core | taskfleet-core
ORCHESTRATECTL_* | TASKFLEET_* | OCTL_*
.orchestratectl | .taskfleet
orchestratectl.worker-telemetry-adapter
homebrew-orchestratectl | homebrew-taskfleet
jarimustonen/orchestratectl | jarimustonen/taskfleet
```

Excluded from the active count were `history/`, `target/`, vendored/package caches, generated snapshots, immutable evidence, fixtures, and `CHANGELOG.md`. Open issue directories were searched separately; closed-issue archaeology was not used to create active findings. Matches in tests and comments were inspected rather than blindly replaced. Every repository classified below as an owner had its root `AGENTS.md`, `README.md`, and relevant config/template read first (where present).

The 41 local repositories and inspected HEADs were:

```text
3dbear-monorepo d1891267f887   aggountant c429ef077e1c
blog 134f8dae2117               claude-code 6a2590911df2
crmctl ca9bcd40eee0             deutschpad f5a121ddd6cf
dkv-thunderbird-plugin f736d0292276  dkv-user-communication 6b88ae204600
dkv-userdb 816d3b1a8b54       ds4 80ebbc396aee
formative-agent 4de8f930b55e    formative-memory 49507597d8e8
formative-memory-maintenance d6fe279f0c19  frondeo-monorepo e8d2e8b1de30
glasspad a724b31451b3          grooveserve-monorepo 145a6de35a8a
homebase 97e073185ae5          homebrew-orchestratectl 20a70f463e69
hyrox-academy af69577e7006      intakectl e28b33fc4f85
issuectl ed74a09e212d           itsellesi-monorepo e886a2531f2d
kunnollavauhtiin-en a976f223ca12  kunnollavauhtiin-images 5a9523ad12f1
kunnollavauhtiin-monorepo ea11ea6ca0a2  kunnollavauhtiin5 413d7d7ec73e
okv-email-templates ce5acf32a5f8  okv-homepage f1a7f6d3c9b9
okv-monorepo 71f48042b791       okv-submissions-leaning 8a390c219c1e
openclaw 277a4b695264          orchestratectl/taskfleet c05238c1b4ad
osa-material-processing 64b0e832aa7c  osa-teachers-tool 81a106957210
ossctl/Shipshape 88f2b98d3411   out-of-context 54742121b5ee
pi b1efcf7d7c5d              pi-mono f953067814ce
project-canon 528f08f82282      simuna-creator a2928d9681ac
vensum-workspace 7793d8a522d8
```

Only 14 had non-excluded lexical matches. After inspection, eight are active owners; `glasspad` is analogy-only, `homebrew-orchestratectl` is the permanent migration stub, `kunnollavauhtiin-monorepo` is stale handoff prose, and the Taskfleet repository's residuals are its own canonical/bounded/permanent compatibility surfaces. `aggountant` and `crmctl` matched only open canon-review issues, not runtime/config dependencies. The other 27 repositories had zero tracked active-scope matches.

Haapa was independently enumerated from `/home/jari/Sources/*/.git` at the live host. Its maintained intake clone set is `3dbear-monorepo`, `blog`, `crmctl`, `deutschpad`, `frondeo-monorepo`, `glasspad`, `homebase`, `intakectl`, `issuectl`, `orchestratectl`, `ossctl`, `out-of-context`, `project-canon`, and `tilictl`. The Taskfleet clone is still named `/home/jari/Sources/orchestratectl`, has old remote text `git@github.com:jarimustonen/orchestratectl.git`, and is at `c05238c1b4ad`; this is active clone configuration relying on GitHub redirect, not evidence for a second source owner.

### Read-only machine and public-channel evidence

Commands run without apply/mutation:

- `hostname -s`, `tw server`, `tw client`, and `tailscale status --json` established that execution was on `gertrud`; no attached seat was routable.
- `homebase fleet status` and `homebase fleet doctor` ran on `gertrud`, `hauis`, and `haapa`. The `orchestratectl` package unit was `ok` on all three. Unrelated existing fleet findings were left untouched (Hauis managed-local/software residue; Haapa dotfile/consult/wilma gaps).
- SSH to `brunhild` timed out. Tailscale reported it offline with last-seen `2026-09-06T10:20:00.1Z`; its Taskfleet installation/state is **unverified**. Its owner/config source remains Homebase `dotfiles/homebase.toml` and `dotfiles/fleet.json`.
- `orchestratectl version --output json`, `config show --output json`, `run list --output json`, path inspection, and Homebrew receipt/tap listing were read-only. Gertrud and Hauis use `/opt/homebrew/bin/orchestratectl`; Haapa uses `/home/jari/.local/bin/orchestratectl`. All report 0.5.1 commit `f0c52ab232706fb480a51bfd45f2171c6b7aa056` and the 13-name old catalog.
- `gh repo view`, `gh release view`, GitHub contents API, and crates.io API calls (with an identifying `User-Agent`) established the canonical public receipts stated above.

## Owner map

Evidence keys refer to the ledger after the table.

| Owner repository / proposed focused E2 issue | Exact paths or units | Purpose and current dependency channel | State/config and compatibility constraint | Machine scope / reachability | Ordering and prerequisite | Evidence |
|---|---|---|---|---|---|---|
| **homebase** — `taskfleet-homebase-fleet-channel` | `dotfiles/{fleet.json,homebase.toml}` unit `orchestratectl`; `dotfiles/setup.d/{orchestratectl.sh,brew-trust.sh}`; `dotfiles/src/{brew-packages.txt,bin/homebase-fleet-{apply,update}}`; `dotfiles/src/.orchestratectl/config.toml`; `dotfiles/src/.config/tmux/projects.conf`; fleet tests and scoped AGENTS | Installs old Homebrew formula `jarimustonen/orchestratectl/orchestratectl` on Macs, old GitHub installer/binary on Linux, then runs `orchestratectl skill install --agent all --force`; declares the old source checkout/remote and old skill ownership | Populated `~/.orchestratectl`; config is a symlink to Homebase's old path and selects `harness.default=pi`. Taskfleet 0.6 may adopt this root in place. Do not move state while any real non-terminal run exists; do not rename stable `OCTL_*` | `gertrud` reachable/old Brew; `hauis` reachable/old Brew; `haapa` reachable/old release binary; `brunhild` unreachable/unverified | **First convergence slice.** After E1 closes, reconcile/quiesce real old runs. Update/check canonical formula/installer and skill catalog, canonical config path and source checkout metadata; land/push; run Homebase tests; apply only through normal fleet policy. This supplies `taskfleet` on Haapa before intakectl switches | H1–H7, M1–M4 |
| **intakectl** — `taskfleet-intakectl-runtime-convergence` | `crates/intakectl-cli/src/{intake,drain,template}.rs`; `templates/{agent-trigger.md,README.md}`; `recipes/{intake-repos.yaml,README.md}`; `e2e-drain.sh`; acceptance/E2E tests; Haapa `intakectl.service` and `intakectl-drainer.service`; `/home/jari/Sources/orchestratectl` target clone | Shells out to default `orchestratectl`; override is `AGENTBRIDGE_INTAKE_ORCHESTRATECTL`; parses stable JSON; polls/reattaches/cancels; registry key and route description are `orchestratectl`; Haapa PATH currently resolves only old binary | Intake's own `AGENTBRIDGE_*` namespace, DB `agentbridge`, lease/worktree dirs, and image markers are intentionally stable. The dependency-specific executable/default/diagnostic should become Taskfleet. Preserve or deliberately transition queued/external `--repo orchestratectl` callers before making `taskfleet` the sole key. Queue was idle; Haapa Taskfleet root absent and old root present | Haapa only, reachable. Service and drainer both active; live source `e28b33fc4f857f4291e4b40434d34c38b62a895f`; deployed binary reports `0.1.0 (3ec7617)` | Depends on canonical Taskfleet binary installed on Haapa by Homebase. Deploy an overlap-safe intake repo-key transition before Homebase workflow callers switch; run full intakectl gate; deploy from green main; restart drainer because hot worker-path files change; verify queue idle, executable inode, `/readyz`, and one safe dry-run/controlled intake path | I1–I8, M3 |
| **issuectl** — `taskfleet-issue-intake-template-convergence` | `crates/issuectl-core/templates/issue-intake-{skill,prompt}.md`; dogfood `.claude/skills/issue-intake/SKILL.md`, `.pi/agent/skills/issue-intake/SKILL.md`, `.codex/prompts/issue-intake.md` | Published `/issue-intake` says `/worktree-bug-analysis` additionally needs `orchestratectl`; no linked crate/process integration | Repo-local generated copies are hash/byte guarded and must be regenerated from one template change. This is documentation/catalog identity only; no Taskfleet state | Source reachable at `ed74a09e212dc2a2777da8958dec66f5f7846285`; generated copies occur downstream on multiple repos/machines | Land and release (or otherwise make the canonical template available) before refreshing downstream generated copies. Run issuectl's template/dogfood tests and full gate | Q1–Q3 |
| **homebase** — `taskfleet-homebase-workflow-references` | `dotfiles/src/.claude/skills/{review-lens-audit,wrap-up,mail-triage}/**`; root/dotfiles AGENTS; `.claude/skills/issue-intake`, `.codex/prompts/issue-intake`; health comments where operational; intake valid-key prose | Personal workflows directly invoke old command/home and route tool reports with `intakectl file --repo orchestratectl`; review-lens also names old low-level `octl-*` skills | Use canonical `taskfleet`, `~/.taskfleet` where a fresh/current path is intended, and canonical Taskfleet low-level skill names. Keep genuine old-home migration guidance only when explicitly compatibility-labelled | Same four Homebase hosts; `brunhild` unverified | Serialize after `taskfleet-homebase-fleet-channel`; depends on intakectl accepting canonical `taskfleet` repo key and on issuectl's updated template. Same-repo slice cannot run in parallel with the first Homebase slice | H8–H11, Q1 |
| **3dbear-monorepo** — `taskfleet-3dbear-workflow-convergence` | `AGENTS.md:523-532`; `skills/issue-intake/SKILL.md:256`; selected current workflow prose. Exclude old run reports/TODO history and the verified old `--model` research note | Active agent policy routes tool bugs to `~/Sources/orchestratectl`; tracked issue-intake copy declares old prerequisite | No Taskfleet config/state is repo-local. Refresh generated issuectl skill from its owner; use canonical Taskfleet repository/path in current policy | Gertrud source `d1891267…`; Haapa intake clone `fd736fa0…` (behind its remote/current local source); both reachable. Other team machines are outside Homebase's personal fleet and unverified | Depends on issuectl template availability and canonical intake key. Disjoint from blog/deutschpad/project-canon | A1–A3, HA1 |
| **blog** — `taskfleet-blog-issue-intake-convergence` | `.claude/skills/issue-intake/SKILL.md:252` | Tracked generated issuectl skill declares old prerequisite | Generated copy only; do not hand-diverge it from issuectl template | Gertrud and Haapa clones reachable at `134f8dae…` | Depends on issuectl template availability; refresh through issuectl's supported install path. Parallel with other downstream repos | B1–B2 |
| **deutschpad** — `taskfleet-deutschpad-workflow-convergence` | `AGENTS.md:31` | Current operating policy gives executable old `run create/wait/show/cancel` recipes | No repo-local Taskfleet state/config. Update only current operational guidance; old run ids/homes in TODO history remain history | Gertrud and Haapa clones reachable at `f5a121dd…` | After Taskfleet is available on supported reachable operator machines; disjoint repository, parallel-safe with downstream docs work | D1–D2 |
| **project-canon** — `taskfleet-project-canon-reference-convergence` | `.claude/skills/issue-intake/SKILL.md:252`, `.codex/prompts/issue-intake.md:247`; `crates/project-canon-cli/skills/cli-canon/SKILL.md:3,271`; matching skill tests; `AGENTS.md:160` current worker-brief rationale | Distributed/dogfooded skills use old tool name in prerequisite, audit examples, and tool-bug routing; no process/Cargo dependency | Public repo rule requires neutral facts. `taskfleet` is now the public canonical coordinate, so examples should use it; retain the specifically historical bug reference only if explicitly historical | Gertrud/Haapa source at `528f08f…`; its released binary is fleet-managed separately | Depends on issuectl template for generated copies. Update project-canon's own skill source/tests in the same repo worktree; parallel with other downstream repos | P1–P3 |
| **ossctl / Shipshape** — `taskfleet-shipshape-reference-convergence` | `AGENTS.md:142` (current fleet release policy) | Names `orchestratectl` as a current cargo-dist-owned fleet repo | No Taskfleet runtime state. Update current fleet prose only. Preserve `octl-core`/`orchestratectl` release-engine fixtures, old package graph test data, architecture analogies, and Taskfleet formula fixtures | Gertrud/Haapa source at `88f2b98d…`; reachable | Independent documentation-only source edit; can run in parallel once owner map is accepted | S1–S3 |

### Evidence ledger

- **H1** `homebase@97e073185ae5:dotfiles/fleet.json:92-96` and `dotfiles/homebase.toml:100-105` define unit `orchestratectl` and its exact old formula check.
- **H2** `dotfiles/setup.d/orchestratectl.sh:2-137` owns old Brew/Linux installers, binary retirement and all-agent skill installation.
- **H3** `dotfiles/setup.d/brew-trust.sh:24,38`, `dotfiles/src/brew-packages.txt:106`, and `dotfiles/src/bin/homebase-fleet-update:46` pin old tap/formula/repository.
- **H4** `dotfiles/AGENTS.md` “Autonomous harness and pi runtime” and `dotfiles/src/.orchestratectl/config.toml` own the linked `pi` default.
- **H5** `dotfiles/src/.config/tmux/projects.conf:47` pins `~/Sources/orchestratectl` and `git@github.com:jarimustonen/orchestratectl.git`.
- **H6** Homebase's host inventory is `dotfiles/homebase.toml:15-32`; read-only fleet status/doctor showed package unit `orchestratectl: ok` on Gertrud, Hauis and Haapa.
- **H7** installed 0.5.1 skill provenance is `~/.orchestratectl/state/pi-installed-skills.json`; Claude/pi copies carry `cli_version: 0.5.1`. No Taskfleet-owned telemetry extension exists under `~/.pi/agent/extensions`.
- **H8** `dotfiles/src/.claude/skills/review-lens-audit/SKILL.md:343-400` executes old command/home and old low-level skill names.
- **H9** `dotfiles/src/.claude/skills/wrap-up/SKILL.md:65-136` uses old repository/product/intake key.
- **H10** `dotfiles/AGENTS.md:228-234,563-571` and root `AGENTS.md` assign old binary/skill ownership.
- **H11** Homebase's tracked issue-intake copies contain the same old prerequisite as issuectl.
- **I1** `intakectl@e28b33fc4f85:AGENTS.md:16-29` and `README.md:13` define the binary layering.
- **I2** `crates/intakectl-cli/src/intake.rs:336-417` defaults to `orchestratectl` and reads `AGENTBRIDGE_INTAKE_ORCHESTRATECTL`.
- **I3** `intake.rs:1204-2059` owns create/show/reattach/cancel JSON integration and old-home diagnostics.
- **I4** `crates/intakectl-cli/src/{drain,template}.rs`, templates and tests carry the second agent-trigger path and executable default.
- **I5** `recipes/intake-repos.yaml:40,86-93` defines old target key/product/home; sibling clone resolution is `intake.rs:2623-2774`.
- **I6** `install-drainer-haapa.sh:118-174`, `drainer.sh:19-49`, and checked-in systemd units own Haapa deployment/config generation.
- **I7** live `systemctl --user show/cat`: `intakectl.service` executes `/home/jari/.local/bin/intakectl serve`; `intakectl-drainer.service` executes `/home/jari/Sources/intakectl/drainer.sh`; both active/running.
- **I8** live `intakectl doctor --json`: 9 ok, 1 unrelated auth warning, queue `0 pending/0 processing/0 dead`; config secrets remained redacted.
- **Q1** `issuectl@ed74a09e212d:crates/issuectl-core/templates/issue-intake-{skill,prompt}.md:251-256`.
- **Q2** issuectl `AGENTS.md` “Critical rule” maps templates to all dogfooded copies and requires lockstep.
- **Q3** `crates/issuectl-core/src/skill.rs` owns install/provenance behavior; old `tool:"orchestratectl"` test corpus is a separate compatibility fixture and is not a command dependency.
- **A1** `3dbear-monorepo@d1891267f887:AGENTS.md:523-532`; **A2** `skills/issue-intake/SKILL.md:256`; **A3** root README establishes this as an active maintained development/distribution monorepo.
- **B1** `blog@134f8dae2117:.claude/skills/issue-intake/SKILL.md:252`; **B2** root AGENTS/README establish the maintained deployed blog.
- **D1** `deutschpad@f5a121ddd6cf:AGENTS.md:31`; **D2** root README/AGENTS identify the live Haapa app and worktree workflow.
- **P1** `project-canon@528f08f82282:crates/project-canon-cli/skills/cli-canon/SKILL.md:3,271`; **P2** generated issue-intake copies; **P3** root AGENTS identifies the distributed catalog and public-neutrality rule.
- **S1** `ossctl@88f2b98d3411:AGENTS.md:142`; **S2** root AGENTS/README define permanent `ossctl` source/state compatibility; **S3** inspected old-package fixtures plus canonical Taskfleet formula reconciliation fixtures.
- **M1** Gertrud: old Brew formula/tap and populated legacy home; one genuine non-terminal run, this E1 worker.
- **M2** Hauis: old Brew formula/tap and populated legacy home; five old `pending` records, including existing worktree paths, require explicit reconcile/preserve review before any state movement.
- **M3** Haapa: old `~/.local/bin/orchestratectl`, populated legacy home, one `stillborn:true` pending record with no worktree/supervisor, active intake services, and old-named Taskfleet target clone.
- **M4** Brunhild: SSH unreachable and Tailscale offline; runtime deliberately reported unverified.

## Haapa and intake ownership

Ownership is exact, not inferred:

- **Service source owner:** `/home/jari/Sources/intakectl` (`jarimustonen/intakectl`, live checkout `e28b33fc…`). Its `deploy.sh`, `install-drainer-haapa.sh`, `drainer.sh`, checked-in unit files, code, and `recipes/intake-repos.yaml` define and deploy the intake behavior.
- **Machine convergence owner:** `/home/jari/Sources/homebase` (`97e073185ae5`), via package unit `orchestratectl` in `dotfiles/fleet.json` / `dotfiles/homebase.toml` and setup hook `dotfiles/setup.d/orchestratectl.sh`.
- **Live units:** `~/.config/systemd/user/intakectl.service` and `intakectl-drainer.service`. The first runs the installed service binary; the second runs the source-owned launcher and resolves `orchestratectl` through PATH.
- **Registry owner:** intakectl's own `recipes/intake-repos.yaml`, selected by `drainer.sh`; the anchor is `/home/jari/Sources/homebase`, but Homebase does not own the registry. Non-anchor targets resolve from `AGENTBRIDGE_INTAKE_REPO_<KEY>` or an anchor sibling.
- **Taskfleet intake clone:** `/home/jari/Sources/orchestratectl`, old remote spelling, canonical repository content. Rename/repoint/provision belongs to the intakectl E2 deployment transaction; Homebase owns the executable channel, not this registry clone.

## Intentional non-changes

These residuals are not E2 substitutions:

- **Stable protocol:** every public `OCTL_*` worker identity, notify, readiness, and documented test/control contract; state schema/JSON field names; contract id `orchestratectl.worker-telemetry-adapter`.
- **No adapter to rename:** `docs/WORKER-CONTROL-PLANE-ROLLOUT.md` and `issues/uncommonly-vague-family/item.md` explicitly record that the production pi adapter package is absent. Neither local `pi` nor `pi-mono`, installed pi extensions, nor maintained repos contain that contract id outside Taskfleet.
- **Bounded through 0.7.x:** Cargo package/binary `orchestratectl`, old branded env/config aliases, old config fallback, legacy-home adoption and diagnostics, old-installer migration stub, and compatibility tests/fixtures.
- **Permanent migration/safety:** old Homebrew repository `jarimustonen/homebrew-orchestratectl` and its `tap_migrations.json`; GitHub redirect; split-root/legacy readers; old crates/releases; migration receipts.
- **Historical and immutable:** changelogs, closed issues, recorded `source_ref: orchestratectl:…`, old run/event data, model-review metadata, 0.5.1 homes, snapshots/evidence, and release-engine tests deliberately modeling old `octl-core → orchestratectl` package graphs.
- **Stale/no operational effect:** Glasspad source comments comparing envelope/release conventions, `kunnollavauhtiin-monorepo` TODO cleanup prose, old run ids/homes in project TODO narratives, and old research observations. They may be edited opportunistically but do not authorize an E2 migration worktree.
- **Intakectl's own compatibility names:** database `agentbridge`, broad `AGENTBRIDGE_*` config namespace, migration bytes, image markers, lease and worktree-state paths stay unchanged. Only the dependency-specific command/field/suffix and repo route are in E2 scope.

## E2 wave plan

Only the issue slugs listed below are authorized by E1. They are **proposals, not materialized issues**.

### Wave 0 — safety preflight (conductor, no repository edit)

1. Confirm this E1 run is terminal and re-run old/canonical `run list` on every reachable host.
2. Reconcile Hauis's five non-terminal legacy records and inspect their actual worktrees/branches; preserve uncertain work. Treat Haapa's stillborn row as state evidence, not a live writer. Do not physically migrate any home as part of ordinary E2 unless separately chosen and Taskfleet's quiescence gate passes.
3. Record Brunhild as unverified until reachable.

### Wave 1 — source prerequisites (parallel only where disjoint)

- `taskfleet-homebase-fleet-channel` — canonical package/config/checkout/skill channel; converge Gertrud, Hauis, Haapa; Brunhild remains unverified if offline.
- `taskfleet-issue-intake-template-convergence` — canonical issuectl template + dogfood, full gate and normal release decision.
- In parallel, disjoint `taskfleet-shipshape-reference-convergence` may update only Shipshape's current fleet prose.

Do not overlap the two Homebase issues. Do not switch intakectl's executable before Taskfleet exists on Haapa.

### Wave 2 — intake overlap and independent direct consumers

After Wave 1's Haapa package proof:

- `taskfleet-intakectl-runtime-convergence` — add/verify canonical executable and overlap-safe canonical intake key, then deploy/restart/verify on idle Haapa.
- `taskfleet-deutschpad-workflow-convergence` — direct recipes.

These repositories are disjoint and may run in parallel. Intakectl's deployment verification is the prerequisite for Homebase to emit only the new intake key.

### Wave 3 — generated-copy and caller convergence

After issuectl's template is available and intakectl accepts the canonical key, run these disjoint repositories in parallel:

- `taskfleet-homebase-workflow-references` (serialized after the earlier Homebase slice),
- `taskfleet-3dbear-workflow-convergence`,
- `taskfleet-blog-issue-intake-convergence`,
- `taskfleet-project-canon-reference-convergence`.

Refresh generated issue-intake copies through issuectl's supported installer/generation path; do not hand-fork them. Each repository runs its own green gate and machine/deployment policy. No E2 may rename `OCTL_*`, persisted provenance, immutable fixtures, or the telemetry contract id.

### Wave 4 — reachable machine verification

Run Homebase's normal read-only status/doctor after convergence on each reachable fleet host, verify canonical binary/version/channel and Taskfleet-managed skill catalog, and verify no old executable is selected by Haapa intake. Do not claim Brunhild converged until directly observed.

## E3 baseline

Repeat from the exact post-E2 commits:

1. Enumerate the same immediate Git repositories under `/Users/jari/Sources` and the live Haapa clone set; record HEAD and remote for each.
2. Run the same tracked `git grep` pattern set, plus standalone `\boctl\b|octl-`, excluding the same immutable/generated/history boundaries. Search open issues separately from closed archaeology.
3. For every residual, label exactly one of: `canonical`, `active-legacy (fail)`, `stable-protocol`, `bounded-through-0.7`, `permanent-safety/migration`, `fixture`, `history/immutable`, or `stale/no-action`.
4. On each reachable fleet host, collect read-only `homebase fleet status`, `homebase fleet doctor`, `taskfleet version --output json`, `taskfleet config show --output json`, `taskfleet run list --output json`, resolved executable path, package receipt/tap, state-root presence, and installed skill provenance. Record unreachable hosts as unverified.
5. On Haapa, additionally verify `intakectl doctor --json`, queue idle/health, both unit paths and executable inode, drainer PATH/default, canonical target clone path+remote, and a safe controlled/dry-run subprocess contract. No selected `orchestratectl` executable, old intake key, old exact GitHub URL, old tap formula, or old installer may remain active.
6. Recheck crates.io/GitHub Release/canonical tap receipts and permanent old-tap migration metadata. Registry artifacts and the Cargo wrapper are expected compatibility, not failures.
7. Confirm the external pi adapter status. If still absent, report absent; if later present, inspect its owning source/package/install manifest and require it to retain `orchestratectl.worker-telemetry-adapter` plus `OCTL_RUN_ID`, `OCTL_NODE_ID`, and `OCTL_ATTEMPT` unchanged.

### Acceptance criteria status

- [x] All maintained reachable repositories and Homebase fleet units are covered by an auditable search inventory.
- [x] Active references are separated from intentional compatibility/history/generated occurrences.
- [x] Haapa and intake ownership is identified with exact repository paths/units and reachability status.
- [x] Each required E2 change has one owning repository, dependency channel, ordering, and proposed issue.
- [x] E1 authorizes only the listed E2 worktrees; no migration has occurred.
