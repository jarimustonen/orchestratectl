#!/usr/bin/env bash
# R8 pre-live simulation: every Homebrew mutation is below a disposable prefix.
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
archive="$repo_root/target/distrib/taskfleet-aarch64-apple-darwin.tar.xz"
formula="$repo_root/target/distrib/taskfleet.rb"
(cd "$repo_root/target/distrib" && shasum -a 256 -c "$repo_root/issues/taskfleet-integrated-validation/evidence/distribution-artifact-hashes.txt" >/dev/null)
[[ -s "$archive" && -s "$formula" ]] || { echo "cargo-dist artifacts required" >&2; exit 2; }
expected_homebrew_version="Homebrew 6.0.21-70-g2316567"
expected_homebrew_head="2316567ba9be476c217c49829a70b7ffe4b806d4"
[[ "$(brew --version | head -1)" == "$expected_homebrew_version" ]] || { echo "$expected_homebrew_version required" >&2; exit 2; }
[[ "$(git -C "$(brew --repository)" rev-parse HEAD)" == "$expected_homebrew_head" ]] || { echo "Homebrew commit mismatch" >&2; exit 2; }
tmp="$(mktemp -d "${TMPDIR:-/tmp}/taskfleet-r8-brew.XXXXXX")"
trap 'rm -rf "$tmp"' EXIT

# Local candidate tap: exact cargo-dist formula/archive, labelled 0.6.0 solely
# so Homebrew exercises upgrade from the real public 0.5.1 receipt.
git init -q "$tmp/new-tap"
mkdir -p "$tmp/new-tap/Formula"
python3 - "$formula" "$tmp/new-tap/Formula/taskfleet.rb" "$archive" <<'PY'
import hashlib, pathlib, sys
src, dst, archive = map(pathlib.Path, sys.argv[1:])
s = src.read_text()
version = 'version "0.5.1"'
old = 'https://github.com/jarimustonen/orchestratectl/releases/download/v0.5.1/taskfleet-aarch64-apple-darwin.tar.xz'
assert s.count(version) == 1, "expected exactly one generated version"
assert s.count(old) == 1, "expected exactly one generated archive URL"
digest = hashlib.sha256(archive.read_bytes()).hexdigest()
assert s.count(f'sha256 "{digest}"') == 1, "formula/archive digest mismatch"
s = s.replace(version, 'version "0.6.0"').replace(old, archive.resolve().as_uri())
assert archive.resolve().as_uri() in s and 'version "0.6.0"' in s
dst.write_text(s)
PY
git -C "$tmp/new-tap" add .
git -C "$tmp/new-tap" -c user.name=fixture -c user.email=fixture@example.invalid commit -qm 'fixture canonical formula'

git clone -q https://github.com/jarimustonen/homebrew-orchestratectl.git "$tmp/old-tap"
required_head="$(jq -r .required_head "$repo_root/issues/taskfleet-distribution-topology/old-tap-migration/manifest.json")"
[[ "$(git -C "$tmp/old-tap" rev-parse HEAD)" == "$required_head" ]]

homebrew_head="$expected_homebrew_head"
git clone -q --shared "$(brew --repository)" "$tmp/prefix/Homebrew"
mkdir -p "$tmp/prefix/bin" "$tmp/home" "$tmp/cache"
ln -s ../Homebrew/bin/brew "$tmp/prefix/bin/brew"
brew_bin="$tmp/prefix/bin/brew"
brew_env=(env -i HOME="$tmp/home" PATH="$tmp/prefix/bin:/usr/bin:/bin" HOMEBREW_CACHE="$tmp/cache" HOMEBREW_NO_AUTO_UPDATE=1 HOMEBREW_NO_ANALYTICS=1 HOMEBREW_NO_INSTALL_CLEANUP=1)
[[ "$("${brew_env[@]}" "$brew_bin" --prefix)" == "$tmp/prefix" ]]
[[ "$(git -C "$tmp/prefix/Homebrew" rev-parse HEAD)" == "$homebrew_head" ]]

# Real old formula and receipt first.
"${brew_env[@]}" "$brew_bin" tap jarimustonen/orchestratectl "file://$tmp/old-tap" >/dev/null
"${brew_env[@]}" "$brew_bin" install jarimustonen/orchestratectl/orchestratectl >/dev/null
[[ -x "$tmp/prefix/bin/orchestratectl" && ! -e "$tmp/prefix/bin/taskfleet" && ! -L "$tmp/prefix/bin/taskfleet" ]]
env -i HOME="$tmp/home" PATH=/usr/bin:/bin ORCHESTRATECTL_HOME="$tmp/old-state" \
  "$tmp/prefix/bin/orchestratectl" version --output json | jq -e '.data.version == "0.5.1"' >/dev/null

# Activate the reviewed migration only in the local tap, then expose the exact
# local canonical candidate. No public remote can be pushed by these operations.
git -C "$tmp/old-tap" config user.name fixture
git -C "$tmp/old-tap" config user.email fixture@example.invalid
git -C "$tmp/old-tap" am -q "$repo_root/issues/taskfleet-distribution-topology/old-tap-migration/0001-migrate-orchestratectl-formula-to-taskfleet-tap.patch"
"${brew_env[@]}" "$brew_bin" tap jarimustonen/taskfleet "file://$tmp/new-tap" >/dev/null
"${brew_env[@]}" "$brew_bin" trust jarimustonen/taskfleet >/dev/null
# Pull the newly committed old-tap migration into Homebrew's installed tap clone.
# This is the real receipt transition boundary; testing the source clone without
# `brew update` leaves Homebrew on the pre-migration formula by construction.
"${brew_env[@]}" "$brew_bin" update >/dev/null
[[ "$(git -C "$tmp/prefix/Homebrew" rev-parse HEAD)" == "$homebrew_head" ]]
# `brew update` applies tap migration metadata and performs the receipt migration.
# Calling `brew migrate` again would attempt to migrate the same receipt twice.
migrated_receipt="$(find "$tmp/prefix/Cellar/taskfleet" -name INSTALL_RECEIPT.json -print -quit)"
[[ -n "$migrated_receipt" ]]
jq -e '.source.tap == "jarimustonen/taskfleet"' "$migrated_receipt" >/dev/null
"${brew_env[@]}" "$brew_bin" upgrade jarimustonen/orchestratectl/orchestratectl
[[ -x "$tmp/prefix/bin/taskfleet" && ! -e "$tmp/prefix/bin/orchestratectl" && ! -L "$tmp/prefix/bin/orchestratectl" ]] || {
  printf 'post-upgrade bin links:\n' >&2; find "$tmp/prefix/bin" -maxdepth 1 -type l -exec sh -c 'printf "%s -> %s\\n" "$1" "$(readlink "$1")"' _ {} \; >&2; exit 1;
}
migrated_version="$(env -i HOME="$tmp/home" PATH=/usr/bin:/bin TASKFLEET_HOME="$tmp/canonical-state" \
  "$tmp/prefix/bin/taskfleet" version --output json)"
printf '%s\n' "$migrated_version" | jq -e '.data.commit == "c3ef8b740ac531f12ce81c759ed209d178cf36bd"' >/dev/null || {
  printf 'migrated receipt did not upgrade to the candidate archive: %s\n' "$migrated_version" >&2; exit 1;
}
"${brew_env[@]}" "$brew_bin" info --json=v2 jarimustonen/orchestratectl/orchestratectl >"$tmp/old-name-info.json"
jq -e '(.formulae // .) as $f | ($f|length)==1 and $f[0].name=="taskfleet" and $f[0].full_name=="jarimustonen/taskfleet/taskfleet"' "$tmp/old-name-info.json" >/dev/null
"${brew_env[@]}" "$brew_bin" uninstall taskfleet >/dev/null
[[ ! -d "$tmp/prefix/Cellar/taskfleet" ]]

# Fresh canonical path from the same isolated tap/prefix.
"${brew_env[@]}" "$brew_bin" install jarimustonen/taskfleet/taskfleet >/dev/null
[[ -x "$tmp/prefix/bin/taskfleet" && ! -e "$tmp/prefix/bin/orchestratectl" && ! -L "$tmp/prefix/bin/orchestratectl" ]]
env -i HOME="$tmp/home" PATH=/usr/bin:/bin TASKFLEET_HOME="$tmp/canonical-state" \
  "$tmp/prefix/bin/taskfleet" version --output json | jq -e '.data.commit == "c3ef8b740ac531f12ce81c759ed209d178cf36bd"' >/dev/null
"${brew_env[@]}" "$brew_bin" uninstall taskfleet >/dev/null
[[ -z "$(find "$tmp/prefix/Cellar" -mindepth 1 -print -quit 2>/dev/null || true)" ]]
printf 'pre-live disposable Homebrew old-receipt migration and fresh install passed\n'
