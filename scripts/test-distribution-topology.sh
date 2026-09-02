#!/usr/bin/env bash
# Credential-free R7 cargo-dist and disposable Homebrew protocol test.
set -euo pipefail

[[ $# -eq 1 ]] || { echo "usage: $0 <path-to-cargo-dist-0.28.2>" >&2; exit 2; }
dist_bin="$(cd "$(dirname "$1")" && pwd -P)/$(basename "$1")"
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
tmp="$(mktemp -d "${TMPDIR:-/tmp}/taskfleet-r7-dist.XXXXXX")"
tmp="$(cd "$tmp" && pwd -P)"
cleanup() { rm -rf "$tmp"; }
trap cleanup EXIT

[[ "$($dist_bin --version)" == "cargo-dist 0.28.2" ]] || { echo "cargo-dist 0.28.2 required" >&2; exit 2; }
[[ "$(brew --version | head -1)" == Homebrew\ 6.0.21* ]] || {
  echo "Homebrew 6.0.21 is required for the sealed migration drill" >&2; exit 2;
}
cd "$repo_root"
"$dist_bin" generate --check >/dev/null || {
  echo "cargo-dist generated workflow is stale" >&2; exit 2;
}
"$dist_bin" plan --output-format=json >"$tmp/plan.json"
./scripts/validate-distribution-topology.sh "$tmp/plan.json"

# The one old-name artifact is inert, deterministic and cannot mutate a clean home.
mkdir -p "$tmp/stub-home"
stub="$tmp/orchestratectl-installer.sh"
TASKFLEET_STUB_OUT="$stub" ./scripts/build-legacy-installer-stub.sh >/dev/null
set +e
HOME="$tmp/stub-home" "$stub" >"$tmp/stub.stdout" 2>"$tmp/stub.stderr"
stub_status=$?
set -e
[[ "$stub_status" == 1 ]]
[[ "$(shasum -a 256 "$stub" | awk '{print $1}')" == \
  "$(jq -r .cargo_dist.stub_sha256 release/taskfleet-distribution.json)" ]]
[[ ! -s "$tmp/stub.stdout" ]]
grep -F 'No files or state were changed.' "$tmp/stub.stderr" >/dev/null
grep -F 'https://github.com/jarimustonen/taskfleet/releases/latest/download/taskfleet-installer.sh' "$tmp/stub.stderr" >/dev/null
[[ -z "$(find "$tmp/stub-home" -mindepth 1 -print -quit)" ]]

# Exercise cargo-dist's real collection/generation path on the native macOS
# target. The full three-target graph is asserted from plan.json; this build
# proves the generated formula, shell installer, archive and collected stub.
"$dist_bin" build --artifacts=all --target=aarch64-apple-darwin \
  --output-format=json >"$tmp/build-manifest.json"
actual_stub="$repo_root/target/distrib/orchestratectl-installer.sh"
[[ "$(jq -r '.artifacts["orchestratectl-installer.sh"].path' "$tmp/build-manifest.json")" == "$actual_stub" ]]
cmp "$stub" "$actual_stub"
[[ "$(shasum -a 256 "$actual_stub" | awk '{print $1}')" == \
  "$(jq -r .cargo_dist.stub_sha256 release/taskfleet-distribution.json)" ]]
formula="$repo_root/target/distrib/taskfleet.rb"
shell_installer="$repo_root/target/distrib/taskfleet-installer.sh"
grep -F 'bin.install "taskfleet"' "$formula" >/dev/null
if grep -F 'bin.install "orchestratectl"' "$formula" >/dev/null ||
   awk '/BINARY_ALIASES =/{aliases=1} aliases{print} aliases && /^  }/{exit}' "$formula" |
     grep -F orchestratectl >/dev/null; then
  echo "generated Homebrew formula installs an old-name binary or alias" >&2; exit 2
fi
if grep -F orchestratectl "$formula" | grep -Ev '^[[:space:]]*(homepage|url) ' >/dev/null; then
  echo "generated formula has an old identity outside truthful pre-R9 hosting URLs" >&2; exit 2
fi
grep -F '_bins="taskfleet"' "$shell_installer" >/dev/null
if grep -F '_bins="orchestratectl"' "$shell_installer" >/dev/null; then
  echo "generated shell installer contains the old binary" >&2; exit 2
fi
archive="$repo_root/target/distrib/taskfleet-aarch64-apple-darwin.tar.xz"
[[ "$(tar -tf "$archive" | grep -E '(^|/)(taskfleet|orchestratectl)$' | sed 's#.*/##')" == taskfleet ]]

# Apply the sealed old-tap change only in a disposable clone. Its tree must be
# exactly the reviewed delete+add tree and no public ref is reachable here.
git clone -q "https://github.com/jarimustonen/homebrew-orchestratectl.git" "$tmp/old-source"
required_head="$(jq -r .required_head issues/taskfleet-distribution-topology/old-tap-migration/manifest.json)"
[[ "$(git -C "$tmp/old-source" rev-parse HEAD)" == "$required_head" ]]
git -C "$tmp/old-source" config user.name fixture
git -C "$tmp/old-source" config user.email fixture@example.invalid
git -C "$tmp/old-source" am -q "$repo_root/issues/taskfleet-distribution-topology/old-tap-migration/0001-migrate-orchestratectl-formula-to-taskfleet-tap.patch"
[[ "$(git -C "$tmp/old-source" rev-parse HEAD^{tree})" == "$(jq -r .prepared_tree issues/taskfleet-distribution-topology/old-tap-migration/manifest.json)" ]]
[[ ! -e "$tmp/old-source/Formula/orchestratectl.rb" ]]
jq -e '.orchestratectl == "jarimustonen/taskfleet/taskfleet"' "$tmp/old-source/tap_migrations.json" >/dev/null

# Build a disposable Homebrew installation by sharing only Homebrew's Git
# objects. Taps, cache, prefix and HOME all live below $tmp; no user tap or keg
# can be read or changed.
mkdir -p "$tmp/prefix" "$tmp/home" "$tmp/cache"
git clone -q --shared "$(brew --repository)" "$tmp/prefix/Homebrew"
mkdir -p "$tmp/prefix/bin"
ln -s ../Homebrew/bin/brew "$tmp/prefix/bin/brew"
brew_bin="$tmp/prefix/bin/brew"
brew_env=(env -i HOME="$tmp/home" PATH="$tmp/prefix/bin:/usr/bin:/bin" HOMEBREW_CACHE="$tmp/cache" HOMEBREW_NO_AUTO_UPDATE=1 HOMEBREW_NO_ANALYTICS=1)
[[ "$("${brew_env[@]}" "$brew_bin" --prefix)" == "$tmp/prefix" ]]

# Local canonical tap candidate uses cargo-dist's exact generated formula.
git clone -q "https://github.com/jarimustonen/homebrew-taskfleet.git" "$tmp/new-source"
expected_new_head="$(jq -r .public_receipts.proof_commit release/taskfleet-distribution.json)"
expected_new_tree="$(jq -r .public_receipts.proof_tree release/taskfleet-distribution.json)"
[[ "$(git -C "$tmp/new-source" rev-parse HEAD)" == "$expected_new_head" ]]
[[ "$(git -C "$tmp/new-source" rev-parse HEAD^{tree})" == "$expected_new_tree" ]]
[[ -z "$(git -C "$tmp/new-source" ls-tree -r --name-only HEAD)" ]]
mkdir -p "$tmp/new-source/Formula"
cp "$formula" "$tmp/new-source/Formula/taskfleet.rb"
git -C "$tmp/new-source" add Formula/taskfleet.rb
git -C "$tmp/new-source" -c user.name=fixture -c user.email=fixture@example.invalid commit -qm 'fixture: canonical formula'

# Clone both taps into the disposable Homebrew prefix without contacting or
# mutating their public remotes. Formulary must follow the full cross-tap rename.
"${brew_env[@]}" "$brew_bin" tap jarimustonen/taskfleet "file://$tmp/new-source" >/dev/null
"${brew_env[@]}" "$brew_bin" trust jarimustonen/taskfleet >/dev/null
"${brew_env[@]}" "$brew_bin" tap jarimustonen/orchestratectl "file://$tmp/old-source" >/dev/null
formula_json="$("${brew_env[@]}" "$brew_bin" info --json=v2 jarimustonen/orchestratectl/orchestratectl)"
jq -e '(if type == "array" then . else .formulae end) as $formulae |
  ($formulae | length) == 1 and $formulae[0].name == "taskfleet" and
  $formulae[0].tap == "jarimustonen/taskfleet" and
  ([$formulae[0].aliases[]? | select(. == "orchestratectl")] | length) == 0' <<<"$formula_json" >/dev/null
[[ -z "$(find "$tmp/prefix/Cellar" -mindepth 1 -print -quit 2>/dev/null || true)" ]]
[[ -z "$(find "$tmp/prefix/bin" -type f -o -type l | grep -v '/brew$' || true)" ]]

printf 'Taskfleet cargo-dist and disposable Homebrew topology passed\n'
