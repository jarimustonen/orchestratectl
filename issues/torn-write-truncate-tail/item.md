---
created: 2026-06-12
updated: 2026-06-29
type: bug
status: fixed
priority: normal
closed: 2026-06-29
---

# events.jsonl torn-write recovery doesn't truncate before append

## Description

- **Milloin tämä näkyy käyttäjälle**: kun orchestratectl-prosessi kaatuu kesken `events.jsonl`-rivin kirjoituksen (tappo, kernel-paniikki, levyn täyttyminen) ja seuraava sanctioned-write-path-kutsu yrittää appendata uutta tapahtumaa.
- **Miten se näkyy**: tapahtumaloki sisältää rivin, jossa katkennut JSON ja uusi tapahtuma ovat sulautuneet — `read_all_events` epäonnistuu pysyvästi, `event create` palauttaa system-virheen, ajo on käytännössä jumissa.
- **Miksi sillä on väliä**: koko sanctioned-write-path:n lupaus murenee, jos kerran katkennut rivi jättää ajon pysyvästi rikki. Skill-shim-kutsut ja supervisor menettävät kyvyn edetä.
- **Miksi tämä vaatii oman suunnittelunsa**: `recover_last_seq` pitää refaktoroida palauttamaan sekä viimeinen seq että viimeisen kelvollisen rivin tavu-offset. Append-polku tarvitsee `set_len(valid_offset) + fsync(parent_dir)` ennen kirjoitusta, ja olemassa olevat `append_event` / `append_event_with_seq` -kutsupolut pitää päivittää yhtenäisesti. Vaatii oman testisarjan kaatumissimulaatioilla.
