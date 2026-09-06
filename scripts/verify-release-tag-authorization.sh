#!/usr/bin/env bash
# Fail-closed authorization for a pushed release tag. The held-tag wrapper is
# the sole producer of the durable, version-scoped authorization ref.
set -euo pipefail

for command_name in gh jq git awk cargo; do
  command -v "$command_name" >/dev/null 2>&1 || {
    echo "release authorization requires $command_name" >&2
    exit 1
  }
done

: "${GITHUB_REPOSITORY:?GITHUB_REPOSITORY is required}"
: "${GITHUB_REF:?GITHUB_REF is required}"
: "${GITHUB_REF_TYPE:?GITHUB_REF_TYPE is required}"
: "${GITHUB_REF_NAME:?GITHUB_REF_NAME is required}"

[[ "$GITHUB_REPOSITORY" == "jarimustonen/taskfleet" ]] || exit 1
[[ "$(gh api "repos/$GITHUB_REPOSITORY" --jq .node_id)" == "R_kgDOS3Iezw" ]] || exit 1
[[ "$GITHUB_REF_TYPE" == tag ]] || exit 1
[[ "$GITHUB_REF" == "refs/tags/$GITHUB_REF_NAME" ]] || exit 1
[[ "$GITHUB_REF_NAME" =~ ^v[0-9]+\.[0-9]+\.[0-9]+([+-][0-9A-Za-z.-]+)?$ ]] || exit 1
version="$(awk -F'"' '/^\[workspace\.package\]/{p=1;next} /^\[/{p=0} p&&/^version[[:space:]]*=/{print $2;exit}' Cargo.toml)" || exit 1
[[ -n "$version" && "$GITHUB_REF_NAME" == "v$version" ]] || exit 1
./scripts/verify-release-activation.sh >/dev/null || exit 1
./scripts/verify-release-github-policy.sh >/dev/null || exit 1

# checkout resolves annotated and lightweight tags to the commit. Do not compare
# with github.sha, which may identify an annotated tag object for a tag push.
release_commit="$(git rev-parse 'HEAD^{commit}')" || exit 1
[[ "$release_commit" =~ ^[0-9a-f]{40}$ ]] || exit 1
authorization_name="taskfleet-release-authorizations/$GITHUB_REF_NAME"
authorization_json="$(gh api "repos/$GITHUB_REPOSITORY/git/ref/heads/$authorization_name")" || exit 1
jq -e --arg ref "refs/heads/$authorization_name" --arg sha "$release_commit" '
  (keys | index("ref")) != null and (keys | index("object")) != null and
  .ref == $ref and .object.type == "commit" and .object.sha == $sha
' <<<"$authorization_json" >/dev/null || exit 1

# The wrapper creates the authorization ref only after its exact-SHA main CI
# wait succeeds and immediately before Shipshape resumes the held tag. Live main
# may advance after tag push; the durable ref, not a racy later main lookup, is
# the release-time proof.
printf 'authorized %s at %s by refs/heads/%s\n' \
  "$GITHUB_REF_NAME" "$release_commit" "$authorization_name"
