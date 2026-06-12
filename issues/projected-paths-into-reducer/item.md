---
created: 2026-06-12
updated: 2026-06-12
type: improvement
status: open
priority: normal
---

# Move projection enumeration into reducer (event-create dry-run parity)

## Description

- **Milloin tämä näkyy käyttäjälle**: kun käyttäjä ajaa `event create --dry-run` ja luottaa `projections`-listaan päättääkseen, mitä tiedostoja seuraavaksi luetaan, tai kun reduceria laajennetaan uuden projection-tiedoston kanssa.
- **Miten se näkyy**: `--dry-run` raportoi väärät tiedostot — esim. unohtaa `nodes/<id>.json` kun reducer alkaa päivittää solmun `discussions`-listaa, tai listaa tiedostoja joita reducer ei oikeasti kosketa.
- **Miksi sillä on väliä**: skill-shim ja muut bash-kutsujat luottavat siihen, että sanctioned-write-path kertoo totuuden; ristiriita rikkoo niiden cache-invalidointi-päättelyn ja voi piilottaa todellisia regressioita reducerissa.
- **Miksi tämä vaatii oman suunnittelunsa**: ratkaisu tarkoittaa reducerin sivuvaikutus-luettelointia (esim. `plan_projections(&Event) -> Vec<PathBuf>`) ja `apply_event`:n palauttamista koskemaan polkulistaa. Tämä koskee jokaista kindiä, edellyttää yhtenäisyystestiä CLI-listaa vasten ja siirtää loogisen tiedon CLI:stä coreen. Liian iso muutos tähän PR:ään ja vaatii oman API-suunnittelun.
