---
created: 2026-06-12
updated: 2026-06-29
type: improvement
status: fixed
priority: normal
closed: 2026-06-29
---

# Bound spin-off issuectl subprocess (timeout, output cap, env policy, kind mapping)

## Description

## Tausta

`materialize_via_issuectl` käyttää `Command::output()` -kutsua (`approve.rs:209`), joka odottaa lapsiprosessia rajattomasti, lukee kaiken stdoutin/stderrin muistiin ilman ylärajaa, perii koko vanhemman ympäristön (mukaan lukien salaisuudet) eikä aseta nimenomaista `current_dir`-arvoa. Lisäksi `--type feature` on kovakoodattu välittämättä `proposed_kind`-arvosta (`approve.rs:204`).

## Milloin tämä näkyy käyttäjälle

- `issuectl` jää jumiin (verkko-ongelma, lukko, deadlock) → `taskfleet approve` jumittuu määräämättömäksi ajaksi
- `issuectl` palauttaa hyvin suuria tulosteita → CLI:n muistinkäyttö pomppaa
- Bug-tyyppinen spin-off materialisoituu virheellisesti `feature`-tikettinä

## Miten se näkyy

- Skripti, jolla ei ole ulkoista aikakatkaisua, jää roikkumaan
- Lokeissa näkyy isoja `issuectl`-tulosteita; OOM mahdollinen
- Issue-trackerissä on väärä tyyppi

## Miksi sillä on väliä

CLI on automaation runko: rikkinäinen `issuectl` ei saa kaataa koko agentti-ketjua. Bug-tikettien luokittelu vaikuttaa raportointiin ja tiimiprioriteetteihin.

## Miksi tämä vaatii oman suunnittelunsa

Korjaus kytkeytyy materialisointi-arkkitehtuuriin (toinen spin-off, `spinoff-issuectl-materialization-arch`). Lisäksi se vaatii:

- Uuden riippuvuuden (`wait-timeout` tai oma poll-toteutus) lisäämistä `taskfleet-cli` cratelle
- Ympäristömuuttujien sallittujen listojen ja `current_dir`-politiikan päättämistä
- `proposed_kind` → `issuectl --type` -mappauksen verifiointia `issuectl`:n hyväksymiä arvoja vasten
- Aikakatkaisun oletusarvon ja `--issuectl-timeout`-lipun semantiikan päättämistä

Nämä eivät ole mekaaninen yhden tiedoston korjaus, vaan tarkoituksellinen rajapintapolitiikka. Kannattaa käsitellä materialisointi-arkkitehtuurin yhteydessä.
