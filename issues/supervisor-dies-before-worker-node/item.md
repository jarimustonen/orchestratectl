---
created: 2026-08-10
updated: 2026-08-10
type: bug
status: fixed
priority: normal
labels: [supervisor, spinoff]
commits:
- hash: 5678bf3
  summary: surface stillborn runs in run list with age-gate and first-class stillborn DTO flag
closed: 2026-08-10
---

# supervisor died before creating any worker node (spinoff pending→stalled, 0 work)

## Description

## Comments

### 2026-08-10T11:29:40Z · @jari

Havaittu 2026-08-10 (3dbear-monorepo, /stint rinnakkaisaalto). Run 01kznacgtjecha8p17b31k119s (kind=spinoff, title=simuna-ai-block-gated). `run wait` palautti: status=pending, stalled=true, landed=false, method=unverified, error='supervisor died before creating any worker node'. Run-dir sisälsi vain events.jsonl + manifest.json — EI worker-branchia, EI committeja, issue jäi 'open'. Eli supervisor kuoli ENNEN worker-noden luontia → 0 työtä tehty. Vaikutus: puhdas re-spawn riittää (ei harvest), mutta run jää pending-tilaan ja näyttää jumittuneelta kunnes joku huomaa. Toistui rinnakkaisaallossa jossa oli useita supervisoreita + FS-contentiota samaan aikaan (git index.lock -kilpailua havaittu samassa ikkunassa). Korjausehdotus: (a) supervisor retry/backoff worker-noden luonnissa jos ensiyritys kuolee, TAI (b) run create palauttaa virheen jos supervisor ei ehdi luoda worker-nodea N sekunnissa (ei jää pending-valheeseen). Liittyy vanhaan: supervisor-spawn-fails-silently-at-run-create.

### 2026-08-10T12:25:18Z · @jari

TOISTUI 2× LISÄÄ samassa /stint-sessiossa 2026-08-10 (yht. 3×): auto-provision-observer (01kzns7mcx) ja auto-provision-observer-v2 (01kznsjt4y) — molemmat status=pending stalled=true nodes=0, vain run.created-event, supervisor ei elossa, worktree-branch luotu base-mainista (0 committia). KORRELAATIO: tapahtui kun kone oli saturoitunut (git worktree remove + taskfleet run list jumittuivat 120s timeouttiin, useita rinnakkaisia supervisoreita + FS-contentiota). Hypoteesi: supervisor kuolee worker-noden luontivaiheessa kun järjestelmä on kuormittunut (fork/exec tai FS-lukko epäonnistuu hiljaa). Re-spawn EI auta jos kuorma pysyy — kuolee heti uudestaan. Cleanup itsekin jumittui (worktree remove timeout → dir jäi, prune ei poistanut, piti rm -rf käsin). Ehdotus vahvistuu: (a) supervisor retry/backoff worker-noden luonnissa, (b) run create fail-fast ei-elossa-supervisorilla (ei jää pending-valheeseen), (c) run create backpressure/jono kun N supervisoria jo elossa.

