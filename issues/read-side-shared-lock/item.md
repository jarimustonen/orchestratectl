---
created: 2026-06-12
updated: 2026-06-12
type: improvement
status: open
priority: normal
---

# Read paths need shared flock (or docstring downgrade)

## Description

- **Milloin tämä näkyy käyttäjälle**: kun `orchestratectl run show` (tai TUI) lukee `manifest.json`:in ja `nodes/*.json`:in samalla kun toinen prosessi suorittaa reduceria (esim. `discussion.opened` joka päivittää sekä `discussions/<id>.json`:in että `manifest.open_discussions`-laskurin).
- **Miten se näkyy**: lukija näkee uuden `discussions/<id>.json`-tiedoston, mutta `manifest.open_discussions` näyttää vielä vanhaa lukemaa — tai päinvastoin. Lukijoiden raportit ovat hetkellisesti ristiriidassa.
- **Miksi sillä on väliä**: skill-shim ja supervisor luottavat siihen, että projection on koherentti. design.md väittää atomisuuden, mutta käytännössä jaettua lukkoa ei käytetä.
- **Miksi tämä vaatii oman suunnittelunsa**: pitää lisätä `RunLock::with_shared_lock` (LOCK_SH) jokaiseen lukupolun verbiin (`run show`, `run list`, `event tail` kun se valmistuu, TUI:n watcher). Pelkkä docstringin lievennys "eventual consistency"-tyyliseksi on vaihtoehto, mutta käyttäjäkokemus huononee. Päätös koskee koko lukurajapintaa.
