#!/usr/bin/env bash
# Non-mutating Taskfleet distribution policy test.
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
cd "$repo_root"

./scripts/validate-distribution-topology.sh >/dev/null
workflow=.github/workflows/release.yml
grep -A12 '^on:' "$workflow" | grep -Eq '^[[:space:]]+push:'
if grep -A12 '^on:' "$workflow" | grep -Eq 'pull_request:|workflow_dispatch:'; then
  echo "release workflow must be tag-only" >&2
  exit 2
fi
if grep -F 'secrets: inherit' "$workflow" >/dev/null; then
  echo "release workflow must not inherit secrets" >&2
  exit 2
fi
[[ "$(grep -c 'repository: "jarimustonen/homebrew-taskfleet"' "$workflow")" == 1 ]]
grep -F 'name = "taskfleet"' crates/taskfleet/Cargo.toml >/dev/null
printf 'Taskfleet distribution policy tests passed\n'
