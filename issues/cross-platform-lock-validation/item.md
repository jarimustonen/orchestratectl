---
created: 2026-06-27
updated: 2026-06-27
type: chore
status: open
priority: normal
epic: orchestratectl-mvp
labels: [review-spinoff]
---

# Cross-platform lock validation + supply-chain advisory gate for fs4

## Description

Follow-up from the fs2 → fs4 swap (issue replace-fs2-with-fs4). Four-model /llm-review converged on two non-blocking hardening items that were out of scope for a drop-in dependency swap:

1. **Cross-platform lock coverage.** The V4 flock stress test (50 threads × 1000 iters) is only validated on macOS APFS. fs4 swaps the platform backends (rustix vs libc on Unix, windows-sys vs winapi on Windows), and advisory-lock semantics differ across platforms and on networked/synced filesystems (NFS, SMB, WSL, Docker bind mounts). Add Linux (and, if Windows is ever supported, Windows) runs of the stress test, plus a multi-process (not just multi-thread) exclusion test that proves a second RunLock holder blocks until the first drops. Note: orchestratectl currently targets macOS, so this is low priority unless Linux support is planned.

2. **Supply-chain advisory gate.** The motivation for dropping fs2 was that it is unmaintained with known soundness issues. Add a `cargo deny` (or `cargo audit`) advisory check in CI to prevent fs2 — or any other RUSTSEC-flagged crate — from being reintroduced via a transitive dependency.

Optional: pin an MSRV CI job at rust-version 1.85 so a future fs4/rustix minor bump that raises MSRV is caught rather than silently regressing. (At the currently locked versions, fs4 1.1.0 = 1.75 and rustix 1.1.4 = 1.63, both below 1.85, so this is not currently triggered.)

See history/review-fs4-migration.md on the replace-fs2-with-fs4 branch for the full review.
