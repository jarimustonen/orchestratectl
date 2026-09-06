#!/usr/bin/env bash
# Verify the live rules that prevent ordinary writers/workflows from creating
# release tags or forging/moving wrapper authorization receipts.
set -euo pipefail
repo="jarimustonen/taskfleet"
check_ruleset() {
  local id="$1" name="$2" target="$3" include="$4" json
  json="$(gh api "repos/$repo/rulesets/$id")" || exit 1
  jq -e --argjson id "$id" --arg name "$name" --arg target "$target" --arg include "$include" '
    .id == $id and .name == $name and .target == $target and .enforcement == "active" and
    .conditions.ref_name == {exclude:[],include:[$include]} and
    .bypass_actors == [{actor_id:5,actor_type:"RepositoryRole",bypass_mode:"always"}] and
    ([.rules[].type] | sort) == (["creation","deletion","non_fast_forward","update"] | sort)
  ' <<<"$json" >/dev/null
}
check_ruleset 22234415 "Taskfleet release tags" tag 'refs/tags/**'
check_ruleset 22234417 "Taskfleet release authorization refs" branch \
  "refs/heads/taskfleet-release-authorizations/**"
printf 'Taskfleet live release rulesets verified\n'
