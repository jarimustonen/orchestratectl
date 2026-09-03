# ADR 0002 R9 adversarial review synthesis

Reviewers: `gemini-3.1-pro-preview`, `gpt-5.6-sol`, `claude-fable-5`, and
`deepseek-v4-pro`. The review used two cross-review rounds and one bounded
context follow-up. Raw rounds are preserved beside this synthesis.

## Confirmed findings fixed before candidate CI

1. cargo-dist generated the reusable activation-gate caller with `actions: write`
   but without `contents: read`, even though the called workflow checks out the
   repository. The permission is now declared through `dist-workspace.toml`, the
   workflow was regenerated with cargo-dist 0.28.2, and both Rust and shell
   topology checks pin it.
2. A first attempt put the persistent self-hosted runner directly into the
   ordinary PR matrix, exposing it to fork PR code. CI now retains hosted
   Ubuntu/macOS coverage and has a separate self-hosted ARM64 job limited to
   same-repository pull requests.
3. The first trust split used `matrix.*` in a job-level `if`, where that context
   is unavailable before matrix expansion. Splitting the self-hosted job removes
   that invalid expression.
4. The active R7→R9 plan still told R9 to install a tap token and move release
   state to `ready`. It now records the actual boundary: R9 restores canonical
   tag dispatch while release/distribution stay blocked and the token stays
   inert for R10.

## Incorrect or dropped concerns

- cargo-dist's credentialed `host --steps=create` was claimed to mutate GitHub
  before the gate. Source inspection of exact 0.28.2 shows its GitHub arm only
  computes manifest URLs; network mutation belongs to the later generated host
  job. The claim is incorrect for this topology.
- `source_repository.current` must become the canonical repository after the
  one-way rename; treating it as an immutable before-ledger would break the
  activation verifier. Before-state is preserved separately.
- The old latest-installer URL cannot resolve to a not-yet-published canonical
  artifact. It is explicitly deferred to R10 rather than reported as passed.

## Accepted pre-R10 residual

cargo-dist 0.28.2's generated host job accepts skipped build dependencies, so a
blocked tag run relies on the activation gate's early whole-run cancellation.
This is the explicitly documented R7 workaround, not a new R9 design. The gate
normally finishes before the slower plan, no tag exists, release activation is
still blocked, and the tap token is inert. The review correctly notes that this
is not a hard structural dependency and that cargo-dist also generates
`secrets: inherit` for the reusable gate. R10 must re-evaluate both before
installing live credentials or setting activation to `ready`; R9 does not
silently reinterpret them as release authorization.

## Verdict

After the four fixes, no reviewer retained a source blocker before candidate
CI. Candidate CI, canonical clone/fetch/push, immutable after-state receipts,
and final exact-main CI remain evidence gates rather than source findings.
