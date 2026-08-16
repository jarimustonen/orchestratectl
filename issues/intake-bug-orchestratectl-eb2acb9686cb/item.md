---
created: 2026-08-16
updated: 2026-08-16
type: bug
reporter: jari
status: cannot-reproduce
priority: normal
labels:
- via:agent-homebase-wrapup
- needs-triage
closed: 2026-08-16
closed_by: claude
---

# run wait --output json returns null terminal fields for a settled run

## Description

run wait --output json returns null terminal fields for a settled run

## Observed

`orchestratectl run wait <run-id> --output json` returns an envelope whose `.data`
terminal fields are all `null` even when the run has settled:

    $ orchestratectl run wait 01m02nchnq8awsdmwbz2evz7c4 --output json | jq -c \
        '{status: .data.status, landed: .data.landed, landed_method: .data.landed_method, summary: .data.summary}'
    {"status":null,"landed":null,"landed_method":null,"summary":null}

The exit code was 0 (settled), but none of the terminal fields were populated. Running
`orchestratectl run show <run-id> --output json` immediately afterward returned them fully
populated:

    {"status":"done","landed":true,"landed_method":"report-marker"}

## Expected

`run wait --output json` should carry the settled run's terminal fields (`status`,
`landed`, `landed_method`, and the folded-in report `summary`) in `.data` — the
worktree-spinoff skill documents `run wait` as folding the terminal report summary in, so a
caller shouldn't need a follow-up `run show`. Either the fields belong at a different JSON
path than `.data.status` (in which case document it), or they are genuinely not populated
(the bug).

## Impact

Every settled-run check this session had to fall back to a second `run show` call to read
`status`/`landed`. Low severity (workaround exists) but it defeats the point of `run wait`
folding the summary in, and the null-vs-populated split between the two commands is
surprising. Observed with orchestratectl 0.1.8 on macOS (darwin 25.5.0), 2026-08-16.

## Resolution

### 2026-08-16T15:32:29Z · @claude

Ei bugi. `run wait` palauttaa listan ajoja (`data.runs[]`), koska se voi odottaa montaa yhtä aikaa; raportin kysely haki yksittäistä kenttää suoraan (`.data.status`), jota ei ole. Oikea kysely `.data.runs[0].status` palauttaa kaikki kentät. Verifioitu koodista (run/wait.rs: WaitData.runs). Jäännös: run show palauttaa yhden ajon, run wait listan — muotoero on dokumentaatioasia, ei koodivirhe.
