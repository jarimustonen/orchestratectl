---
created: 2026-08-21
updated: 2026-08-23
type: bug
reporter: pi
status: untriaged
priority: normal
provenance: other
provenance_detail: orchestrator runtime observation
source_ref: stint:2026-08-21/deploy-first-version-stale
---

# First version check after cargo install can report the replaced binary's stale commit

## Description

During the 2026-08-21 orchestratectl stint, the repository-authorized local deploy sequence failed twice in the same way.

After `cargo install --path crates/octl-cli --force --locked` completed successfully and replaced `~/.cargo/bin/orchestratectl`, the immediately following `orchestratectl version --output json` reported an old build commit and caused the mandatory commit-equality gate to fail. Running the exact same version command moments later, without reinstalling or modifying any file, reported the newly installed expected commit.

First occurrence after installing HEAD `72883396a0dc2e47e1fab6abc2efd0877e488fcc`: the chained deploy exited 1 before skill installation, but a follow-up version call reported `72883396a0dc2e47e1fab6abc2efd0877e488fcc` correctly.

Second occurrence after installing HEAD `fa3a81a239f62a79d688df66043ec4ab715890d2`:

```text
Installing orchestratectl v0.4.1 (...)
Finished release profile
Replacing /Users/jari/.cargo/bin/orchestratectl
Replaced package ...
expected=fa3a81a239f62a79d688df66043ec4ab715890d2
actual=8777b2e3c5b891abf396c6486c9e81e17ffcfe85
```

Immediately afterward, both `orchestratectl version` and `./target/release/orchestratectl version` reported `fa3a81a239f62a79d688df66043ec4ab715890d2`. Re-running the provenance check succeeded, followed by skill install and doctor (1131 ok / 0 warn / 0 fail).

The deploy policy intentionally treats commit equality as load-bearing, so retrying silently is not an acceptable permanent workaround. Determine whether this is shell command hashing, APFS rename/exec visibility, Cargo installation behavior, or another stale executable-selection race, and provide a bounded verification protocol that cannot accept a genuinely stale binary.

## Comments

### 2026-08-23T07:45:55Z · @orchestrator

Recurrence on 2026-08-23 after installing HEAD f268f884035391888b6ec9984bd84c2fa3ac7954. The authorized chained deploy completed `cargo install --path crates/octl-cli --force --locked`, then exited 1 at the immediate expected-vs-actual commit equality test before `ls`, skill install, or doctor. The chain did not print the mismatching actual value. An explicit diagnostic probe immediately afterward resolved `/Users/jari/.cargo/bin/orchestratectl` and reported the expected f268f884 commit without any reinstall or file mutation. The failed attempt remains a failed deploy; this note records the recurrence before any fresh attempt.
