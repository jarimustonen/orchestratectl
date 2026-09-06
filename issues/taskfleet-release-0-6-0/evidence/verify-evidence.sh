#!/usr/bin/env bash
set -euo pipefail
repo_root="$(git rev-parse --show-toplevel)"
evidence="$repo_root/issues/taskfleet-release-0-6-0/evidence"
index="$evidence/index.json"
expected_commit=23f7fcf6d9de969300dce560538ce1f3a11f2a2a
expected_tree=fbabcec6898d9529758eb79f5f42182bd866b9e4

jq -e --arg commit "$expected_commit" --arg tree "$expected_tree" '
  .schema_version == 1 and .overall == "pass" and
  .tested_commit == $commit and .tested_tree == $tree and
  .phase_a_complete == true and .phase_b_candidate_complete == true and
  .phase_c_authorized == false
' "$index" >/dev/null
[[ "$(git rev-parse "$expected_commit^{tree}")" == "$expected_tree" ]]

manifest="$(mktemp)"
actual="$(mktemp)"
trap 'rm -f "$manifest" "$actual"' EXIT
jq -r '.artifacts[].path' "$index" | LC_ALL=C sort >"$manifest"
(
  cd "$repo_root/issues/taskfleet-release-0-6-0"
  find evidence -type f ! -path evidence/index.json -print | LC_ALL=C sort
) >"$actual"
cmp "$manifest" "$actual"

while IFS=$'\t' read -r expected path; do
  [[ "$(shasum -a 256 "$repo_root/issues/taskfleet-release-0-6-0/$path" | awk '{print $1}')" == "$expected" ]]
done < <(jq -r '.artifacts[] | [.sha256,.path] | @tsv' "$index")

printf 'R10 evidence verified for %s (%s)\n' "$expected_commit" "$expected_tree"
