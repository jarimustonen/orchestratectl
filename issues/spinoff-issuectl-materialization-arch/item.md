---
created: 2026-06-12
updated: 2026-06-12
type: improvement
status: open
priority: normal
---

# Redesign spin-off issuectl materialization to avoid duplicate external issues

## Description

## Tausta

`orchestratectl spinoff approve` -komento kutsuu nykyään `issuectl new` -prosessia ennen kuin se hankkii ajoon liittyvän `flock`-lukon (`approve.rs:111`). Kahden samanaikaisen hyväksynnän tapauksessa molemmat kutsuvat `issuectl`:in ja luovat omat issue-tikettinsä — vain toinen voittaa tapahtumalokin kilpailun, ja toinen jättää orpoja tikettejä ulkoiseen seurantaan. Vaikka `--idempotency-key` on tarjolla, sitä ei välitetä `issuectl`:lle, joten lupa ei kata ulkoista sivuvaikutusta.

## Milloin tämä näkyy käyttäjälle

- Käyttäjä tai agentti yrittää uudelleen hyväksyntää aikakatkaisun, verkkohäiriön tai signaalin jälkeen
- Useampi agentti tai pipeline ajaa saman hyväksynnän rinnakkain
- Käyttäjä luottaa `--idempotency-key`:hen ja olettaa että koko operaatio on retry-turvallinen

## Miten se näkyy

- Issue-trackerissä on kaksi tikettiä yhdelle hyväksytylle spin-offille
- `spinoffs/<id>.json`-projektio osoittaa vain yhteen tikettiin; toinen on orpo eikä mikään skripti tiedä siitä
- `--dry-run` ei näytä todellista lopputulosta

## Miksi sillä on väliä

Issue-trackerin tikettien duplikaatit aiheuttavat hämmennystä ja ylimääräistä siivoustyötä. Pahempaa: orpotikettejä ei kytketä mihinkään ajoon, joten ne jäävät elämään ilman seurantaa.

## Miksi tämä vaatii oman suunnittelunsa

Ratkaisuvaihtoehdot eivät ole triviaaleja:

1. **Lukon sisällä materialisointi** — yksinkertaisin, mutta blokkaa ajon tapahtumalokin `issuectl`-kutsun ajaksi (~satoja millisekunteja).
2. **Reserve-then-materialize -kuvio** — uusi `spinoff.issue_materializing` tapahtumakind ja erillinen materialisointivaihe; vaatii reducer-muutoksia ja crash-recovery -säännöt.
3. **Erillinen `spinoff materialize` -verbi** — `approve` vaatii `--issue-slug`:n; auto-materialisointi siirtyy omaan komentoonsa. Selkein lifecycle, suurin API-muutos.
4. **Deterministinen idempotency-avain `issuectl`:lle** — vaatii että `issuectl new` tukee idempotency-avaimia (ei tällä hetkellä).

Päätös vaikuttaa supervisor-prosessin tulevaan suunnitteluun, joten se on parempi tehdä isompana suunnitteluvaiheena kuin tämän PR:n yhteydessä.
