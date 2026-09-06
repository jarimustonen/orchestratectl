#!/usr/bin/env bash
# Validate ADR 0002 R11 against actual public taps in disposable non-/tmp prefixes.
set -euo pipefail
[[ $# -eq 1 ]] || { echo "usage: $0 <output-json>" >&2; exit 2; }
out_file="$1"
expected_version=0.6.1
expected_commit=7e93bd6195fbaf6de0b43d9161228ae2373ab5d1
old_parent=85ce830378f38cf17283efddd966d5754354e403
old_head=20a70f463e699af5ddba6f6455c20a183c496ca5
new_head=c9e68594340b2b775d23159a3545d53f15306471
cache_parent="$HOME/Library/Caches"
source_homebrew="$(brew --repository)"
github_token="$(gh auth token)"
roots=()
cleanup() {
  local root
  for root in "${roots[@]:-}"; do rm -rf "$root"; done
}
trap cleanup EXIT

fail_log() {
  local root="$1" message="$2"
  echo "$message" >&2
  for f in "$root"/command.{stdout,stderr}; do
    [[ -f "$f" ]] && sed "s#$root#<DISPOSABLE_ROOT>#g" "$f" >&2
  done
  exit 1
}

init_root() {
  root="$(mktemp -d "$cache_parent/taskfleet-r11-homebrew.XXXXXX")"
  roots+=("$root")
  prefix="$root/prefix"; home="$root/home"; cache="$root/cache"
  mkdir -p "$prefix/bin" "$home" "$cache"
  gtimeout 300 git clone --quiet --shared "$source_homebrew" "$prefix/Homebrew"
  ln -s ../Homebrew/bin/brew "$prefix/bin/brew"
  brew_bin="$prefix/bin/brew"
  brew_env=(env -i HOME="$home" PATH="$prefix/bin:/usr/bin:/bin" HOMEBREW_CACHE="$cache"
    HOMEBREW_GITHUB_API_TOKEN="$github_token" HOMEBREW_NO_ANALYTICS=1
    HOMEBREW_NO_INSTALL_CLEANUP=1 HOMEBREW_NO_ENV_HINTS=1 CI=1)
  brew_no_update_env=("${brew_env[@]}" HOMEBREW_NO_AUTO_UPDATE=1)
  run_brew --prefix
  [[ "$(cat "$root/command.stdout")" == "$prefix" ]] || fail_log "$root" "disposable prefix mismatch"
  homebrew_initial_head="$(git -C "$prefix/Homebrew" rev-parse HEAD)"
}

run_brew() {
  : >"$root/command.stdout"; : >"$root/command.stderr"
  if ! gtimeout 300 "${brew_no_update_env[@]}" "$brew_bin" "$@" >"$root/command.stdout" 2>"$root/command.stderr"; then
    fail_log "$root" "brew command failed: brew $*"
  fi
}

run_brew_update() {
  : >"$root/command.stdout"; : >"$root/command.stderr"
  if ! gtimeout 300 "${brew_env[@]}" "$brew_bin" "$@" >"$root/command.stdout" 2>"$root/command.stderr"; then
    fail_log "$root" "brew update command failed: brew $*"
  fi
}

try_brew() {
  : >"$root/command.stdout"; : >"$root/command.stderr"
  set +e
  gtimeout 300 "${brew_no_update_env[@]}" "$brew_bin" "$@" >"$root/command.stdout" 2>"$root/command.stderr"
  try_status=$?
  set -e
}

clone_tap() {
  local identity="$1" repo="$2" expected="$3"
  tap_path="$prefix/Homebrew/Library/Taps/${identity%/*}/homebrew-${identity#*/}"
  mkdir -p "$(dirname "$tap_path")"
  gtimeout 300 git -c credential.helper= clone --quiet "https://github.com/$repo.git" "$tap_path"
  [[ "$(git -C "$tap_path" rev-parse HEAD)" == "$expected" ]] || fail_log "$root" "public tap head mismatch: $repo"
  [[ "$(git -C "$tap_path" remote get-url origin)" == "https://github.com/$repo.git" ]] || fail_log "$root" "public tap origin mismatch: $repo"
}

trust_tap() { run_brew trust "$1"; }

sanitize_json() { sed "s#$root#<DISPOSABLE_ROOT>#g" "$1" | jq -S .; }

capture_receipt() {
  local rack="$1" destination="$2"
  local receipt="$prefix/Cellar/$rack/$expected_version/INSTALL_RECEIPT.json"
  [[ -f "$receipt" ]] || fail_log "$root" "missing receipt for $rack $expected_version"
  sanitize_json "$receipt" >"$destination"
}

capture_version() {
  local destination="$1"
  env -i HOME="$home" PATH=/usr/bin:/bin TASKFLEET_HOME="$root/state" \
    "$prefix/bin/taskfleet" version --output json >"$root/version.json"
  jq -e --arg version "$expected_version" --arg commit "$expected_commit" \
    '.data.version == $version and .data.commit == $commit' "$root/version.json" >/dev/null
  jq -S . "$root/version.json" >"$destination"
}

assert_canonical_ownership() {
  local names formulae
  names="$(run_brew list --formula; cat "$root/command.stdout")"
  [[ "$names" == taskfleet || "$names" == $'orchestratectl\ntaskfleet' ]]
  [[ -x "$prefix/bin/taskfleet" && ! -e "$prefix/bin/orchestratectl" && ! -L "$prefix/bin/orchestratectl" ]]
  [[ ! -e "$prefix/Cellar/orchestratectl" && ! -L "$prefix/Cellar/orchestratectl" ]]
  [[ "$(find "$prefix/Cellar" -mindepth 1 -maxdepth 1 -type d -exec basename {} \;)" == taskfleet ]]
  [[ "$(find "$prefix/Cellar/taskfleet" -mindepth 1 -maxdepth 1 -type d -exec basename {} \;)" == "$expected_version" ]]
  formulae="$(find "$prefix/Homebrew/Library/Taps" -path '*/Formula/*.rb' -type f | sed "s#$prefix/Homebrew/Library/Taps/##" | sort)"
  [[ "$formulae" == "jarimustonen/homebrew-taskfleet/Formula/taskfleet.rb" ]]
}

capture_ownership() {
  local destination="$1" names formulae
  assert_canonical_ownership
  names="$(cat "$root/command.stdout")"
  formulae="$(find "$prefix/Homebrew/Library/Taps" -path '*/Formula/*.rb' -type f | sed "s#$prefix/Homebrew/Library/Taps/##" | sort)"
  jq -n -S --arg names "$names" --arg formula "$formulae" \
    '{brew_list_formula_projection:($names|split("\n")), physical_racks:["taskfleet"], physical_versions:["0.6.1"], formula_files:[$formula], installed_binaries:["taskfleet"], orchestratectl_binary_or_alias:"absent"}' >"$destination"
}

assert_uninstalled() {
  [[ ! -e "$prefix/bin/taskfleet" && ! -L "$prefix/bin/taskfleet" ]]
  [[ ! -e "$prefix/bin/orchestratectl" && ! -L "$prefix/bin/orchestratectl" ]]
  [[ -z "$(find "$prefix/Cellar" -mindepth 1 -print -quit 2>/dev/null || true)" ]]
  run_brew list --formula
  [[ ! -s "$root/command.stdout" ]]
}

finish_root() {
  local finished="$root"
  rm -rf "$finished"
  [[ ! -e "$finished" ]]
}

work="$(mktemp -d "$cache_parent/taskfleet-r11-evidence-work.XXXXXX")"
roots+=("$work")

# Path 1: fresh, fully-qualified canonical install and uninstall.
init_root
clone_tap jarimustonen/taskfleet jarimustonen/homebrew-taskfleet "$new_head"
trust_tap jarimustonen/taskfleet
run_brew install jarimustonen/taskfleet/taskfleet
capture_receipt taskfleet "$work/fresh-receipt.json"
capture_version "$work/fresh-version.json"
capture_ownership "$work/fresh-ownership.json"
run_brew info --json=v2 jarimustonen/taskfleet/taskfleet
sanitize_json "$root/command.stdout" >"$work/fresh-info.json"
run_brew uninstall taskfleet
assert_uninstalled
fresh_homebrew_head="$homebrew_initial_head"
finish_root

# Path 2: old tap-qualified identity resolves through the live migration map.
init_root
clone_tap jarimustonen/orchestratectl jarimustonen/homebrew-orchestratectl "$old_head"
clone_tap jarimustonen/taskfleet jarimustonen/homebrew-taskfleet "$new_head"
trust_tap jarimustonen/orchestratectl
trust_tap jarimustonen/taskfleet
run_brew info --json=v2 jarimustonen/orchestratectl/orchestratectl
sanitize_json "$root/command.stdout" >"$work/qualified-resolution.json"
jq -e '.formulae | length == 1 and .[0].name == "taskfleet" and .[0].full_name == "jarimustonen/taskfleet/taskfleet" and .[0].tap == "jarimustonen/taskfleet"' "$work/qualified-resolution.json" >/dev/null
run_brew install jarimustonen/orchestratectl/orchestratectl
capture_receipt taskfleet "$work/qualified-receipt.json"
capture_version "$work/qualified-version.json"
capture_ownership "$work/qualified-ownership.json"
run_brew uninstall taskfleet
assert_uninstalled
finish_root

# Path 3: an old receipt with the destination already trusted is consumed by
# brew update, then brew upgrade advances the migrated keg to v0.6.1.
init_root
clone_tap jarimustonen/orchestratectl jarimustonen/homebrew-orchestratectl "$old_head"
old_tap_path="$tap_path"
git -C "$old_tap_path" checkout --quiet "$old_parent"
trust_tap jarimustonen/orchestratectl
run_brew install jarimustonen/orchestratectl/orchestratectl
[[ "$(run_brew list --formula; cat "$root/command.stdout")" == orchestratectl ]]
sanitize_json "$prefix/Cellar/orchestratectl/0.5.1/INSTALL_RECEIPT.json" >"$work/legacy-auto-receipt.json"
[[ -x "$prefix/bin/orchestratectl" && ! -e "$prefix/bin/taskfleet" ]]
clone_tap jarimustonen/taskfleet jarimustonen/homebrew-taskfleet "$new_head"
trust_tap jarimustonen/taskfleet/taskfleet
run_brew_update update
[[ "$(git -C "$old_tap_path" rev-parse HEAD)" == "$old_head" ]] || fail_log "$root" "brew update did not reach live old-tap head"
auto_post_update_formulae="$(run_brew list --formula; cat "$root/command.stdout")"
run_brew upgrade
run_brew cleanup taskfleet
auto_post_upgrade_formulae="$(run_brew list --formula; cat "$root/command.stdout")"
[[ "$auto_post_update_formulae" == $'orchestratectl\ntaskfleet' && "$auto_post_upgrade_formulae" == $'orchestratectl\ntaskfleet' ]]
capture_receipt taskfleet "$work/auto-migrated-receipt.json"
capture_version "$work/auto-migrated-version.json"
capture_ownership "$work/auto-ownership.json"
try_brew migrate orchestratectl
auto_explicit_status="$try_status"
auto_explicit_message="$(cat "$root/command.stdout" "$root/command.stderr" | sed "s#$root#<DISPOSABLE_ROOT>#g" | tr '\n' ' ' | sed 's/[[:space:]]\+/ /g; s/ $//')"
assert_canonical_ownership
run_brew uninstall taskfleet
assert_uninstalled
finish_root

# Paths 4–5: without destination trust, update/upgrade must preserve the old keg
# and explain the trust boundary. After explicit tap/trust, brew migrate performs
# the cross-tap move; upgrade, direct canonical reinstall and final uninstall pass.
init_root
clone_tap jarimustonen/orchestratectl jarimustonen/homebrew-orchestratectl "$old_head"
old_tap_path="$tap_path"
git -C "$old_tap_path" checkout --quiet "$old_parent"
trust_tap jarimustonen/orchestratectl
run_brew install jarimustonen/orchestratectl/orchestratectl
sanitize_json "$prefix/Cellar/orchestratectl/0.5.1/INSTALL_RECEIPT.json" >"$work/legacy-explicit-receipt.json"
run_brew_update update
[[ "$(git -C "$old_tap_path" rev-parse HEAD)" == "$old_head" ]] || fail_log "$root" "explicit path did not update old tap"
explicit_update_message="$(cat "$root/command.stdout" "$root/command.stderr" | sed "s#$root#<DISPOSABLE_ROOT>#g" | tr '\n' ' ' | sed 's/[[:space:]]\+/ /g; s/ $//')"
run_brew upgrade
[[ "$(run_brew list --formula; cat "$root/command.stdout")" == orchestratectl ]]
clone_tap jarimustonen/taskfleet jarimustonen/homebrew-taskfleet "$new_head"
trust_tap jarimustonen/taskfleet/taskfleet
run_brew migrate orchestratectl
explicit_migrate_message="$(cat "$root/command.stdout" "$root/command.stderr" | sed "s#$root#<DISPOSABLE_ROOT>#g" | tr '\n' ' ' | sed 's/[[:space:]]\+/ /g; s/ $//')"
[[ "$(run_brew list --formula; cat "$root/command.stdout")" == taskfleet ]]
run_brew upgrade
run_brew cleanup taskfleet
capture_receipt taskfleet "$work/explicit-migrated-receipt.json"
capture_version "$work/explicit-migrated-version.json"
capture_ownership "$work/explicit-ownership.json"
run_brew uninstall taskfleet
assert_uninstalled
run_brew install jarimustonen/taskfleet/taskfleet
capture_receipt taskfleet "$work/direct-after-migration-receipt.json"
capture_version "$work/direct-after-migration-version.json"
capture_ownership "$work/direct-ownership.json"
run_brew uninstall taskfleet
assert_uninstalled
migration_homebrew_final_head="$(git -C "$prefix/Homebrew" rev-parse HEAD)"
finish_root

jq -n -S \
  --arg generated_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --arg brew_version "$(brew --version | head -1)" \
  --arg brew_head "$fresh_homebrew_head" \
  --arg brew_final_head "$migration_homebrew_final_head" \
  --arg auto_post_update "$auto_post_update_formulae" \
  --arg auto_post_upgrade "$auto_post_upgrade_formulae" \
  --arg auto_migrate_status "$auto_explicit_status" \
  --arg auto_migrate_message "$auto_explicit_message" \
  --arg explicit_update_message "$explicit_update_message" \
  --arg explicit_migrate_message "$explicit_migrate_message" \
  --slurpfile fresh_receipt "$work/fresh-receipt.json" \
  --slurpfile fresh_version "$work/fresh-version.json" \
  --slurpfile fresh_info "$work/fresh-info.json" \
  --slurpfile fresh_ownership "$work/fresh-ownership.json" \
  --slurpfile qualified_resolution "$work/qualified-resolution.json" \
  --slurpfile qualified_receipt "$work/qualified-receipt.json" \
  --slurpfile qualified_version "$work/qualified-version.json" \
  --slurpfile qualified_ownership "$work/qualified-ownership.json" \
  --slurpfile legacy_auto_receipt "$work/legacy-auto-receipt.json" \
  --slurpfile auto_migrated_receipt "$work/auto-migrated-receipt.json" \
  --slurpfile auto_migrated_version "$work/auto-migrated-version.json" \
  --slurpfile auto_ownership "$work/auto-ownership.json" \
  --slurpfile legacy_explicit_receipt "$work/legacy-explicit-receipt.json" \
  --slurpfile explicit_migrated_receipt "$work/explicit-migrated-receipt.json" \
  --slurpfile explicit_migrated_version "$work/explicit-migrated-version.json" \
  --slurpfile explicit_ownership "$work/explicit-ownership.json" \
  --slurpfile direct_receipt "$work/direct-after-migration-receipt.json" \
  --slurpfile direct_version "$work/direct-after-migration-version.json" \
  --slurpfile direct_ownership "$work/direct-ownership.json" \
  '{schema_version:1, generated_at:$generated_at, overall:"pass",
    isolation:{prefix_class:"disposable-non-temporary", home:"isolated", cache:"isolated", cellar:"isolated", system_homebrew_objects:"read-only-shared-clone", command_timeout_seconds:300, analytics:"disabled", cleanup:"all-roots-removed", credentials:"runtime-only-not-retained"},
    homebrew:{version:$brew_version, initial_head:$brew_head, post_update_head:$brew_final_head},
    fresh_canonical:{result:"pass", command:"brew install jarimustonen/taskfleet/taskfleet", receipt:$fresh_receipt[0], runtime:$fresh_version[0], info:$fresh_info[0], ownership:$fresh_ownership[0], uninstall_residue:"absent"},
    old_tap_qualified:{result:"pass", command:"brew install jarimustonen/orchestratectl/orchestratectl", resolution:$qualified_resolution[0], receipt:$qualified_receipt[0], runtime:$qualified_version[0], ownership:$qualified_ownership[0], uninstall_residue:"absent"},
    old_receipt_automatic_migration:{result:"pass", baseline_formula:"orchestratectl", baseline_version:"0.5.1", baseline_receipt:$legacy_auto_receipt[0], commands:["brew update","brew upgrade"], destination_trusted_before_update:true, post_update_formulae:$auto_post_update, post_upgrade_formulae:$auto_post_upgrade, receipt:$auto_migrated_receipt[0], runtime:$auto_migrated_version[0], orchestratectl_binary_or_alias:"absent", ownership:$auto_ownership[0]},
    explicit_migrate_after_automatic:{command:"brew migrate orchestratectl", exit_status:($auto_migrate_status|tonumber), semantics:"automatic update already consumed migration", sanitized_message:$auto_migrate_message},
    old_receipt_explicit_migration:{result:"pass", baseline_formula:"orchestratectl", baseline_version:"0.5.1", baseline_receipt:$legacy_explicit_receipt[0], update_upgrade_before_trust:"preserved old keg", update_message:$explicit_update_message, command:"brew migrate orchestratectl", migrate_message:$explicit_migrate_message, receipt:$explicit_migrated_receipt[0], runtime:$explicit_migrated_version[0], orchestratectl_binary_or_alias:"absent", ownership:$explicit_ownership[0]},
    direct_after_migration:{result:"pass", command:"brew install jarimustonen/taskfleet/taskfleet", receipt:$direct_receipt[0], runtime:$direct_version[0], ownership:$direct_ownership[0], final_uninstall_residue:"absent"}}
  ' >"$out_file"
rm -rf "$work"
[[ ! -e "$work" ]]
[[ -z "$(find "$cache_parent" -maxdepth 1 -name 'taskfleet-r11-homebrew.*' -print -quit)" ]]
