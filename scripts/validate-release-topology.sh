#!/usr/bin/env bash
# Single structural authority for the checked-in Taskfleet release topology.
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
topology="$repo_root/release/taskfleet-release.json"
[[ -f "$topology" ]] || { echo "release topology not found: $topology" >&2; exit 2; }
jq -e '
  (keys | sort) == ["activation","crates_io","distribution","owners","repository","schema_version"] and
  .schema_version == 1 and (.activation == "blocked-r8-r9-r10" or .activation == "ready") and
  (.repository | test("^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$")) and
  .owners == ([.owners[]] | unique | sort) and (.owners | length > 0) and
  (.crates_io | keys | sort) == ["legs","workflow"] and .crates_io.workflow == "publish-crates.yml" and
  .crates_io.legs == [
    {package:"taskfleet-core",manifest:"crates/taskfleet-core/Cargo.toml",depends_on:null},
    {package:"taskfleet",manifest:"crates/taskfleet/Cargo.toml",depends_on:"taskfleet-core"},
    {package:"orchestratectl",manifest:"compat/orchestratectl/Cargo.toml",depends_on:"taskfleet"}
  ] and
  .distribution == [
    {package:"taskfleet",registry:"gh-releases",workflow:"release.yml"},
    {package:"taskfleet",registry:"homebrew",workflow:"release.yml"}
  ]
' "$topology" >/dev/null || {
  echo "release topology is not the admitted five-leg Taskfleet graph: $topology" >&2
  exit 2
}
printf '%s\n' "$(jq -er .repository "$topology")"
