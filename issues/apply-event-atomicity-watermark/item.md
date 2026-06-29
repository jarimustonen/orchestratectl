---
created: 2026-06-12
updated: 2026-06-29
type: bug
status: fixed
priority: normal
closed: 2026-06-29
---

# Append-then-apply is not atomic across reducer failures (need applied_seq)

## Description

- **Milloin tämä näkyy käyttäjälle**: kun `apply_event` epäonnistuu reducerin sisällä (I/O-virhe, väärä JSON, levyn täyttyminen) tapahtumarivin onnistuneen `sync_all`:n jälkeen, ja kutsuja yrittää uudelleen samalla `--idempotency-key`:llä.
- **Miten se näkyy**: tapahtumaloki sisältää tapahtuman, mutta projection-tiedostot (manifest, nodes, discussions, spinoffs) eivät heijasta sitä. Idempotenssi-uusinta palauttaa edellisen seq:n eikä reducer aja koskaan tapahtumaa loppuun.
- **Miksi sillä on väliä**: lukijat (`run show`, TUI) näkevät pysyvästi väärän tilan suhteessa kanoniseen tapahtumalokiin. Vastoin design.md §1:n lupausta että append + projection ovat atomisia.
- **Miksi tämä vaatii oman suunnittelunsa**: tarvitaan `applied_seq`-watermark manifest.json:iin, lock-acquire -aikainen replay kaikille soveltamattomille tapahtumille, sekä reducerin idempotenssin uudelleenarviointi (laskureiden double-counting riski). Tämä on uusi invariant ja koskettaa jokaista lukupolkua ja reducer-funktiota.
