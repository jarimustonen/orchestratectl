#!/usr/bin/env bash
# Deterministic project hook for ossctl's engine-owned version bump.
# Runs inside ossctl's clean release checkout after manifest/pin/lock/changelog edits.
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"

snapshot_dir="crates/octl-cli/tests/snapshots"
snapshots=(
  "$snapshot_dir/envelope_snapshots__version_json.snap"
  "$snapshot_dir/envelope_snapshots__version_jsonl.snap"
  "$snapshot_dir/envelope_snapshots__version_text.snap"
)

state_dir="$(mktemp -d "${TMPDIR:-/tmp}/ossctl-bump-hook.XXXXXX")"
cleanup() { rm -rf "$state_dir"; }
trap cleanup EXIT

# Preserve the complete non-version-snapshot state across the test. This catches
# tracked, staged, and untracked side effects while allowing ossctl's pre-hook
# manifest/lock/changelog edits to remain exactly as they were.
exclude_args=()
for snapshot in "${snapshots[@]}"; do
  exclude_args+=(":(exclude)$snapshot")
done
git diff --binary HEAD -- . "${exclude_args[@]}" >"$state_dir/before.diff"
git ls-files --others --exclude-standard -z >"$state_dir/before.untracked"

INSTA_UPDATE=always cargo test --locked -p orchestratectl --test envelope_snapshots

if find . \( -path ./target -o -path ./.git \) -prune -o -name '*.snap.new' -print -quit | grep -q .; then
  echo "bump hook left unreviewed .snap.new files" >&2
  find . \( -path ./target -o -path ./.git \) -prune -o -name '*.snap.new' -print >&2
  exit 1
fi

git diff --binary HEAD -- . "${exclude_args[@]}" >"$state_dir/after.diff"
git ls-files --others --exclude-standard -z >"$state_dir/after.untracked"
cmp -s "$state_dir/before.diff" "$state_dir/after.diff" || {
  echo "bump hook changed a file outside the three version snapshots" >&2
  exit 1
}
cmp -s "$state_dir/before.untracked" "$state_dir/after.untracked" || {
  echo "bump hook created or removed an unrelated untracked file" >&2
  exit 1
}

./scripts/check-version-snapshots.sh

for snapshot in "${snapshots[@]}"; do
  test -f "$snapshot" || { echo "missing version snapshot: $snapshot" >&2; exit 1; }
  git diff --quiet HEAD -- "$snapshot" && {
    echo "version bump did not regenerate expected snapshot: $snapshot" >&2
    exit 1
  }
done
