#!/usr/bin/env bash
# Exercise the production ruleset filter and its fail-closed diagnostics.
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
tmp="$(mktemp -d "${TMPDIR:-/tmp}/taskfleet-policy-fixture.XXXXXX")"
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/bin"
ln -s "$(command -v jq)" "$tmp/bin/jq"
ln -s "$(command -v mktemp)" "$tmp/bin/mktemp"
ln -s "$(command -v grep)" "$tmp/bin/grep"
ln -s "$(command -v tail)" "$tmp/bin/tail"
ln -s "$(command -v rm)" "$tmp/bin/rm"
cat >"$tmp/bin/gh" <<'STUB'
#!/bin/sh
id="${2##*/}"
case "${GH_FIXTURE_MODE:-valid}:$id" in
  api-failure:*) echo 'gh: Resource not accessible by integration (HTTP 403)' >&2; exit 1 ;;
  malformed:*) printf '%s\n' '{not-json'; exit 0 ;;
  redacted:*)
    jq -n --argjson id "$id" '{id:$id,name:"redacted",target:"tag",enforcement:"active",
      conditions:{ref_name:{exclude:[],include:["refs/tags/**"]}},rules:[]}' ;;
  mismatch:*)
    jq -n --argjson id "$id" '{id:$id,name:"wrong SECRET_FIXTURE_VALUE",target:"tag",enforcement:"active",
      conditions:{ref_name:{exclude:[],include:["refs/tags/**"]}},rules:[],bypass_actors:[]}' ;;
  valid:22234415)
    jq -n '{id:22234415,name:"Taskfleet release tags",target:"tag",enforcement:"active",
      conditions:{ref_name:{exclude:[],include:["refs/tags/**"]}},
      rules:[{type:"creation"},{type:"update"},{type:"deletion"},{type:"non_fast_forward"}],
      bypass_actors:[{actor_id:5,actor_type:"RepositoryRole",bypass_mode:"always"}]}' ;;
  valid:22234417)
    jq -n '{id:22234417,name:"Taskfleet release authorization refs",target:"branch",enforcement:"active",
      conditions:{ref_name:{exclude:[],include:["refs/heads/taskfleet-release-authorizations/**"]}},
      rules:[{type:"creation"},{type:"update"},{type:"deletion"},{type:"non_fast_forward"}],
      bypass_actors:[{actor_id:5,actor_type:"RepositoryRole",bypass_mode:"always"}]}' ;;
  *) exit 97 ;;
esac
STUB
chmod +x "$tmp/bin/gh"

run_policy() {
  env -i PATH="$tmp/bin:/usr/bin:/bin" GH_FIXTURE_MODE="${1:-valid}" \
    "$repo_root/scripts/verify-release-github-policy.sh"
}
run_policy valid >/dev/null

assert_failure() {
  local mode="$1" expected="$2" output status
  set +e
  output="$(run_policy "$mode" 2>&1)"
  status=$?
  set -e
  [[ "$status" -ne 0 ]] || { echo "$mode policy fixture unexpectedly passed" >&2; exit 1; }
  grep -F "$expected" <<<"$output" >/dev/null || {
    echo "$mode policy fixture lacked diagnostic: $expected" >&2
    printf '%s\n' "$output" >&2
    exit 1
  }
  if grep -F 'SECRET_FIXTURE_VALUE' <<<"$output" >/dev/null; then
    echo "$mode policy fixture leaked API response content" >&2
    exit 1
  fi
}
assert_failure api-failure 'API lookup failed (HTTP 403)'
assert_failure malformed 'API returned an unexpected JSON shape'
assert_failure redacted 'API response omits privileged bypass_actors'
assert_failure mismatch 'does not match the required protected policy'

printf 'Taskfleet live release policy fixtures passed with %s\n' "$(jq --version)"
