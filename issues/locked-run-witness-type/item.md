---
created: 2026-06-12
updated: 2026-06-12
type: improvement
status: open
priority: normal
---

# LockedRun witness type for unlocked event-append API

## Description

- Ei suoraa käyttäjävaikutusta — perustelu: tämä on tyyppiturvallisuuden parannus, joka estää Rust-kutsujia kutsumasta `append_and_apply_unlocked` / `append_event_with_seq` ilman flockia. Käyttäjille näkyvä regressio (deadlock tai datakorruptio) ilmenisi vain bugin kautta uudessa core-kutsupolussa, ei tämänhetkisessä koodissa.
- **Miksi tämä vaatii oman suunnittelunsa**: `RunLock::with_lock` täytyy palauttaa `&LockedRun<'a>` -witness, ja jokainen lukotonta varianttia kutsuva (CLI:n `event create`, tulevat supervisor-polut) pitää muuttaa ottamaan witness vastaan. Refaktori koskettaa octl-core-API:a julkisesti; vaatii harkitun julkisen tyypin nimen, lifetime-kontekstin, ja regression-testit nykyisille kutsupoluille. Liian iso muutos tämän PR:n scopeen.
