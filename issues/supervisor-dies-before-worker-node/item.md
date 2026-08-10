---
created: 2026-08-10
updated: 2026-08-10
type: bug
status: open
priority: normal
labels: [supervisor, spinoff]
---

# supervisor died before creating any worker node (spinoff pending→stalled, 0 work)

## Description

## Comments

### 2026-08-10T11:29:40Z · @jari

Havaittu 2026-08-10 (3dbear-monorepo, /stint rinnakkaisaalto). Run 01kznacgtjecha8p17b31k119s (kind=spinoff, title=simuna-ai-block-gated). `run wait` palautti: status=pending, stalled=true, landed=false, method=unverified, error='supervisor died before creating any worker node'. Run-dir sisälsi vain events.jsonl + manifest.json — EI worker-branchia, EI committeja, issue jäi 'open'. Eli supervisor kuoli ENNEN worker-noden luontia → 0 työtä tehty. Vaikutus: puhdas re-spawn riittää (ei harvest), mutta run jää pending-tilaan ja näyttää jumittuneelta kunnes joku huomaa. Toistui rinnakkaisaallossa jossa oli useita supervisoreita + FS-contentiota samaan aikaan (git index.lock -kilpailua havaittu samassa ikkunassa). Korjausehdotus: (a) supervisor retry/backoff worker-noden luonnissa jos ensiyritys kuolee, TAI (b) run create palauttaa virheen jos supervisor ei ehdi luoda worker-nodea N sekunnissa (ei jää pending-valheeseen). Liittyy vanhaan: supervisor-spawn-fails-silently-at-run-create.
