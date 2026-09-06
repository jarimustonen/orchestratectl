---
created: 2026-06-27
updated: 2026-06-27
type: chore
closed: 2026-06-27
status: done
priority: normal
epic: taskfleet-mvp
labels: [review-spinoff]
---

# Cross-platform lock validation + supply-chain advisory gate for fs4

## Description

Follow-up from the fs2 → fs4 swap (issue replace-fs2-with-fs4). Four-model /llm-review converged on two non-blocking hardening items that were out of scope for a drop-in dependency swap:

1. **Cross-platform lock coverage.** The V4 flock stress test (50 threads × 1000 iters) is only validated on macOS APFS. fs4 swaps the platform backends (rustix vs libc on Unix, windows-sys vs winapi on Windows), and advisory-lock semantics differ across platforms and on networked/synced filesystems (NFS, SMB, WSL, Docker bind mounts). Add Linux (and, if Windows is ever supported, Windows) runs of the stress test, plus a multi-process (not just multi-thread) exclusion test that proves a second RunLock holder blocks until the first drops. Note: taskfleet currently targets macOS, so this is low priority unless Linux support is planned.

2. **Supply-chain advisory gate.** The motivation for dropping fs2 was that it is unmaintained with known soundness issues. Add a `cargo deny` (or `cargo audit`) advisory check in CI to prevent fs2 — or any other RUSTSEC-flagged crate — from being reintroduced via a transitive dependency.

Optional: pin an MSRV CI job at rust-version 1.85 so a future fs4/rustix minor bump that raises MSRV is caught rather than silently regressing. (At the currently locked versions, fs4 1.1.0 = 1.75 and rustix 1.1.4 = 1.63, both below 1.85, so this is not currently triggered.)

See history/review-fs4-migration.md on the replace-fs2-with-fs4 branch for the full review.

## Resolution

Scoped to **item 2 (supply-chain advisory gate)** this pass; item 1 deferred.

**Shipped:**

- `deny.toml [advisories]`: added explicit `unsound = "all"` as general
  defense-in-depth, alongside the existing `unmaintained = "all"` / `yanked =
  "deny"`. Documented that the v1 `vulnerability`/`notice` knobs were removed in
  the v2 schema (cargo-deny#611) and vulnerabilities now always deny — so no extra
  knob is needed to fail on a RUSTSEC vulnerability.
- `deny.toml [bans]`: added `deny = [{ crate = "fs2", ... }]` reintroduction
  guard. A transitive dep pulling fs2 back in fails the `deny` CI job.

**Empirical findings from the /llm-review pass (cargo-deny 0.19.9, against a
freshly fetched RustSec DB):**

- **fs2 has no RUSTSEC advisory in the database today** — there is no
  `crates/fs2/` entry. So the `unmaintained`/`unsound` advisory gates do NOT catch
  fs2; the `[bans]` entry is the *sole* guard. The original issue framing ("fs2 is
  unmaintained with known soundness issues, add an advisory gate") is only half
  right: an advisory gate would not have caught fs2. The name-based ban is what
  protects us. Comments in `deny.toml` corrected to say this plainly.
- **`unmaintained = "all"` is a hard error gate, not a warning.** Verified by
  adding a real unmaintained crate (`backoff`, RUSTSEC-2025-0012) to the tree:
  `cargo deny check advisories` emits `error[unmaintained]` and exits 1
  ("advisories FAILED"). With `unmaintained = "none"` it exits 0. (Two reviewers
  claimed v2 informational advisories only warn and need `--deny warnings` — that
  is false for 0.19.9.)
- **fs2 ban fires correctly:** `cargo add fs2` → `cargo deny check bans` fails with
  `error[banned] crate 'fs2 = 0.4.3' is explicitly banned`; rolled back. A
  `package = "fs2"` rename is still caught (lockfile crate name); a differently
  named fork is not (accepted limitation, noted in config).
- `cargo deny check --locked` passes clean on the current tree (advisories, bans,
  licenses, sources all ok).
- Design note updated: `issues/ci-and-lints/design.md` cargo-deny section.

**Deferred spin-off findings (from review — out of scope here):**

- **Cross-platform target coverage for cargo-deny.** CI runs on the Linux runner,
  so cargo-deny only evaluates the host-target dependency graph; a macOS-only
  reintroduction of a banned/flagged crate could go uncaught. Adding
  `[graph].targets = ["x86_64-apple-darwin", "aarch64-apple-darwin", ...]` would
  close this — but doing so surfaces pre-existing **license** allow-list failures
  (platform-specific crates, plus deprecated `MIT/Apache-2.0` slash-syntax not
  matching the allow-list), and `all-features = true` widens that further. Needs
  its own triage. This dovetails with deferred **item 1** (cross-platform lock
  coverage) above.
- **Advisory-DB freshness.** The advisory gate is only as fresh as the RustSec DB.
  Mitigated in CI because `EmbarkStudios/cargo-deny-action@v2` fetches the DB on
  each run rather than relying on the cached `~/.cargo` registry. No change needed
  unless the workflow is ever switched to a self-managed/offline DB.
- **`multiple-versions = "warn"`** (pre-existing, deliberate) and an explicit
  empty `ignore = []` advisory policy were raised as nice-to-haves; left as-is.

**MSRV gate (optional item):** already present — CI has an `msrv (1.85)` job
(`.github/workflows/ci.yml`) pinned to `rust-version` from `Cargo.toml`. No change
needed.

**Deferred — item 1 (cross-platform lock coverage):** not done. taskfleet
targets macOS only and no Linux CI runner exists today; adding one (plus a
multi-process exclusion test) is out of scope here. Revisit if/when Linux support
is planned. The flock stress test remains macOS-APFS-only.
