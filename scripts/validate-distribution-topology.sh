#!/usr/bin/env bash
# Validate the Taskfleet-only cargo-dist topology without publishing.
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
cd "$repo_root"

jq -e '
  .schema_version == 1 and .activation == "ready" and
  .cargo_dist.version == "0.28.2" and
  .cargo_dist.apps == ["taskfleet"] and
  .cargo_dist.tap == "jarimustonen/homebrew-taskfleet" and
  .cargo_dist.trigger == "tag-push" and
  .cargo_dist.pr_run_mode == "skip" and
  .source_repository.current == "jarimustonen/taskfleet"
' release/taskfleet-distribution.json >/dev/null

grep -F 'cargo-dist-version = "0.28.2"' dist-workspace.toml >/dev/null
grep -F 'pr-run-mode = "skip"' dist-workspace.toml >/dev/null
grep -F 'dispatch-releases = false' dist-workspace.toml >/dev/null
grep -F 'tap = "jarimustonen/homebrew-taskfleet"' dist-workspace.toml >/dev/null
if grep -Eq '^\[\[dist\.extra-artifacts\]\]' dist-workspace.toml; then
  echo "Taskfleet distribution must not carry transition artifacts" >&2
  exit 2
fi

cargo metadata --locked --no-deps --format-version 1 | jq -e '
  ([.packages[].name] | sort) == ["taskfleet", "taskfleet-core"] and
  ([.packages[] | .targets[] | select(.kind == ["bin"]) | .name]) == ["taskfleet"] and
  ([.packages[] | select(.name == "taskfleet") | .dependencies[] |
    select(.name == "taskfleet-core" and (.req | startswith("=")))] | length) == 1
' >/dev/null

grep -F 'repository: "jarimustonen/homebrew-taskfleet"' .github/workflows/release.yml >/dev/null
printf 'Taskfleet distribution topology verified\n'
