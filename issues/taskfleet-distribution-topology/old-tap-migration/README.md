# Sealed old-tap migration (do not push before R11)

This directory is the atomic future migration for
`jarimustonen/homebrew-orchestratectl`. It was prepared against exact public
head `85ce830378f38cf17283efddd966d5754354e403` and is **not activated**.
The live old tap still contains the truthful 0.5.1 formula.

The one patch deletes `Formula/orchestratectl.rb` and adds
`tap_migrations.json` mapping the renamed formula to the full canonical identity
`jarimustonen/taskfleet/taskfleet`. `manifest.json` pins the input formula blob
and expected output tree. `scripts/test-distribution-topology.sh` applies it to a
disposable clone and exercises Homebrew 6.0.21 cross-tap resolution without
installing a formula or touching the user's taps.

R11 may apply this patch only after R10 has published and verified the canonical
formula. Before applying, require the exact `required_head`; if the old tap has
moved, stop and regenerate/review rather than using a three-way apply. Commit the
delete and metadata addition together. Never configure cargo-dist to write this
old tap again. This R7 artifact does not add `formula_renames.json`: the disposable
full-identity Formulary drill resolves without it, but R11 must still test the
installed-keg migrator/receipt path and add reviewed new-tap rename metadata if
that separate path requires it.
