#!/usr/bin/env bash
# Verify the live rules that prevent ordinary writers/workflows from creating
# release tags or forging/moving wrapper authorization receipts.
set -euo pipefail
repo="jarimustonen/taskfleet"

check_ruleset() {
  local id="$1" name="$2" target="$3" ref_pattern="$4" json api_error http_status
  api_error="$(mktemp "${TMPDIR:-/tmp}/taskfleet-ruleset-api.XXXXXX")"
  if ! json="$(gh api "repos/$repo/rulesets/$id" 2>"$api_error")"; then
    # Report only the HTTP status emitted by gh. Never echo an API response,
    # request headers, or credential-bearing environment data.
    http_status="$(grep -Eo 'HTTP [0-9]{3}' "$api_error" | tail -1 || true)"
    rm -f "$api_error"
    echo "release ruleset $id API lookup failed${http_status:+ ($http_status)}" >&2
    return 1
  fi
  rm -f "$api_error"

  jq -e 'type == "object" and
    (.id | type) == "number" and (.name | type) == "string" and
    (.target | type) == "string" and (.enforcement | type) == "string" and
    (.conditions.ref_name.exclude | type) == "array" and
    (.conditions.ref_name.include | type) == "array" and
    (.rules | type) == "array"
  ' <<<"$json" >/dev/null || {
    echo "release ruleset $id API returned an unexpected JSON shape" >&2
    return 1
  }

  # GitHub deliberately omits bypass_actors from the public/non-admin response.
  # Reading this field requires repository Administration (read). A normal
  # workflow GITHUB_TOKEN cannot be granted that permission, so a redacted
  # response is an authorization failure rather than a policy mismatch.
  jq -e 'has("bypass_actors") and (.bypass_actors | type) == "array"' \
    <<<"$json" >/dev/null || {
    echo "release ruleset $id API response omits privileged bypass_actors; credential requires repository Administration read" >&2
    return 1
  }

  jq -e --argjson id "$id" --arg name "$name" --arg target "$target" --arg ref_pattern "$ref_pattern" '
    .id == $id and .name == $name and .target == $target and .enforcement == "active" and
    .conditions.ref_name == {exclude:[],include:[$ref_pattern]} and
    .bypass_actors == [{actor_id:5,actor_type:"RepositoryRole",bypass_mode:"always"}] and
    ([.rules[].type] | sort) == (["creation","deletion","non_fast_forward","update"] | sort)
  ' <<<"$json" >/dev/null || {
    echo "release ruleset $id does not match the required protected policy" >&2
    return 1
  }
}

check_ruleset 22234415 "Taskfleet release tags" tag 'refs/tags/**'
check_ruleset 22234417 "Taskfleet release authorization refs" branch \
  "refs/heads/taskfleet-release-authorizations/**"
printf 'Taskfleet live release rulesets verified\n'
