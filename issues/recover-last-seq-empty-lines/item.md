---
created: 2026-06-12
updated: 2026-06-12
type: improvement
status: open
priority: normal
---

# recover_last_seq: loop over multiple trailing empty lines

## Description

- Ei suoraa käyttäjävaikutusta — perustelu: kolme peräkkäistä tyhjää riviä `events.jsonl`:n lopussa on epätodennäköinen tila (yksi tyhjä rivi käsitellään jo, eikä mikään write-polku tuota niitä). Vaikutus näkyisi vain manuaalisesti muokatussa tai eksoottisesti vaurioituneessa lokissa.
- **Miksi tämä vaatii oman suunnittelunsa**: muutos on pieni — käsin kirjoitettu rekursio muutetaan silmukaksi — mutta lokin recovery-polku ansaitsee oman testisarjan eri vaurioskenaarioilla (torn tail, tyhjät rivit, NUL-tavut, väärä BOM). Halpa fix ilman testikattavuutta ei paranna luotettavuutta.
