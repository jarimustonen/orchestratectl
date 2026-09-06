---
created: 2026-08-16
updated: 2026-08-20
type: task
status: done
priority: high
lane: skills
lane_seq: 50
closed: 2026-08-20
closed_by: pi
commits:
- hash: acc2859
  summary: remove user-specific facts from shipped artifacts
---

# Audit: no user-specific facts in a public artifact

## Description

## Description

This repository is **public** and distributed. Audit it for user-specific facts that must not
ship in a public artifact, and move any that exist into user configuration.

### The rule

Maintainer rule, 2026-08-16: *a public repo must not reference user-specific things at all;
user-specific things belong in user config.*

A publicly-distributed artifact MUST NOT contain personal account handles, private
repo/project names, personal filesystem-layout conventions, hostnames, internal URLs, or
org-internal identifiers — not in source, **not in built-in defaults**, not in generated
scaffold/template output, not in installed skill content, not in docs, not in tests or
fixtures.

**Key point, and the one that caused the original defect: overridability does not launder a
user-specific default.** "Every value is configurable" is not a defence — an unset default is
still whatever ships in the package. The correct built-in default is neutral/absent, with an
actionable error naming the config key or env var to set. Never a silent guess at someone's
environment.

### Why this issue exists

`project-canon` — the family's own conformance tool — shipped these to crates.io in `0.1.1`
and `0.2.0`:

```rust
gh_account: "jarimustonen".to_string(),
repo_root:  "~/Sources".to_string(),
const DEFAULT_FAMILY_TOOLS: [&str; 7] = [
    "issuectl", "taskfleet", "crmctl", "tilictl", "ossctl", "intakectl", "glasspad",
];
```

Three of those seven are **private** repositories, so a public crate disclosed the names of
private projects. The defect survived a design pass that explicitly considered portability,
because the reasoning stopped at "it's overridable".

Every public tool in the family is plausibly exposed to the same class of defect, so each is
being audited rather than assumed clean.

## What to check

Grep the whole repo — source, defaults, templates, generated output, docs, README, tests,
golden fixtures, skill content — for:

- the maintainer's account handle / name / email
- names of **private** family repos: `crmctl`, `tilictl`, `intakectl`, `aggountant`
  (public siblings are fine to reference where genuinely relevant, e.g. a real dependency)
- personal path conventions (`~/Sources`, `/Users/<name>`, personal machine hostnames)
- internal URLs, internal service names, org-internal identifiers
- any built-in default that encodes one person's environment rather than a neutral value

## Acceptance

- No user-specific value anywhere in the shipped artifact (per the list above).
- Any environment-specific value the tool genuinely needs is read from user config, with a
  neutral built-in default and an actionable error when a required value is unset.
- Fixtures, examples, and docs use obviously fictional values.
- The maintainer's own setup still works, expressed through user config outside the repo.
- **If the audit finds nothing, close the issue saying so** — a recorded clean result is the
  point; it is what makes the family-wide sweep meaningful.

## Comments

Filed 2026-08-16 from `project-canon` as part of a family-wide sweep after the leak above was
found. Companion work in `project-canon`: `portable-neutral-defaults` (the concrete cleanup)
and `canon-no-user-specifics` (promoting this rule to a canon section with a mechanical
`doctor` check, so it is enforced rather than remembered). Once that check ships, this audit
becomes automated — this issue is the one-time manual pass.

### 2026-08-16T17:21:11Z · @claude

SCOPING (triage 2026-08-16, ei vielä auditointi — vain laajuuden kartoitus lanen valintaa varten).

Grep shipatusta artefaktista (pois lukien issues/, TODO.md, history/) hakusanoilla
`jarimustonen|/Users/jari|~/Sources|crmctl|tilictl|intakectl|aggountant|jari.mustonen|jari@`
osuu 19 tiedostoon:

**Mukana toimitettavat SKILL-templaatit (5)** — nämä ovat `skills`-lanen kuumia tiedostoja:
`taskfleet-run-overview`, `taskfleet-spawn-spinoff`, `taskfleet-overview`, `worktree-merge`, `worktree-spinoff`.

**Juuridokumentit (7):** AGENTS.md, CLAUDE.md, README.md, CONTRIBUTING.md, SECURITY.md, CHANGELOG.md, OSS-RELEASE.md
**Rakennus/julkaisu (4):** Cargo.toml, dist-workspace.toml, .github/workflows/{publish-crates,release}.yml
**Muut (3):** .github/ISSUE_TEMPLATE/config.yml, .claude/skills/issue/SKILL.md, .codex/prompts/issue.md

Nolla osumaa `crates/*/src/`-hakemistoissa — eli sisäänrakennettuja oletusarvoja ei tämän greppauksen
perusteella ole saastunut, toisin kuin project-canonissa. Se on hyvä merkki muttei vielä todiste;
varsinainen auditointi tarkistaa myös testit ja fixturet.

TÄRKEÄ EROTTELU auditoijalle: iso osa osumista on **legitiimejä julkisia koordinaatteja**, ei vuotoja —
`github.com/jarimustonen/taskfleet` (repon oikea osoite Cargo.tomlissa, README-badgeissa,
release-workflowissa) ja Homebrew-tap `jarimustonen/taskfleet`. Näitä ei pidä poistaa.
Huolestuttava luokka on: yksityisten repojen nimet (crmctl, tilictl, intakectl, aggountant),
henkilökohtaiset polkukäytännöt (~/Sources, /Users/jari) ja sähköpostiosoitteet.

LANE: `skills`, koska viiden SKILL-templaatin päällekkäisyys tarkoittaa ettei tätä saa ajaa
rinnakkain skills-lanen kanssa. Sijoitettu lanen loppuun (`lane_seq 50`), jotta se ei estä
lanen halpoja korjauksia — mutta huomaa että ne korjaukset koskevat samoja tiedostoja, joten
järjestys on tarkoituksellinen, ei mielivaltainen.

### 2026-08-16T17:21:39Z · @claude

KORJAUS edelliseen muistiinpanoon: `lane_seq 50` EI aseta tätä lanen viimeiseksi. `issuectl dag` järjestää ensin prioriteetin mukaan ja vasta sitten `lane_seq`:llä, joten `priority: high` nostaa tämän lanen toiseksi (heti `spinoff-report-fields-null`:n jälkeen, joka on myös high ja voittaa tasatilanteen pienemmällä lane_seq:llä).

Se on puolustettavissa — julkisen artefaktin vuototarkistus on aidosti kiireellinen — mutta se EI ole se järjestys jota tavoittelin. Jos haluat sen lanen loppuun halpojen korjausten jälkeen, laske prioriteetti: `issuectl update audit-no-user-specifics --priority normal`. Sama mekanismi selittää miksi `lifecycle`-lanen kärki on `uncommonly-fuzzy-swing` eikä `shell-quote-dedup`.

## Resolution

### 2026-08-20T06:50:06Z · @pi

Completed the full shipped-artifact audit. Source, metadata/defaults, root docs, release/scaffold configuration, generated cargo-dist workflow, tests/fixtures, bundled skill templates, installed-skill content, and package manifests were inspected. Genuine user-specific facts were removed: personal authorship/contact metadata, private repository/provenance names, personal source-root assumptions, and the private runner hostname. The only surviving account-handle occurrences in shipped scope are required public coordinates for the GitHub repositories and Homebrew taps; reserved example emails, $HOME paths, vendor bot identity, and canonical Linuxbrew paths were classified as neutral. cargo package produced 26-file taskfleet-core and 163-file taskfleet archives; both archive scans were clean. A release build using the generated HOME/GITHUB_WORKSPACE remaps passed a strings scan with no audited values, and GitHub private vulnerability reporting was enabled and verified. /llm-review and /assess-findings were completed; confirmed localized findings were applied (preserve RUSTFLAGS, remap both build roots, ignore regenerated canon copies, verify the private reporting channel, restore public ADR provenance, and use portable $HOME examples). The skill insta loop completed with zero snapshot changes to accept, and every snapshot was therefore unchanged. The exact green gate passed: fmt, clippy -D warnings, release nextest (1000 passed, 1 skipped), doctests, and rustdoc -D warnings. Tool-sensitive suites also passed under a stripped PATH (44/44). The tracked issue archive/TODO/history remain outside the issue-defined shipped scope and are absent from Cargo packages, dist assets, Homebrew output, and installed skills.
