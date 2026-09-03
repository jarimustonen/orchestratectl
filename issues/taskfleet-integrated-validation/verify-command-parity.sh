#!/usr/bin/env bash
# R8: compare canonical and bounded Cargo-wrapper machine behavior in sandboxes.
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
canonical="${TASKFLEET_BIN:-$repo_root/target/release/taskfleet}"
compat="${ORCHESTRATECTL_BIN:-$repo_root/target/release/orchestratectl}"
[[ -x "$canonical" && -x "$compat" ]] || { echo "release binaries are required" >&2; exit 2; }
tmp="$(mktemp -d "${TMPDIR:-/tmp}/taskfleet-r8-parity.XXXXXX")"
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/home"

# Exhaustively compare the complete public Clap command tree. Invocation branding
# is the only normalized field; every flag, positional, alias, requirement,
# output contract, and visible command must otherwise be byte-equivalent.
env -i HOME="$tmp/home" PATH=/usr/bin:/bin TASKFLEET_HOME="$tmp/help-tree" \
  "$canonical" --help --output json --depth full >"$tmp/canonical-help.json" 2>"$tmp/canonical-help.err"
env -i HOME="$tmp/home" PATH=/usr/bin:/bin TASKFLEET_HOME="$tmp/help-tree" \
  "$compat" --help --output json --depth full >"$tmp/compat-help.json" 2>"$tmp/compat-help.err"
[[ ! -s "$tmp/canonical-help.err" ]]
[[ "$(grep -cF '`orchestratectl` is deprecated' "$tmp/compat-help.err" || true)" == 1 ]]
jq -S . "$tmp/canonical-help.json" >"$tmp/canonical-help.normalized.json"
jq -S 'walk(if type=="object" and has("command") then .command |= sub("^orchestratectl";"taskfleet") else . end)' \
  "$tmp/compat-help.json" >"$tmp/compat-help.normalized.json"
cmp "$tmp/canonical-help.normalized.json" "$tmp/compat-help.normalized.json"
jq '{schema_version:1, source:"structured --help --depth full", commands:[.. | objects | select(has("command") and has("subcommands")) | select((.hidden // false)==false) | .command], count:([.. | objects | select(has("command") and has("subcommands")) | select((.hidden // false)==false) | .command]|length)}' \
  "$tmp/canonical-help.json" >"${R8_PARITY_INVENTORY:-$tmp/public-command-inventory.json}"

# Exercise every visible command path through both entrypoints with a forced
# invalid flag. This is side-effect free but proves dispatch, parser selection,
# machine-error stdout, filtered stderr, and exit parity for the complete tree.
while IFS= read -r command_path; do
  suffix="${command_path#taskfleet}"
  read -r -a path_args <<<"$suffix"
  set +e
  env -i HOME="$tmp/home" PATH=/usr/bin:/bin TASKFLEET_HOME="$tmp/all-paths" \
    "$canonical" --output json "${path_args[@]}" --r8-invalid >"$tmp/all-canon.out" 2>"$tmp/all-canon.err"
  canonical_status=$?
  env -i HOME="$tmp/home" PATH=/usr/bin:/bin TASKFLEET_HOME="$tmp/all-paths" \
    "$compat" --output json "${path_args[@]}" --r8-invalid >"$tmp/all-compat.out" 2>"$tmp/all-compat.err"
  compat_status=$?
  set -e
  [[ "$canonical_status" == "$compat_status" && "$canonical_status" != 0 ]]
  cmp "$tmp/all-canon.out" "$tmp/all-compat.out"
  [[ "$(grep -cF '`orchestratectl` is deprecated' "$tmp/all-compat.err" || true)" == 1 ]]
  grep -vF '`orchestratectl` is deprecated' "$tmp/all-compat.err" \
    | sed 's/Usage: orchestratectl/Usage: taskfleet/g' >"$tmp/all-compat.filtered.err" || true
  cmp "$tmp/all-canon.err" "$tmp/all-compat.filtered.err"
done < <(jq -r '.commands[]' "${R8_PARITY_INVENTORY:-$tmp/public-command-inventory.json}")

# Commands are state-independent or run against byte-identical empty roots. Help
# branding is intentionally different and is tested separately below.
commands=(
  'version'
  '--output json version'
  '--output jsonl version'
  '--output json run list'
  '--output jsonl run list'
  '--output json skill list'
  '--output json config show'
)
for i in "${!commands[@]}"; do
  mkdir -p "$tmp/state-$i"
  read -r -a args <<<"${commands[$i]}"
  set +e
  env -i HOME="$tmp/home" PATH=/usr/bin:/bin TASKFLEET_HOME="$tmp/state-$i" \
    "$canonical" "${args[@]}" >"$tmp/canon.out" 2>"$tmp/canon.err"
  canonical_status=$?
  env -i HOME="$tmp/home" PATH=/usr/bin:/bin TASKFLEET_HOME="$tmp/state-$i" \
    "$compat" "${args[@]}" >"$tmp/compat.out" 2>"$tmp/compat.err"
  compat_status=$?
  set -e
  [[ "$canonical_status" == "$compat_status" ]] || { echo "exit mismatch: ${commands[$i]}" >&2; exit 1; }
  cmp "$tmp/canon.out" "$tmp/compat.out" || { echo "stdout mismatch: ${commands[$i]}" >&2; exit 1; }
  [[ "$(grep -cF '`orchestratectl` is deprecated' "$tmp/compat.err" || true)" == 1 ]] || {
    echo "compat deprecation count mismatch: ${commands[$i]}" >&2; exit 1;
  }
  grep -vF '`orchestratectl` is deprecated' "$tmp/compat.err" >"$tmp/compat.filtered.err" || true
  cmp "$tmp/canon.err" "$tmp/compat.filtered.err" || { echo "non-deprecation stderr mismatch: ${commands[$i]}" >&2; exit 1; }
  if [[ " ${commands[$i]} " == *' jsonl '* ]]; then
    jq -c . <"$tmp/compat.out" >/dev/null
  fi
done

# Invalid input preserves stdout and exit status too; warning remains stderr-only.
set +e
env -i HOME="$tmp/home" PATH=/usr/bin:/bin TASKFLEET_HOME="$tmp/canon-invalid" "$canonical" --output bogus version >"$tmp/canon.out" 2>"$tmp/canon.err"
canonical_status=$?
env -i HOME="$tmp/home" PATH=/usr/bin:/bin TASKFLEET_HOME="$tmp/compat-invalid" "$compat" --output bogus version >"$tmp/compat.out" 2>"$tmp/compat.err"
compat_status=$?
set -e
[[ "$canonical_status" == "$compat_status" && "$canonical_status" != 0 ]]
cmp "$tmp/canon.out" "$tmp/compat.out"
[[ "$(grep -cF '`orchestratectl` is deprecated' "$tmp/compat.err" || true)" == 1 ]]
grep -vF '`orchestratectl` is deprecated' "$tmp/compat.err" >"$tmp/compat.filtered.err" || true
cmp "$tmp/canon.err" "$tmp/compat.filtered.err"

# The documented invocation-only differences are bounded to help branding and a
# single warning; hidden self-exec suppresses the warning.
env -i HOME="$tmp/home" PATH=/usr/bin:/bin TASKFLEET_HOME="$tmp/help" "$compat" --help >"$tmp/help.out" 2>"$tmp/help.err"
grep -F 'Usage: orchestratectl' "$tmp/help.out" >/dev/null
[[ "$(grep -cF '`orchestratectl` is deprecated' "$tmp/help.err" || true)" == 1 ]]
env -i HOME="$tmp/home" PATH=/usr/bin:/bin TASKFLEET_HOME="$tmp/hidden" OCTL_INTERNAL_SELF_EXEC=1 \
  "$compat" --output jsonl version >"$tmp/hidden.out" 2>"$tmp/hidden.err"
[[ ! -s "$tmp/hidden.err" ]]
jq -c . <"$tmp/hidden.out" >/dev/null
public_count="$(jq -r .count "${R8_PARITY_INVENTORY:-$tmp/public-command-inventory.json}")"
printf 'both-name command parity passed (%d public command surfaces, %d ordinary commands plus invalid/help/hidden checks)\n' "$public_count" "${#commands[@]}"
