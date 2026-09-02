#!/usr/bin/env bash
# Mutating distribution boundary: all later gates and canonical identity required.
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
cd "$repo_root"

./scripts/validate-release-topology.sh >/dev/null
jq -e '
  .activation == "ready" and
  .repository == "jarimustonen/taskfleet"
' release/taskfleet-release.json >/dev/null || {
  echo "release activation requires ready canonical Taskfleet repository topology" >&2
  exit 2
}
jq -e '
  .activation == "ready" and
  .source_repository.current == "jarimustonen/taskfleet" and
  .source_repository.after_r9 == "jarimustonen/taskfleet" and
  .cargo_dist.trigger == "tag-push" and
  .cargo_dist.tap == "jarimustonen/homebrew-taskfleet"
' release/taskfleet-distribution.json >/dev/null || {
  echo "release activation requires completed R8/R9 distribution topology" >&2
  exit 2
}
if grep -Eq '^dispatch-releases[[:space:]]*=[[:space:]]*true' dist-workspace.toml; then
  echo "release activation requires cargo-dist tag dispatch" >&2
  exit 2
fi
grep -A12 '^on:' .github/workflows/release.yml | grep -Eq '^[[:space:]]+push:' || {
  echo "release activation requires a generated cargo-dist tag trigger" >&2
  exit 2
}
grep -A12 '^on:' .github/workflows/release.yml |
  grep -F -- "- '**[0-9]+.[0-9]+.[0-9]+*'" >/dev/null || {
  echo "release activation requires the exact cargo-dist version-tag pattern" >&2
  exit 2
}
cargo metadata --locked --no-deps --format-version=1 | jq -e '
  [.packages[] | select(.name == "taskfleet-core" or .name == "taskfleet" or
    .name == "orchestratectl") |
    (.repository == "https://github.com/jarimustonen/taskfleet" and
     .homepage == "https://github.com/jarimustonen/taskfleet")] | length == 3 and all
' >/dev/null || {
  echo "release activation requires canonical Cargo repository/homepage metadata" >&2
  exit 2
}

printf 'Taskfleet release activation verified\n'
