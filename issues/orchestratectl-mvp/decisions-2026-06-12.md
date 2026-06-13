# orchestratectl MVP — kerätyt päätös­pyynnöt 2026-06-12

Kerätty session aikana esiin tulleista keskusteluista ja handoff-dokumenteista. Tarkoitus: sinä lisäät kommenttisi kunkin kohdan alle ja minä sen jälkeen toteutan / lykkään / muokkaan sen mukaan.

Korit:
- **A** = estää MVP-valmistumisen
- **B** = muokkaa MVP:n muotoa (hyvä päättää pian)
- **C** = voi lykätä myöhemmäksi

---

## A1. Cross-repo `create.sh` — exit-koodi osittaisessa virhetilanteessa

**Status**: Estää MVP:n viimeisen issuen (`all-kinds-spawn`).

**Tilanne**: Kun teemme cross-repo-patchin homebasen `~/.claude/skills/worktree/scripts/create.sh`-skriptiin, sen pitää palauttaa strukturoitu JSON ja jäsennellyt exit-koodit. Yksi epäselvä tilanne: worktree luotiin onnistuneesti, mutta tmux-ikkunaan agentin syöttö epäonnistuu kesken ilmoittamisen.

**Vaihtoehdot**:
1. **Exit 1** = "user/system error" — skripti siivoaa worktreen pois, käyttäjä alkaa puhtaalta pöydältä.
2. **Exit 2** = "refused-but-actionable" — worktree jätetään paikalleen, käyttäjä voi yrittää manuaalisesti uudelleen ilman täysstarttia.

**Suositukseni**: 2. Linjassa AI-first-konvention "refused-but-actionable" -semantiikan kanssa, ja säästää uudelleenrakennusta.

**Miksi tämä on tärkeää**: Vaikuttaa siihen miten orchestratectl:n supervisor reagoi spawn-epäonnistumiseen ja millaista UX:ää tulee.

### Jarin kommentti
> 1,  Ainoa mitä pitää jättää on debuggausta varkten tmux kenties auki; ehkä tlle oma flagi
---

## A2. Cross-repo `create.sh` — `agent_pid_hint`-kentän muoto

**Status**: Estää MVP:n viimeisen issuen.

**Tilanne**: Skriptin JSON-vastauksessa on kenttä `agent_pid_hint`. Joskus workmux ei pysty raportoimaan agentti­prosessin PID:iä (esim. tmux send-keys ei lukita kunnes Claude todella käynnistyy).

**Vaihtoehdot**:
1. **`"agent_pid_hint": null`** — JSON-kenttä aina läsnä, arvo `null` puuttuessa.
2. **Kenttä jätetty pois** — JSON-rakenteessa ei ole kyseistä avainta lainkaan.

**Suositukseni**: 1 (null). Vähemmän yllättävä JSON-kuluttajille; supervisor uudelleen­etsii PID:n joka tapauksessa tmuxista.

**Miksi tämä on tärkeää**: Pieni mutta vakiintuva sopimus. Jos jokin moduuli alkaa olettaa kentän olevan aina läsnä, formaatin muutos myöhemmin on rikkova.

### Jarin kommentti
> 1, mutta tämä on kanssa virhetilanne josta pitäsi tulla hard failure. Ainoa mitä pitää jättää on debuggausta varkten tmux kenties auki
---

## A3. `supervisor-process` /llm-review lykätty

**Status**: Suositeltava ennen kuin `all-kinds-spawn` rakentuu päälle.

**Tilanne**: Spinoff-agentti toteutti supervisor-prosessin (MVP:n arkkitehtonisesti tärkein issue, 5 validointiporttia, prosessielinkaari, signaalit) — mutta KIELTÄYTYI ajamasta /llm-review:ta itse, koska se "ei kuluta external-LLM-tokeneita autonomisesti ilman vahvistusta". Testit + clippy + fmt ovat vihreät.

**Vaihtoehdot**:
1. **Aja `/llm-review`** muutoksiin (commitit `b1e43ce..6c50c9a`) jälkikäteen. Findingit listalle ja sovelletaan ennen all-kinds-spawn-vaihetta.
2. **Luota testeihin** ja siirrytään suoraan eteenpäin. Riskinä: race-window/signaali-virheet eivät näy testeissä mutta löytyvät tuotannossa.

**Suositukseni**: 1. Tämä on MVP:n kovin osa; review-hinta on kertaluonteinen, virhe-debugausa monelta päivältä on kallista.

**Miksi tämä on tärkeää**: 1 ihmistyöpäivän säästö nyt vs viikkojen virhe-jahdit myöhemmin.

### Jarin kommentti
> 1
---

## B1. V4 flock-latenssi 64× yli budjetin

**Status**: Muokkaa MVP:n muotoa — vaikuttaa kaikkien CLI-komentojen ja erityisesti tulevan `/orchestrate`/`/fan-out`-skaalauksen suorituskykyyn.

**Tilanne**: V4-stressitesti (50 säiettä, 1000 iteraatiota) läpäisi OIKEELLISUUDEN — kaikki tapahtuma­numerot ovat järjestyksessä eikä rivit korruptoidu. Mutta latenssi: p99 ≈ **639 ms**, oletus oli alle 10 ms. Käytännössä ~250 op/s per run, ei "tuhansia". Yksi short-lived `event create` -kutsu fsync-roundtrippineen on hidas.

**Vaihtoehdot**:
1. **Hyväksy nykyinen ratkaisu**, dokumentoi kapasiteetti­raja, ohjeista että supervisorit batchaavat omat kirjoituksensa (joka on kanonisessa Fork-1-päätöksessä mainittu fallback). MVP valmiimpi nopeammin.
2. **Toteuta `RunWriter`-API nyt** (cached seq-laskuri + batched fsync). Tämä on yksi 13 follow-up-issueesta (`runwriter-batched-append-api`). Tehtynä ennen muiden valmistumista, vältetään uudelleen­työ.

**Suositukseni**: 2. Halvempi nyt kuin myöhemmin (6 muuta CLI:tä nojaa nykyiseen API:in jo).

**Miksi tämä on tärkeää**: Jos `/orchestrate` skaalataan 50+ rinnakkais­agenttiin, nykyinen ratkaisu menee polvilleen. MVP:n jälkeen on ikävää huomata että pitää uudelleen­refaktoroida.

### Jarin kommentti
> 1
---

## B2. `--json` vs `--output text|json|jsonl` — kanoninen lippu

**Status**: Muokkaa CLI-käyttöliittymää. Vaikuttaa miten kaikki tulevat alikomennot kirjoitetaan.

**Tilanne**: AGENTS-AI-FIRST-CLI -doku nimeää virallisesti `--output text|json|jsonl` -lipun formaatin valitsijaksi. Mutta toteutettiin ekana `--json`-boolean (lyhyempi, agentti­ystävällisempi). Yksi review-spinoff (`output-flag-and-streaming`) toteuttaa `--output`:n.

**Vaihtoehdot**:
1. **`--json` pysyy lyhennyksenä** kuten `gh -o json` / `kubectl -o yaml`. Molemmat toimivat.
2. **Vain `--output`** kun se landaa, `--json` deprekoidaan ja poistetaan.

**Suositukseni**: 1. Agentit kirjoittavat `--json` nopeammin, ja se on selkeästi yleisin tapaus.

**Miksi tämä on tärkeää**: Kaikki tulevat alikomennot pitää tehdä yhteen tyyliin.

### Jarin kommentti
> 2, ja jsonl oletus
---

## B3. orchestratectl-skill — Install/Upgrade-ohjeistus

**Status**: Muokkaa skill-jakeluun liittyvää konseptia.

**Tilanne**: Pyyntösi: skill orchestratectl:lle joka toimii samalla logiikalla kuin issuectl:n skill — sisältää install/upgrade-ohjeet niin että agentti homebase-repossa osaa katsoa `orchestratectl --help`:n ja sen avulla tehdä homebasen oikeanlaiseksi. issuectl:llä on osio `## Install or upgrade issuectl`.

**Vaihtoehdot**:
1. **Tee orchestratectl-skilliin sama muoto** kuin issuectl:llä: brew, cargo, shell-installer, version-tarkistus, skill-refresh-ohjeet.
2. **Tee skill yksinkertaisempana** koska orchestratectl on agent-only — agentti ei välttämättä tarvitse "install or upgrade" -kappaletta jos jakelu hoidetaan vain yhdellä kanavalla (esim. cargo install).

**Suositukseni**: 1. Sama formaatti tulee tutuksi agenteille, ja kun MVP julkaistaan, halutaan että agentit osaavat upgradata itsensä.

**Miksi tämä on tärkeää**: Skilli on käyttöliittymä tämän työkalun pohjalle. Hyvä että agentti tunnistaa version-driftin ja osaa korjata sen.

**Lisäkysymys**: brew-tap? Cargo? Shell-installer? Yksi vai useampi kanava?

### Jarin kommentti
> 1, koska skillin pitää olla linjassa ohjelman version kanssa
---

## B4. Envelope `schema_version` -konstantin sijainti

**Status**: Arkkitehtoninen siirto, ei käytöksen muutos.

**Tilanne**: Nykyisellään on kaksi `schema_version`-konstanttia:
- `octl-cli/src/error.rs::SCHEMA_VERSION` (CLI-envelope, eli `--json`-vastauksen muoto)
- `octl-core/src/lib.rs::STATE_SCHEMA_VERSION` (on-disk state-tiedostot)

Molemmat = 1. Reviewerit (anthropic + openai) huomauttivat että CLI-envelope-kontrakti on jaettava (skill-installer, daemon, host-UI kaikki kuluttavat sitä), joten se kuuluu `octl-core`:en, ei CLI-binäärin sisään.

**Vaihtoehdot**:
1. **Siirrä `octl-core`:en heti**. Halvempi nyt.
2. **Jätä CLI-binääriin** kunnes tarvitaan toista kuluttajaa.
3. **Luo `octl-proto` -kraatti** jaetuille envelope-tyypeille (raskaampi, mutta puhtaampi siirtyminen daemoniin).

**Suositukseni**: 1.

**Miksi tämä on tärkeää**: Pieni siirto, mutta vaikeampi tehdä kun useampi alimoduuli käyttää sitä.

### Jarin kommentti
> 1
---

## B5. Re-approve eri `--issue-slug`:lla

**Status**: UX-päätös. Vaikuttaa miten agentit reagoivat virheisiin.

**Tilanne**: `spinoff approve <id> --issue-slug A` toimii ensin. Jos sitten ajetaan `spinoff approve <id> --issue-slug B`, nykyinen toteutus palauttaa hiljaa `A`:n (`idempotent_replay: true`). Useampi LLM-reviewer flagasi tämän UX-ansaksi: kutsuja luulee että `B` kiinnitettiin, mutta ei kiinnittynytkään.

**Vaihtoehdot**:
1. **Eksplicit virhe `proposal-already-approved`** kun slug eroaa. Sopii AI-first-tiukkaan input­validointiin.
2. **Erillinen `spinoff attach-issue` -verbi** slugin liittämiseen. `approve` jää yksiottoiseksi terminaaliksi.
3. **Säilytä nykyinen** käytös, dokumentoi `--help`:ssä että slug ohitetaan jos jo hyväksytty.

**Suositukseni**: 1.

**Miksi tämä on tärkeää**: Hiljaiset ohitukset = vaikeasti debugattavia ongelmia.

### Jarin kommentti
> 1
---

## B6. `--idempotency-key` -skooppi `spinoff approve`:ssa

**Status**: Dokumentaatio/UX. Liittyy B5:een.

**Tilanne**: Lippu estää duplikaattitapahtuman event-lokiin, mutta EI estä toista `issuectl new` -kutsua jos `spinoff approve` ajetaan uudestaan ilman `--issue-slug`-flagia. Eli retry voi luoda 2 issueta vahingossa.

**Vaihtoehdot**:
1. **Pidä lokaali-only**, dokumentoi rajoitus `--help`:ssä ja AGENTS.md:ssä. Ohjaa käyttäjät `--issue-slug`:iin retry-turvallisuuteen.
2. **Plumbaa avain läpi `issuectl new`:hen**. Vaatii issuectl:n saavan oman idempotency-key-lipun (ei ole tällä hetkellä).

**Suositukseni**: 1, lyhyellä tähtäimellä. Vaihtoehto 2 vaatii upstream-työtä issuectl:ssä.

**Miksi tämä on tärkeää**: Retry-turvallisuus on tärkeä autonomisille agenteille.

### Jarin kommentti
> 1, emme nyt rupea tekemään issuectl muutoksia
---

## B7. Deterministisen ID:n koodaus

**Status**: Format-sopimus joka pitäisi lukita ennen kuin skill-shim-kuluttajat alkavat luottaa siihen.

**Tilanne**: `design.md` §1.4 määrittelee deterministisen ID:n: `base32(sha256(...))[:10]` (50 bittiä). Supervisor-toteutus käytti `sha256(...)[:10]` hex (40 bittiä). Pienempi kollisioturva, eri näköiset ID:t.

**Vaihtoehdot**:
1. **Tasata toteutus speksiin** (base32). Kollisioturva paranee, ja skill-shim-kuluttajia ei vielä ole jotka olisivat juuttuneet hex-formaattiin.
2. **Päivitä speksi** hexiin. Kevyempi muutos koodissa, mutta heikompi kollisio­turva tulevaisuuteen.

**Suositukseni**: 1.

**Miksi tämä on tärkeää**: Jos myöhemmin halutaan vaihtaa, vaikuttaa kaikkiin olemassa oleviin events.jsonl-tiedostoihin.

### Jarin kommentti
> Ok, tee 1
---

## C1. Workspace-laajuinen lint-policy

**Status**: Lykättävissä — ratkeaa `ci-and-lints`-issuessa kun se ajetaan.

**Tilanne**: Pedantic-clippy päälle/pois? `#![warn(missing_docs)]` octl-coreen?

**Suositukseni**: Anna `ci-and-lints`-issuen agentin tehdä järkevä default-valinta (pedantic warn, missing_docs warn vain octl-core:lle).

### Jarin kommentti
> ok
---

## C2. Cross-platform HOME-resolvointi (Windows)

**Status**: Lykättävissä — vaihtuu jos Windows-tuki tulee MVP-tavoitteeksi.

**Tilanne**: `log_path()` lukee `$HOME`:n suoraan. Windowsissa se on `%USERPROFILE%`. `home`- tai `directories`-kraatti ratkaisi.

**Vaihtoehdot**: Lisää nyt VS. lykkää v2:een.

**Suositukseni**: Lykkää kunnes Windows-tuki on prioriteetti.

### Jarin kommentti
> lykätään, ei windows tukea vielä
---

## C3. Error envelope -lisäkentät (`details`, `hint`)

**Status**: Lykättävissä.

**Tilanne**: Reviewerit halusivat `details: Option<Value>` ja `hint: Option<String>` rikkaampia AI-toiminallisia virheilmoituksia varten. Tällä hetkellä on `code`, `message`, `invalid_value`, `expected`.

**Suositukseni**: Lisätään myöhemmin kun ekat alimoduulit nojaavat siihen. Additiivinen muutos (ei schema-bumppia).

### Jarin kommentti
> Jätetään näin. Meidän pitäsi ensin ymmärtää mitä virheilmoituksia sieltä voi tulla
---

## C4. Hyväksyntä­kriteerien tarkkuus issue-bodyissä

**Status**: Lykättävissä.

**Tilanne**: Nyky­käytäntö on että spinoff-prompti synteesi hyväksyntä­kriteerit `breakdown.md`:stä + design.md:stä. Vaihtoehtona: backfill `## Acceptance Criteria` -osiot suoraan issue-bodyihin.

**Suositukseni**: Pidetään synteesi. On toiminut.

### Jarin kommentti
> Ok
---

## C5. Schema-version check kirjoituspolulla

**Status**: Lykättävissä — kytketään `core-append-and-apply-api`-refaktoriin.

**Tilanne**: Nykyinen `write_*`-helperit hyväksyvät minkä tahansa `schema_version`-arvon. Lukupuoli validoi mutta kirjoituspuoli ei.

**Suositukseni**: Ratkaistaan kun `RunWriter`/`append_and_apply`-API landaa.

### Jarin kommentti
> ok
---

## C6. `read_all_events_checked`-variantti

**Status**: Lykättävissä — kuuluu supervisor-prosessiin, joka jo landattu.

**Tilanne**: Reviewerit halusivat tarkistuksen `seq == prev + 1` + `run_id`-yhteensopivuus, hyödyllinen reattach-replay-vaiheessa.

**Suositukseni**: Lisää tarpeen tullen. Supervisor on jo merged eikä toistaiseksi tarvitse tätä.

### Jarin kommentti
> ok
---

## C7. `node.report` -regressio at-least-once-toimituksessa

**Status**: Lykättävissä.

**Tilanne**: Vanhempi replayed `node.report` voisi periaatteessa ylikirjoittaa uudemman. MVP-design olettaa että tapahtumat sovelletaan strict `seq`-järjestyksessä.

**Suositukseni**: Trackaa jos supervisor-tasolla näkyy oikeasti out-of-order-tapaus. Per-reporter-versiokenttä lisätään silloin.

### Jarin kommentti
> ok
---

## C8. Hiljainen no-op puuttuvalla projektio­tiedostolla

**Status**: Lykättävissä.

**Tilanne**: Reducerit hiljaa palauttavat `Ok(())` jos kohde-node/manifest puuttuu. MVP-supervisor takaa että `*.created` edeltää `*.status`/`*.report`.

**Suositukseni**: Lisää `ReduceMode::Strict` myöhemmin kun reattach-replay tarvitsee.

### Jarin kommentti
> ok
---

## C9. Reducer "live mutation vs rebuild" -semantiikka

**Status**: Lykättävissä — selvenee kun `RunWriter` landaa.

**Tilanne**: Sama funktio `apply_event` palvelee sekä normaalia tapahtuma­käsittelyä että rebuild-from-scratch -käyttöä. Nimet eivät erottele.

**Suositukseni**: Refaktoroi nimillä `apply_event_live` + `rebuild_projections_from_events` kun `RunWriter`-API landaa.

### Jarin kommentti
> ok
---

## C10. `child.spawned` ennen child-hakemiston olemassaoloa

**Status**: Lykättävissä — supervisor sai jo compensating-event-strategian.

**Tilanne**: Designissa parent kirjaa `child.spawned` ennen kuin child-hakemisto luodaan. Jos hakemiston luonti epäonnistuu, parent-loki viittaa orpoon child-runiin.

**Ratkaisu jo**: supervisor odottaa child-hakemistoa 5 sekuntia, sitten emittoi `child.spawn_failed`-tapahtuman.

### Jarin kommentti
> ok
---

## C11. Orpojen supervisor PID -tiedostojen reaper

**Status**: Lykättävissä.

**Tilanne**: Jos supervisor kaatuu SIGKILL:llä, `supervisor.pid` jää orvoksi. `run reattach` kattaa eksplisiittisen tapauksen mutta jos kukaan ei reattachaa, `run list` näyttää statusta `running` ikuisesti.

**Vaihtoehdot**: `run gc` -komento VS `run list` self-probaa `kill(pid, 0)` ja downgradaa kaatuneeksi.

**Suositukseni**: `run list` self-probe — yksinkertaisempi.

### Jarin kommentti
> ok
---

## C12. Event-log rotaatio-politiikka

**Status**: Lykättävissä — out-of-MVP-scope, mutta hyvä noteerata.

**Tilanne**: events.jsonl on append-only. Pitkä `/orchestrate`-sessio ~1000 ev/s voi tuottaa 100 MB+ lokit.

**Suositukseni**: Noteeraa `validation.md`:ssä. MVP-rajat sallivat sen toistaiseksi.

### Jarin kommentti
> ok
---

## C13. PID-file race window supervisorin alussa

**Status**: Lykättävissä.

**Tilanne**: Supervisor emittoi `supervisor.started` ENNEN kuin ottaa per-run flockin. Jos kaksi supervisoria taistelee PID-tiedostosta samaan aikaan, molemmat voivat kirjoittaa `supervisor.started`-tapahtuman.

**Suositukseni**: Jätä nykyiseksi. Revisitoi jos race näkyy V5/V6-stresstesteissä.

### Jarin kommentti
> ok
---

## C14. Parent-node-olemassaolon tarkastus

**Status**: Ratkaistu kun supervisor merged — n-0001 luodaan supervisorin ensimmäisenä toimena.

**Tilanne**: Aiemmin oli huoli että `run create --parent-*` ei tarkista että parent-node oikeasti olemassa.

**Suositukseni**: Ei toimenpiteitä.

### Jarin kommentti
> ok
---

# Yhteenveto

| Kori | Kohtia | Vaikutus |
|---|---|---|
| **A** | 3 | Estävät MVP-valmistumisen — tarvitaan vastaus jotta päästään loppuun |
| **B** | 7 | Muokkaavat MVP:n muotoa — hyvä päättää pian, defaultit toimivat välillä |
| **C** | 14 | Voi lykätä — suurin osa kytkeytyy follow-up-issueihin |

**Toiminta­minimi MVP:n loppuunsaattamiseksi**: vastaa A1-A3 ja B1-B4. Loput voivat tulla erikseen.

Lähetä tämä takaisin kommentteinesi (tai tee inline-muokkaukset tiedostoon) niin jatkan.
