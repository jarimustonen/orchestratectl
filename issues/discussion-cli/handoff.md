# Handoff — discussion-cli LLM-review DISCUSS items

Two product/contract decisions surfaced during multi-model review of this
PR. Both are out of scope for the focused `list|show|resolve` MVP but
warrant a human call before they harden across more domain verbs.

## F19 — Pitäisikö `--choice` validoida `Discussion.options`-listaa vastaan?

- **Milloin tämä näkyy käyttäjälle:** Kun aloitettu keskustelu tarjoaa kiinteän valikoiman vastauksia (`options: ["drop", "defer"]`) ja agentti tai operaattori antaa `--choice keep`.
- **Miten se näkyy:** CLI hyväksyy nykyisellään minkä tahansa merkkijonon hiljaisesti. Reducer tallentaa `resolution: "keep"`, vaikka tämä ei vastaa mitään ehdotettua vaihtoehtoa.
- **Miksi sillä on väliä:** Downstream-automaatio (esim. supervisor joka konsumoi `discussion.resolved` -tapahtumia) ei voi luottaa siihen, että `resolution` on jokin tunnetuista valinnoista. Hiljainen operaattorivirhe voi mennä huomaamatta kunnes lukuvirta käytetään konkreettiseen päätökseen.
- **Mistä päätös on kyse:** Pitääkö `options` rajata enumiksi, joka pakotetaan (`invalid_choice`-virhe ja `expected: [options]`), vai onko se vain ehdotuksia ja agentilla on aina vapaa vastausvalinta (kuten dokumentaatio nykyisellään kuvaa)? Vapaamuotoisuus tukee agenttien luovaa harkintaa mutta heikentää tilan eheyttä.

## F20 — Pitäisikö virhekuoreen lisätä erillinen `details`-kenttä konfliktitilan viestintään?

- **Milloin tämä näkyy käyttäjälle:** Kun CLI palauttaa `discussion_already_resolved` tai `idempotency_conflict` ja sisällyttää nykyisen resoluution / nootin / aikaleiman `error.expected`-kenttään, jotta retry voi tarkistaa miksi se hylättiin.
- **Miten se näkyy:** AGENTS-AI-FIRST-CLI §10 määrittelee `expected` semantiikan "mitä CLI odotti" -tarkoitukseen, mutta tässä se kantaa "mikä nykytila oli" -informaatiota. Asia toimii sopimuksena, mutta semantiikka on hieman venytetty — sama valinta on tehty `event create`:n `idempotency_conflict`-virheessä, joten kuvio toistuu jo nyt useammassa kohtaa.
- **Miksi sillä on väliä:** Jos useampi domain-komento alkaa käyttää `expected`-kenttää nykytilan välittämiseen, kontrakti hämärtyy. Agentit eivät pysty erottamaan "mitä syötteen pitäisi olla" ja "mitä järjestelmässä on" -informaatiota saman avaimen takaa.
- **Mistä päätös on kyse:** Lisätäänkö envelope-kontraktiin oma `details`-kenttä konfliktoivan tilan välitykseen ja siirretäänkö nykyiset `expected`-käytöt sinne, vai pidetäänkö `expected` joustavana ja kirjataankö AGENTS-AI-FIRST-CLI:hen erikseen, että `expected` voi sisältää joko odotettua syötettä tai konfliktoivaa tilaa?

---

## Resolved (already applied in this PR)

All FIX items from the multi-LLM review landed before merge. See
`history/assessment-discussion-cli.md` for the decision table.

## SPIN-OFF candidates

Filed as separate issues (see `issuectl new` commands below or in the
handoff at the parent epic).

- F13 — Manifest counter desync hardening (`manifest-counter-desync`)
- F14 — Centralize idempotency in `taskfleet-core::AppendOutcome` (`core-idempotency-api`)
- F15 — Reducer-level path-traversal defense for event-log IDs (`reducer-path-traversal-defense`)
- F16 — Single source of truth for projected paths (`projected-paths-into-reducer`)
- F17 — Control-character escaping in human-text CLI output (`cli-text-output-escape`)
- F18 — Wire-contract DTOs for `--json` payloads (`cli-json-dto-layer`)
