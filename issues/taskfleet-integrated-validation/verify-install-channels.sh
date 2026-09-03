#!/usr/bin/env bash
# R8 pre-live Cargo/archive/shell channel checks, entirely disposable.
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
(cd "$repo_root/target/distrib" && shasum -a 256 -c "$repo_root/issues/taskfleet-integrated-validation/evidence/distribution-artifact-hashes.txt" >/dev/null)
tmp="$(mktemp -d "${TMPDIR:-/tmp}/taskfleet-r8-install.XXXXXX")"
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/home" "$tmp/cargo-root" "$tmp/cargo-home" "$tmp/archive" "$tmp/shell-assets" "$tmp/shell-cargo"
cargo_bin="$(rustup which cargo)"
toolchain_bin="$(dirname "$cargo_bin")"
toolchain_root="$(dirname "$toolchain_bin")"
tool_path="$toolchain_bin:/opt/homebrew/bin:/usr/bin:/bin"
assert_absent() { [[ ! -e "$1" && ! -L "$1" ]]; }

# Cargo path install: same package graph, private root and CARGO_HOME. It never
# changes the user's installed binary or registry credentials.
env -i HOME="$tmp/home" PATH="$tool_path" CARGO_HOME="$tmp/cargo-home" DYLD_FALLBACK_LIBRARY_PATH="$toolchain_root/lib" \
  CARGO_TARGET_DIR="$tmp/cargo-target" "$cargo_bin" install --locked --path "$repo_root/crates/taskfleet" --root "$tmp/cargo-root" >/dev/null
[[ -x "$tmp/cargo-root/bin/taskfleet" ]] && assert_absent "$tmp/cargo-root/bin/orchestratectl"
env -i HOME="$tmp/home" PATH=/usr/bin:/bin TASKFLEET_HOME="$tmp/cargo-state" "$tmp/cargo-root/bin/taskfleet" version --output json | jq -e '.data.commit == "c3ef8b740ac531f12ce81c759ed209d178cf36bd"' >/dev/null

# Bounded legacy Cargo wrapper: a separate disposable root receives only the
# old executable, but dispatches the same exact-SHA engine and warns on stderr.
env -i HOME="$tmp/home" PATH="$tool_path" CARGO_HOME="$tmp/cargo-home" DYLD_FALLBACK_LIBRARY_PATH="$toolchain_root/lib" \
  CARGO_TARGET_DIR="$tmp/cargo-target" "$cargo_bin" install --locked --path "$repo_root/compat/orchestratectl" --root "$tmp/legacy-cargo-root" >/dev/null
[[ -x "$tmp/legacy-cargo-root/bin/orchestratectl" ]] && assert_absent "$tmp/legacy-cargo-root/bin/taskfleet"
env -i HOME="$tmp/home" PATH=/usr/bin:/bin ORCHESTRATECTL_HOME="$tmp/legacy-cargo-state" \
  "$tmp/legacy-cargo-root/bin/orchestratectl" version --output json >"$tmp/legacy-version.json" 2>"$tmp/legacy-version.err"
jq -e '.data.commit == "c3ef8b740ac531f12ce81c759ed209d178cf36bd"' "$tmp/legacy-version.json" >/dev/null
[[ "$(grep -cF '`orchestratectl` is deprecated' "$tmp/legacy-version.err" || true)" == 1 ]]

# Raw archive contains and runs only the canonical binary.
archive="$repo_root/target/distrib/taskfleet-aarch64-apple-darwin.tar.xz"
tar -xJf "$archive" -C "$tmp/archive"
python3 - "$tmp/archive" <<'PY'
import os, pathlib, sys
root = pathlib.Path(sys.argv[1])
executables = sorted(p.name for p in root.rglob("*") if p.is_file() and os.access(p, os.X_OK))
assert executables == ["taskfleet"], executables
assert not any(p.name == "orchestratectl" for p in root.rglob("*"))
PY
! tar -tf "$archive" | grep -Eq '(^|/)orchestratectl$'
archive_bin="$(find "$tmp/archive" -type f -name taskfleet -perm -u+x)"
env -i HOME="$tmp/home" PATH=/usr/bin:/bin TASKFLEET_HOME="$tmp/archive-state" "$archive_bin" version --output json | jq -e '.data.commit == "c3ef8b740ac531f12ce81c759ed209d178cf36bd"' >/dev/null

# Generated shell installer, pointed through its documented override at a local
# file:// fixture containing byte-identical cargo-dist artifacts.
cp "$archive" "$tmp/shell-assets/"
cp "$repo_root/target/distrib/taskfleet-aarch64-apple-darwin.tar.xz.sha256" "$tmp/shell-assets/"
cp "$repo_root/target/distrib/taskfleet-installer.sh" "$tmp/installer.sh"
asset_url="$(python3 -c 'import pathlib,sys; print(pathlib.Path(sys.argv[1]).resolve().as_uri())' "$tmp/shell-assets")"
env -i HOME="$tmp/home" CARGO_HOME="$tmp/shell-cargo" PATH=/usr/bin:/bin \
  TASKFLEET_NO_MODIFY_PATH=1 INSTALLER_DOWNLOAD_URL="$asset_url" \
  bash "$tmp/installer.sh" --quiet
[[ -x "$tmp/shell-cargo/bin/taskfleet" ]] && assert_absent "$tmp/shell-cargo/bin/orchestratectl"
env -i HOME="$tmp/home" PATH=/usr/bin:/bin TASKFLEET_HOME="$tmp/shell-state" "$tmp/shell-cargo/bin/taskfleet" version --output json | jq -e '.data.commit == "c3ef8b740ac531f12ce81c759ed209d178cf36bd"' >/dev/null

printf 'disposable Cargo, archive, and pre-live shell installs passed\n'
