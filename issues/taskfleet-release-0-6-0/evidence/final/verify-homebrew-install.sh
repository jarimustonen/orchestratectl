#!/usr/bin/env bash
# Fresh v0.6.1 canonical install/uninstall in a fully disposable, non-/tmp prefix.
set -euo pipefail
expected_version=0.6.1
expected_commit=7e93bd6195fbaf6de0b43d9161228ae2373ab5d1
expected_tap_head=c9e68594340b2b775d23159a3545d53f15306471
root="$(mktemp -d "$HOME/Library/Caches/taskfleet-r10-homebrew.XXXXXX")"
cleanup() { rm -rf "$root"; }
trap cleanup EXIT
prefix="$root/prefix"
home="$root/home"
cache="$root/cache"
mkdir -p "$prefix/bin" "$home" "$cache"
gtimeout 300 git clone --quiet --shared "$(brew --repository)" "$prefix/Homebrew"
gtimeout 300 git clone --quiet https://github.com/jarimustonen/homebrew-taskfleet.git "$root/canonical-tap"
[[ "$(git -C "$root/canonical-tap" rev-parse HEAD)" == "$expected_tap_head" ]]
ln -s ../Homebrew/bin/brew "$prefix/bin/brew"
brew_bin="$prefix/bin/brew"
brew_env=(env -i HOME="$home" PATH="$prefix/bin:/usr/bin:/bin" HOMEBREW_CACHE="$cache" HOMEBREW_NO_AUTO_UPDATE=1 HOMEBREW_NO_ANALYTICS=1 HOMEBREW_NO_INSTALL_CLEANUP=1 HOMEBREW_NO_ENV_HINTS=1)
run() { gtimeout 300 "${brew_env[@]}" "$brew_bin" "$@"; }
[[ "$(run --prefix)" == "$prefix" ]]
run tap jarimustonen/taskfleet "file://$root/canonical-tap" >/dev/null
run trust jarimustonen/taskfleet >/dev/null
installed_tap_head="$(git -C "$prefix/Homebrew/Library/Taps/jarimustonen/homebrew-taskfleet" rev-parse HEAD)"
[[ "$installed_tap_head" == "$expected_tap_head" ]] || { echo "installed tap head mismatch" >&2; exit 1; }
run install jarimustonen/taskfleet/taskfleet >/dev/null
formulae="$(run list --formula)"
[[ "$formulae" == taskfleet ]]
[[ -x "$prefix/bin/taskfleet" && ! -e "$prefix/bin/orchestratectl" && ! -L "$prefix/bin/orchestratectl" ]]
version_json="$(env -i HOME="$home" PATH=/usr/bin:/bin TASKFLEET_HOME="$root/state" "$prefix/bin/taskfleet" version --output json)"
jq -e --arg version "$expected_version" --arg commit "$expected_commit" \
  '.data.version == $version and .data.commit == $commit' <<<"$version_json" >/dev/null
info_json="$(run info --json=v2 jarimustonen/taskfleet/taskfleet)"
jq -e --arg version "$expected_version" '
  .formulae | length == 1 and .[0].name == "taskfleet" and
  .[0].full_name == "jarimustonen/taskfleet/taskfleet" and
  .[0].versions.stable == $version and (.[0].installed | length == 1)
' <<<"$info_json" >/dev/null
run uninstall taskfleet >/dev/null
[[ ! -e "$prefix/bin/taskfleet" && ! -L "$prefix/bin/taskfleet" ]]
[[ ! -e "$prefix/bin/orchestratectl" && ! -L "$prefix/bin/orchestratectl" ]]
[[ -z "$(find "$prefix/Cellar" -mindepth 1 -print -quit 2>/dev/null || true)" ]]
[[ -z "$(run list --formula)" ]]
cat <<EOF
result=pass
prefix_class=disposable-non-temp
formula=jarimustonen/taskfleet/taskfleet
version=$expected_version
embedded_commit=$expected_commit
tap_head=$expected_tap_head
installed_formulae=taskfleet
orchestratectl_alias=absent
uninstall_residue=absent
EOF
