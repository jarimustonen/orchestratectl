---
created: 2026-07-03
updated: 2026-08-16
type: bug
status: wontfix
priority: normal
related: ['@supervisor-dead-merge-no-teardown']
labels: [keep-0.2]
closed: 2026-08-16
closed_by: claude
---

# Verify recycled legacy bare-integer supervisor.pid via process identity

_Source: crates/taskfleet-cli/src/run/merge.rs_

## Description

Documented KNOWN RESIDUAL from supervisor-dead-merge-no-teardown. A legacy supervisor.pid file without a start-time, whose pid was recycled by an unrelated live process, reads as 'alive', so run merge skips reattach and could strand the run. Modern start-time pid files are immune. Robust fix needs a process-identity check (e.g. ps command matches 'taskfleet supervise <id>'). Rare — legacy files are a phased-out migration artifact — filed for completeness. Non-blocking for v0.1.0.

## Decisions

### 2026-08-13T11:10:42Z · @adr-decision-2

KEEP-and-fix: PID identity survives as the crash-backstop recycle defense (A4). Surface survives the thin model; fix is model-independent. Recorded by ADR 0001 (docs/decisions/0001-thin-supervisor-vs-harden.md).

## Resolution

### 2026-08-16T15:32:58Z · @claude

Suljettu epärealistisena. Vaatii kaksi epätodennäköisyyttä yhtä aikaa: vanhentuneen bare-integer pid-tiedoston (issue toteaa itse: poistuva siirtymäartefakti) JA käyttöjärjestelmän kierrättämän saman prosessinumeron toiselle elävälle prosessille. Nykyiset start-time -pid-tiedostot ovat immuuneja.
