#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
index="$root/index.json"
v0=57f6dfb83401694399b363de5d3aa88e4541a22c
v1=7e93bd6195fbaf6de0b43d9161228ae2373ab5d1

jq -e --arg v0 "$v0" --arg v1 "$v1" '
  .schema_version == 1 and .overall == "pass" and
  .burned_version == "0.6.0" and .burned_commit == $v0 and
  .published_version == "0.6.1" and .published_commit == $v1
' "$index" >/dev/null
manifest="$(mktemp)"; actual="$(mktemp)"
trap 'rm -f "$manifest" "$actual"' EXIT
jq -r '.artifacts[].path' "$index" | LC_ALL=C sort >"$manifest"
(cd "$root" && find . -type f ! -name index.json -print | sed 's#^./##' | LC_ALL=C sort) >"$actual"
cmp "$manifest" "$actual"
while IFS=$'\t' read -r expected bytes path; do
  [[ "$(wc -c <"$root/$path" | tr -d ' ')" == "$bytes" ]]
  [[ "$(shasum -a 256 "$root/$path" | awk '{print $1}')" == "$expected" ]]
done < <(jq -r '.artifacts[] | [.sha256,.bytes,.path] | @tsv' "$index")

jq -e --arg v0 "$v0" --arg v1 "$v1" '
  .v0_6_0.commit == $v0 and
  ([.v0_6_0.public_crates[].status] | all(. == "absent_http_404")) and
  .v0_6_0.github_release == "absent_http_404" and
  .v0_6_0.canonical_formula == "absent_from_two-commit-tap-history" and
  .v0_6_0.immutable_refs.tag_commit_sha == $v0 and
  .v0_6_0.immutable_refs.authorization_commit_sha == $v0 and
  .v0_6_1.commit == $v1 and
  (.v0_6_1.public_crates | length == 3) and
  ([.v0_6_1.public_crates[] | (.version == "0.6.1" and .yanked == false and .checksum == .download_sha256 and .source_commit == $v1)] | all) and
  .v0_6_1.public_crates[1].exact_pin.requirement == "=0.6.1" and
  .v0_6_1.public_crates[2].exact_pin.requirement == "=0.6.1" and
  .v0_6_1.github_release.target_commitish == $v1 and
  (.v0_6_1.github_release.assets | length == 13) and
  .v0_6_1.immutable_refs.tag_commit_sha == $v1 and
  .v0_6_1.immutable_refs.authorization_commit_sha == $v1 and
  .homebrew.canonical_tap_head == "c9e68594340b2b775d23159a3545d53f15306471" and
  (.homebrew.canonical_tap_history | length == 2) and
  .homebrew.old_tap_head == "85ce830378f38cf17283efddd966d5754354e403" and
  ([.install_checks[].result] | all(. == "pass"))
' "$root/public-state.json" >/dev/null

jq -e '
  .run_id == "01M1TNW3SMN0XA347D1MG4518R" and .status == "abandoned" and
  .last_seq == 38 and .applied_seq == 38
' "$root/journal-v0.6.0.json" >/dev/null
jq -e '
  .run_id == "01M1TTRXNXK6FPQJK3F92B9AXA" and .status == "completed" and
  .last_seq == 51 and .applied_seq == 51 and
  (.verified | length == 5) and ([.verified[]] | all(. == "matches"))
' "$root/journal-v0.6.1.json" >/dev/null
jq -e '
  ([.runs[] | select(.repository == "jarimustonen/taskfleet") | .databaseId] | sort) == ([34016341659,34016740702,34016740704,34020144153,34020495260,34020495272] | sort) and
  ([.runs[] | select(.databaseId == 34016740702 or .databaseId == 34016740704) | .conclusion] | all(. == "failure")) and
  ([.runs[] | select(.databaseId == 34016341659 or .databaseId == 34020144153 or .databaseId == 34020495260 or .databaseId == 34020495272 or .databaseId == 34021860186 or .databaseId == 34022689350) | .conclusion] | all(. == "success"))
' "$root/workflows.json" >/dev/null
grep -Fx 'result=pass' "$root/homebrew-install-result.txt" >/dev/null
grep -Fx 'installed_formulae=taskfleet' "$root/homebrew-install-result.txt" >/dev/null
grep -Fx 'orchestratectl_alias=absent' "$root/homebrew-install-result.txt" >/dev/null
grep -Fx 'uninstall_residue=absent' "$root/homebrew-install-result.txt" >/dev/null
! grep -R '/Users/' "$root" --exclude=index.json >/dev/null
printf 'R10 final evidence verified: burned v0.6.0, published v0.6.1\n'
