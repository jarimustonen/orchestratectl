#!/usr/bin/env bash
# End-to-end regression for the ossctl 0.9 intentionally-held tag journal.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/bin" "$tmp/home" "$tmp/work" "$tmp/common/ossctl-held-tags"

for tool in bash jq sed awk grep mktemp rm mkdir chmod mv dirname pwd; do
  tool_path="$(command -v "$tool")" || { echo "test prerequisite missing: $tool" >&2; exit 1; }
  ln -s "$tool_path" "$tmp/bin/$tool"
done

version="$(awk -F'"' '/^version = / { print $2; exit }' "$repo_root/Cargo.toml")"
tag="v$version"
bump_commit=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
tag_oid=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
run_id=01M0TESTHELD00000000000000

cat >"$tmp/bin/git" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >>"$GIT_STUB_LOG"
if [[ "$*" == "rev-parse $TAG^{commit}" || "$*" == "rev-parse --verify refs/tags/$TAG^{commit}" ]]; then
  [[ "${HELD_VARIANT:-valid}" != missing-local-tag ]] || exit 1
  if [[ "${HELD_VARIANT:-valid}" == wrong-tag-target ]]; then
    printf '%040d\n' 8
  else
    printf '%s\n' "$BUMP_COMMIT"
  fi
  exit 0
fi
if [[ "$*" == "rev-parse --verify refs/tags/$TAG" ]]; then
  if [[ "${HELD_VARIANT:-valid}" == moved-local-tag ]]; then
    printf '%040d\n' 9
  else
    printf '%s\n' "$TAG_OID"
  fi
  exit 0
fi
case "$*" in
  "rev-parse --show-toplevel") printf '%s\n' "$GIT_STUB_ROOT" ;;
  "remote get-url origin"|"remote get-url --push --all origin") printf '%s\n' 'git@github.com:jarimustonen/orchestratectl.git' ;;
  "cat-file -e "*) [[ "${HELD_VARIANT:-valid}" != missing-local-tag ]] ;;
  "rev-parse --git-common-dir") printf '%s\n' "$GIT_COMMON" ;;
  "ls-remote --tags origin "*)
    [[ "${HELD_VARIANT:-valid}" != remote-present ]] || printf '%s\trefs/tags/%s\n' "$BUMP_COMMIT" "$TAG"
    ;;
  "merge-base --is-ancestor "*) [[ "${HELD_VARIANT:-valid}" != bad-ancestry ]] ;;
  "fetch origin +refs/heads/main:refs/remotes/origin/main") ;;
  "rev-parse HEAD"|"rev-parse origin/main") printf '%s\n' "$BUMP_COMMIT" ;;
  "status --porcelain") ;;
  *) echo "stub git: unexpected arguments: $*" >&2; exit 96 ;;
esac
STUB
chmod +x "$tmp/bin/git"

cat >"$tmp/bin/gh" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >>"$GH_STUB_LOG"
if [[ "$*" == 'repo view jarimustonen/orchestratectl --json nameWithOwner -q .nameWithOwner' ]]; then
  printf '%s\n' jarimustonen/orchestratectl
  exit 0
fi
if [[ "$*" == run\ list* ]]; then
  echo reached-exact-sha-ci-gate >&2
  exit 42
fi
echo "stub gh: unexpected arguments: $*" >&2
exit 98
STUB
chmod +x "$tmp/bin/gh"

cat >"$tmp/bin/ossctl" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >>"$OSSCTL_STUB_LOG"
if [[ "$*" == 'version --json' ]]; then
  printf '%s\n' '{"data":{"version":"0.9.0"}}'
  exit 0
fi
if [[ "$*" == "release show $RUN_ID --json" ]]; then
  pushed_event=''
  outcome=failed
  [[ "${HELD_VARIANT:-valid}" != unexpected-event ]] || pushed_event=',{"seq":25,"kind":"tag_pushed_remote","tag":"'"$TAG"'"}'
  [[ "${HELD_VARIANT:-valid}" != wrong-phase ]] || outcome=ok
  jq -n --arg run "$RUN_ID" --arg tag "$TAG" --arg bump "$BUMP_COMMIT" --arg outcome "$outcome" --argjson pushed "${pushed_event:-null}" '
    {data:{last_seq:25,state:{schema_version:5,run_id:$run,status:"in_progress",current_phase:null,applied_seq:25,
      bump:{commit:$bump},phases:[
        {phase:"bump",outcome:"ok"},{phase:"dry_run",outcome:"ok"},{phase:"build",outcome:"ok"},
        {phase:"publish",outcome:"ok"},{phase:"tag",outcome:$outcome}],
      tags:{($tag):{created_local:true,pushed_remote:false,github_release:false,github_release_delegated:false}}},
      recent_events:([{"seq":23,"kind":"phase_entered","phase":"tag"},{"seq":24,"kind":"tag_created_local","tag":$tag}]
        + (if $pushed == null then [] else [$pushed] end)
        + [{"seq":25,"kind":"phase_completed","phase":"tag","outcome":$outcome}])}}'
  exit 0
fi
echo "stub ossctl: unexpected arguments: $*" >&2
exit 99
STUB
chmod +x "$tmp/bin/ossctl"

write_checkpoint() {
  local remote="${1:-git@github.com:jarimustonen/orchestratectl.git}"
  jq -n --arg run "$run_id" --arg tag "$tag" --arg bump "$bump_commit" --arg oid "$tag_oid" --arg remote "$remote" '
    {schema_version:1,run_id:$run,tag:$tag,bump_commit:$bump,
     marker:["origin",$remote,"refs/tags/"+$tag,$oid,"refs/tags/"+$tag,"0000000000000000000000000000000000000000"]}' \
    >"$tmp/common/ossctl-held-tags/$run_id.json"
}

run_case() {
  local variant="$1"
  env -i HOME="$tmp/home" PATH="$tmp/bin" \
    HELD_VARIANT="$variant" GIT_STUB_LOG="$tmp/git.log" GH_STUB_LOG="$tmp/gh.log" \
    OSSCTL_STUB_LOG="$tmp/ossctl.log" GIT_STUB_ROOT="$repo_root" GIT_COMMON="$tmp/common" \
    BUMP_COMMIT="$bump_commit" TAG_OID="$tag_oid" TAG="$tag" RUN_ID="$run_id" \
    "$repo_root/scripts/ossctl-release.sh" resume "$run_id" >"$tmp/stdout" 2>"$tmp/stderr"
}

assert_rejected_before_gate() {
  local variant="$1"
  : >"$tmp/gh.log"
  set +e
  run_case "$variant"
  status=$?
  set -e
  [[ "$status" -ne 0 && "$status" -ne 42 ]] || {
    echo "$variant was not rejected before the exact-SHA gate (status=$status)" >&2
    cat "$tmp/stderr" >&2
    exit 1
  }
  ! grep -F 'run list' "$tmp/gh.log" >/dev/null || {
    echo "$variant reached the exact-SHA gate" >&2
    exit 1
  }
}

write_checkpoint
set +e
run_case valid
status=$?
set -e
[[ "$status" -eq 42 ]] || {
  echo "valid held journal did not reach exact-SHA gate (status=$status)" >&2
  cat "$tmp/stderr" "$tmp/git.log" "$tmp/ossctl.log" "$tmp/gh.log" >&2
  exit 1
}
grep -F reached-exact-sha-ci-gate "$tmp/stderr" >/dev/null

rm -f "$tmp/common/ossctl-held-tags/$run_id.json"
assert_rejected_before_gate valid
write_checkpoint https://github.com/unrelated/repo.git
assert_rejected_before_gate valid
write_checkpoint
for variant in remote-present missing-local-tag moved-local-tag wrong-tag-target bad-ancestry wrong-phase unexpected-event; do
  assert_rejected_before_gate "$variant"
done

echo "release wrapper held-tag journal tests passed"
