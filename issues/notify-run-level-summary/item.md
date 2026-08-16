---
created: 2026-07-24
updated: 2026-08-16
type: improvement
status: open
priority: normal
related: ['@no-completion-notification-to-parent']
labels: [keep-0.2]
lane: lifecycle
lane_seq: 70
---

# Run-level completion summary for --notify (multi-node runs)

_Source: review of no-completion-notification-to-parent_

## Description

# Run-level completion summary for `--notify` (multi-node runs)

The `run create --notify <cmd>` hook currently reads `OCTL_SUMMARY` from the
`n-0001` node's terminal report only (`crates/octl-cli/src/supervise/notify.rs`,
`read_summary`). That is correct for single-worker kinds (spinoff, code, research,
bugfix, …) but empty or misleading for multi-node runs:

- **Milloin tämä näkyy käyttäjälle** — kun `--notify`-koukku on rekisteröity
  `fan-out`- tai `orchestrate`-ajolle (tai tulevalle moninodiselle ajolle).
- **Miten se näkyy** — `OCTL_SUMMARY` on tyhjä (orchestrate-driverin `n-0001` on
  runko ilman raporttia) tai kertoo vain yhden noden tuloksen, vaikka
  `OCTL_STATUS=failed` johtuisi toisesta nodesta.
- **Miksi sillä on väliä** — ilmoituksen saava sessio saa harhaanjohtavan tai
  tyhjän yhteenvedon juuri niissä ajoissa (kampanjat, fan-outit), joissa tiivis
  tilannekuva olisi arvokkain.
- **Miksi tämä vaatii oman suunnittelunsa** — oikea korjaus on run-tason
  yhteenveto: joko persistoida tiivistelmä `rollup_status`-vaiheessa manifestiin,
  tai aggregoida kaikkien terminaali-nodejen raportit deterministisesti
  (epäonnistuneet ensin). Kumpikin koskettaa reducer/rollup-polkua ja manifestin
  skeemaa, joten se ei kuulu tämän at-most-once-koukun laajuuteen.

Scope: single-node runs (this issue's target) are unaffected. Defer until a
multi-node `--notify` consumer actually exists.

## Decisions

### 2026-08-13T11:10:42Z · @adr-decision-2

KEEP-and-fix: Fan-out + notify survive; a run-level multi-node completion summary applies. Surface survives the thin model; fix is model-independent. Recorded by ADR 0001 (docs/decisions/0001-thin-supervisor-vs-harden.md).
