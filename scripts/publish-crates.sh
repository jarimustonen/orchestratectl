#!/usr/bin/env bash
# Package, publish, and cryptographically reconcile Taskfleet's crates.io legs.
# Duplicate-version diagnostics are never interpreted: only registry receipts
# matching the local archive and source commit can complete a leg.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
cd "$repo_root"
readonly topology="${TASKFLEET_RELEASE_TOPOLOGY:-$repo_root/release/taskfleet-release.json}"
readonly cargo_bin="${CARGO_BIN:-cargo}"
readonly curl_bin="${CURL_BIN:-curl}"
readonly tar_bin="${TAR_BIN:-tar}"
readonly sha256_bin="${SHA256_BIN:-sha256sum}"
readonly sleep_bin="${SLEEP_BIN:-sleep}"
readonly registry="${CRATES_IO_API:-https://crates.io/api/v1}"
readonly user_agent="taskfleet-release-reconciler/1 (+https://github.com/jarimustonen/orchestratectl)"
readonly receipt_dir="${RELEASE_RECEIPT_DIR:-$repo_root/target/release-receipts}"

usage() {
  echo "usage: scripts/publish-crates.sh package [<package>] | publish <package>" >&2
  exit 2
}

[[ -f "$topology" ]] || { echo "release topology not found: $topology" >&2; exit 2; }
jq -e '
  .schema_version == 1 and
  .owners == ([.owners[]] | unique | sort) and (.owners | length > 0) and
  [.crates_io.legs[] | .package] == ["taskfleet-core","taskfleet","orchestratectl"] and
  [.crates_io.legs[] | .depends_on] == [null,"taskfleet-core","taskfleet"] and
  [.distribution[] | (.package + ":" + .registry + ":" + .workflow)] == [
    "taskfleet:gh-releases:release.yml", "taskfleet:homebrew:release.yml"
  ]
' "$topology" >/dev/null || { echo "release topology is not the admitted five-leg Taskfleet graph" >&2; exit 2; }

version="$(awk -F'"' '
  /^\[workspace\.package\]/ { in_package=1; next }
  /^\[/ { in_package=0 }
  in_package && /^version[[:space:]]*=/ { print $2; exit }
' Cargo.toml)"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([+-][0-9A-Za-z.-]+)?$ ]] || {
  echo "invalid workspace version: ${version:-<missing>}" >&2
  exit 2
}
source_commit="${SOURCE_COMMIT:-$(git rev-parse HEAD)}"
[[ "$source_commit" =~ ^[0-9a-f]{40}$ ]] || { echo "invalid source commit: $source_commit" >&2; exit 2; }

metadata_file="$(mktemp "${TMPDIR:-/tmp}/taskfleet-release-metadata.XXXXXX")"
tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/taskfleet-registry.XXXXXX")"
cleanup() { rm -f "$metadata_file"; rm -rf "$tmp_dir"; }
trap cleanup EXIT
"$cargo_bin" metadata --locked --no-deps --format-version 1 >"$metadata_file"

package_manifest() {
  jq -er --arg package "$1" '.crates_io.legs[] | select(.package == $package) | .manifest' "$topology"
}

dependency_of() {
  jq -r --arg package "$1" '.crates_io.legs[] | select(.package == $package) | .depends_on // empty' "$topology"
}

assert_local_metadata() {
  local package="$1" manifest expected_repo dependency
  manifest="$(package_manifest "$package")"
  expected_repo="$(jq -r .repository "$topology")"
  jq -e --arg package "$package" --arg version "$version" --arg manifest "$repo_root/$manifest" --arg repo "https://github.com/$expected_repo" '
    [.packages[] | select(.name == $package and .version == $version and .manifest_path == $manifest and
      .repository == $repo and .homepage == $repo and .license == "MIT" and .rust_version == "1.85")] | length == 1
  ' "$metadata_file" >/dev/null || { echo "$package local package metadata does not match release topology" >&2; exit 2; }
  dependency="$(dependency_of "$package")"
  if [[ -n "$dependency" ]]; then
    jq -e --arg package "$package" --arg dependency "$dependency" --arg requirement "=$version" '
      [.packages[] | select(.name == $package) | .dependencies[] |
       select(.name == $dependency and .req == $requirement)] | length == 1
    ' "$metadata_file" >/dev/null || { echo "$package does not exactly pin $dependency to =$version" >&2; exit 2; }
  fi
}

archive_path() { printf '%s/target/package/%s-%s.crate\n' "$repo_root" "$1" "$version"; }

prepare_archive() {
  local package="$1" archive root archive_commit dependency
  assert_local_metadata "$package"
  "$cargo_bin" package --locked --no-verify --package "$package"
  archive="$(archive_path "$package")"
  [[ -s "$archive" ]] || { echo "cargo did not create $archive" >&2; exit 2; }
  root="$package-$version"
  archive_commit="$("$tar_bin" -xOf "$archive" "$root/.cargo_vcs_info.json" | jq -er '.git.sha1')"
  [[ "$archive_commit" == "$source_commit" ]] || {
    echo "$package archive source commit $archive_commit does not match $source_commit" >&2
    exit 2
  }
  dependency="$(dependency_of "$package")"
  if [[ -n "$dependency" ]]; then
    "$tar_bin" -xOf "$archive" "$root/Cargo.toml" | grep -Eq "^version = \"=$version\"$" || {
      echo "$package normalized archive does not retain an exact =$version dependency" >&2
      exit 2
    }
  fi
  printf 'packaged %s@%s from %s\n' "$package" "$version" "$source_commit"
}

http_get() {
  local url="$1" output="$2" status
  status="$("$curl_bin" -sS -L -A "$user_agent" -o "$output" -w '%{http_code}' "$url")" || {
    echo "registry request failed: $url" >&2
    return 1
  }
  printf '%s\n' "$status"
}

sha256_file() { "$sha256_bin" "$1" | awk '{print $1}'; }

reconcile() {
  local package="$1" local_archive version_json owners_json deps_json remote_archive
  local status expected_checksum downloaded_checksum archive_commit dependency receipt
  local_archive="$(archive_path "$package")"
  version_json="$tmp_dir/$package-version.json"
  owners_json="$tmp_dir/$package-owners.json"
  deps_json="$tmp_dir/$package-deps.json"
  remote_archive="$tmp_dir/$package-$version.crate"

  status="$(http_get "$registry/crates/$package/$version" "$version_json")" || return 1
  [[ "$status" == 200 ]] || return 3
  [[ "$(http_get "$registry/crates/$package/owners" "$owners_json")" == 200 ]] || return 1
  [[ "$(http_get "$registry/crates/$package/$version/dependencies" "$deps_json")" == 200 ]] || return 1
  [[ "$(http_get "$registry/crates/$package/$version/download" "$remote_archive")" == 200 ]] || return 1

  expected_checksum="$(sha256_file "$local_archive")"
  downloaded_checksum="$(sha256_file "$remote_archive")"
  [[ "$expected_checksum" == "$downloaded_checksum" ]] || {
    echo "$package@$version registry archive differs from the sealed local archive" >&2; return 2;
  }
  jq -e --arg checksum "$expected_checksum" --arg repo "https://github.com/$(jq -r .repository "$topology")" \
    --arg description "$(jq -r --arg package "$package" '.packages[] | select(.name == $package) | .description' "$metadata_file")" '
    .version.checksum == $checksum and .version.license == "MIT" and .version.rust_version == "1.85" and
    .crate.repository == $repo and .crate.homepage == $repo and .crate.description == $description
  ' "$version_json" >/dev/null || { echo "$package@$version registry metadata mismatch" >&2; return 2; }
  jq -e --slurpfile topology "$topology" '
    ([.users[].login] | unique | sort) == $topology[0].owners
  ' "$owners_json" >/dev/null || { echo "$package@$version registry owner set mismatch" >&2; return 2; }

  dependency="$(dependency_of "$package")"
  jq -e --arg package "$package" --arg dependency "$dependency" --arg requirement "=$version" \
    --slurpfile topology "$topology" '
    ([.dependencies[] | select(.crate_id as $id | any($topology[0].crates_io.legs[]; .package == $id)) |
      {crate_id, req}] == (if $dependency == "" then [] else [{crate_id:$dependency,req:$requirement}] end))
  ' "$deps_json" >/dev/null || { echo "$package@$version registry dependency requirements mismatch" >&2; return 2; }

  archive_commit="$("$tar_bin" -xOf "$remote_archive" "$package-$version/.cargo_vcs_info.json" | jq -er '.git.sha1')" || {
    echo "$package@$version registry archive has no readable source receipt" >&2; return 2;
  }
  [[ "$archive_commit" == "$source_commit" ]] || {
    echo "$package@$version registry source commit $archive_commit does not match $source_commit" >&2; return 2;
  }

  mkdir -p "$receipt_dir"
  receipt="$receipt_dir/$package-$version.json"
  jq -n --arg package "$package" --arg version "$version" --arg checksum "$expected_checksum" \
    --arg source_commit "$source_commit" --arg dependency "$dependency" --arg requirement "=$version" \
    --slurpfile owners "$owners_json" '
    {schema_version:1,registry:"crates.io",package:$package,version:$version,checksum:$checksum,
     owners:([$owners[0].users[].login] | unique | sort),source_commit:$source_commit,
     dependencies:(if $dependency == "" then [] else [{package:$dependency,requirement:$requirement}] end),
     metadata_verified:true,archive_verified:true}
  ' >"$receipt"
  printf 'reconciled %s@%s (%s)\n' "$package" "$version" "$expected_checksum"
}

publish_leg() {
  local package="$1" rc=0
  prepare_archive "$package"
  if reconcile "$package"; then
    echo "$package@$version already exists and its registry receipt matches exactly; publish skipped"
    return 0
  else
    rc=$?
  fi
  [[ "$rc" -eq 3 ]] || { echo "$package@$version exists but did not reconcile; refusing publish" >&2; return "$rc"; }

  # Cargo's text is deliberately ignored. A success, duplicate, timeout, or
  # transport failure all proceed to the same authoritative registry check.
  if "$cargo_bin" publish --locked --package "$package"; then rc=0; else rc=$?; fi
  for attempt in 1 2 3 4 5 6 7 8 9 10; do
    if reconcile "$package"; then return 0; else reconcile_rc=$?; fi
    [[ "$reconcile_rc" -eq 3 ]] || return "$reconcile_rc"
    [[ "$attempt" -lt 10 ]] && "$sleep_bin" 30
  done
  echo "$package@$version is not index-visible after publish (cargo status $rc); resume this exact leg after registry propagation" >&2
  return 1
}

command="${1:-}"
package="${2:-}"
case "$command" in
  package)
    if [[ -n "$package" ]]; then
      jq -e --arg package "$package" 'any(.crates_io.legs[]; .package == $package)' "$topology" >/dev/null || usage
      prepare_archive "$package"
    else
      while IFS= read -r package; do prepare_archive "$package"; done < <(jq -r '.crates_io.legs[].package' "$topology")
    fi
    ;;
  publish)
    [[ -n "$package" ]] || usage
    jq -e --arg package "$package" 'any(.crates_io.legs[]; .package == $package)' "$topology" >/dev/null || usage
    publish_leg "$package"
    ;;
  *) usage ;;
esac
