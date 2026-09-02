#!/bin/sh
set -eu

compat=false
if [ "${1:-}" = "--compat" ]; then compat=true; shift; fi
bin=${1:-orchestratectl}
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
tmp=$(mktemp -d "${TMPDIR:-/tmp}/orchestratectl-051-verify.XXXXXX")
trap 'rm -rf "$tmp"' EXIT HUP INT TERM

# The committed corpus is immutable evidence. Refuse drift and unexpected links
# before a binary gets a chance to touch a disposable copy.
if find "$here/home" "$here/repo" -type l | grep . >/dev/null; then
  echo "fixture must not contain symlinks" >&2; exit 1
fi
(cd "$here" && shasum -a 256 -c SHA256SUMS >/dev/null)
cp -R "$here/home" "$tmp/home"
cp -R "$here/repo" "$tmp/repo"

export HOME="$tmp/home/user"
export ORCHESTRATECTL_HOME="$tmp/home/orchestratectl"
# Ambient selectors/test seams would make the check host-dependent.
unset ORCHESTRATECTL_PROFILE ORCHESTRATECTL_HARNESS ORCHESTRATECTL_LOG || true
unset OCTL_CREATE_SH OCTL_MERGE_SH OCTL_SUPERVISE_BIN OCTL_TEST_SKIP_MATERIALIZE || true
unset OCTL_RUN_ID OCTL_NODE_ID OCTL_ATTEMPT OCTL_STATUS OCTL_SUMMARY || true

hash_protected() {
  (cd "$tmp" && find home repo -type f \
    ! -path 'home/orchestratectl/logs/*' \
    ! -path 'home/user/.taskfleet-migrations/*' -print0 \
    | LC_ALL=C sort -z | xargs -0 shasum -a 256)
}
before=$(hash_protected)

version=$("$bin" version --output json)
if ! $compat; then
  [ "$(printf '%s' "$version" | jq -r '.data.version')" = 0.5.1 ] || {
    echo "baseline mode requires orchestratectl 0.5.1 (use --compat for a newer reader)" >&2
    exit 1
  }
  [ "$(printf '%s' "$version" | jq -r '.data.commit')" = f0c52ab232706fb480a51bfd45f2171c6b7aa056 ] || {
    echo "baseline mode requires the published 0.5.1 commit" >&2; exit 1
  }
fi

list=$(cd "$tmp/repo" && "$bin" run list --output json)
printf '%s' "$list" | jq -e '
  (.data.runs | length) == 4 and
  ([.data.runs[] | .run_id] | sort) == [
    "01j00000000000000000000001", "01j00000000000000000000002",
    "01j00000000000000000000003", "01j00000000000000000000004"] and
  any(.data.runs[]; .run_id == "01j00000000000000000000001" and .status == "done") and
  any(.data.runs[]; .run_id == "01j00000000000000000000002" and .status == "pending") and
  any(.data.runs[]; .run_id == "01j00000000000000000000003" and .status == "pending") and
  any(.data.runs[]; .run_id == "01j00000000000000000000004" and .kind == "unknown")
' >/dev/null

for id in 01j00000000000000000000001 01j00000000000000000000002 \
          01j00000000000000000000003 01j00000000000000000000004; do
  "$bin" run show "$id" --output json >/dev/null
done

runs="$ORCHESTRATECTL_HOME/runs"
pending="$runs/01j00000000000000000000003/nodes/n-0001.json"
jq -e '
  .status == "running" and
  .pending_merge.op_id == "01j0000000000000000000000a" and
  .pending_merge.expected_source_oid == "1111111111111111111111111111111111111111" and
  .pending_merge.worker_oid == "2222222222222222222222222222222222222222"
' "$pending" >/dev/null
grep -q '"kind":"code"' "$runs/01j00000000000000000000004/events.jsonl"

for dir in "$runs"/*; do
  last=$(tail -n 1 "$dir/events.jsonl" | jq -r .seq)
  [ "$(jq -r .applied_seq "$dir/manifest.json")" = "$last" ]
done
jq -e '.status == "done" and .last_report.success == true and .last_report.origin.kind == "agent"' \
  "$runs/01j00000000000000000000001/nodes/n-0001.json" >/dev/null

config=$(cd "$tmp/repo" && "$bin" config show --output json)
printf '%s' "$config" | jq -e '.data.valid == true and .data.schema_version_config == 2' >/dev/null
grep -q '^\[future-readable\]$' "$ORCHESTRATECTL_HOME/config.toml"
grep -q '^default = "fixture-capable"$' "$tmp/repo/.orchestratectl.toml"

"$bin" doctor --output json | jq -e '
  [.data.checks[] | select(.id | startswith("schema.runs.")) | .status] |
  length == 4 and all(. == "ok")
' >/dev/null

record="$ORCHESTRATECTL_HOME/state/pi-installed-skills.json"
pi_skill="$HOME/.pi/agent/skills/orchestratectl-overview/SKILL.md"
[ "$(jq -r .schema_version "$record")" = 3 ]
recorded=$(jq -r '.skills["orchestratectl-overview"].files["SKILL.md"].sha256' "$record")
actual=$(shasum -a 256 "$pi_skill" | awk '{print $1}')
[ "$recorded" = "$actual" ]
grep -q '^managed-by: orchestratectl$' "$HOME/.claude/skills/orchestratectl-overview/.orchestratectl-managed"
grep -q '^skill_name: orchestratectl-overview$' "$HOME/.claude/skills/orchestratectl-overview/.orchestratectl-managed"
grep -q '^managed-by: orchestratectl$' "$HOME/.codex/prompts/_shared/.orchestratectl-managed"
grep -q '^prompt: orchestratectl-overview$' "$HOME/.codex/prompts/_shared/.orchestratectl-managed"

after=$(hash_protected)
[ "$before" = "$after" ] || { echo "protected fixture files changed during read checks" >&2; exit 1; }

echo "orchestratectl 0.5.1 fixture verification passed"
