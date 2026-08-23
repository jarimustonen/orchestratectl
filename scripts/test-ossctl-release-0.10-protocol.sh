#!/usr/bin/env bash
# Safe real-engine protocol test for the exact fleet ossctl 0.10.1 build admitted
# by scripts/ossctl-release.sh. Uses only a local bare remote and stops at the
# exact-SHA CI lookup, before the held release tag can be pushed.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
readonly expected_commit="6879e040a520a7a9c6196ed77791b4f2f10ad6f4"
tmp="$(mktemp -d "${TMPDIR:-/tmp}/ossctl-010-protocol.XXXXXX")"
cleanup() { rm -rf "$tmp"; }
trap cleanup EXIT
mkdir -p "$tmp/bin" "$tmp/home" "$tmp/cargo"

real_git="$(command -v git)"
real_ossctl="$(command -v ossctl)"
version_json="$($real_ossctl version --json)"
jq -e --arg commit "$expected_commit" '
  .schema_version == 1 and .data.schema_version == 1 and
  .data.version == "0.10.1" and .data.commit == $commit
' <<<"$version_json" >/dev/null || {
  echo "test requires the validated ossctl 0.10.1 commit $expected_commit" >&2
  exit 1
}

test -z "$(git -C "$repo_root" status --porcelain)" || {
  echo "real ossctl protocol test requires a clean source tree" >&2
  exit 1
}

for tool in bash jq sed awk grep mktemp rm mkdir chmod mv dirname pwd sleep; do
  tool_path="$(command -v "$tool")" || { echo "test prerequisite missing: $tool" >&2; exit 1; }
  ln -s "$tool_path" "$tmp/bin/$tool"
done
# Never put rustup proxies in the isolated PATH: with an isolated HOME they can
# select/update a global toolchain. Pin this test to the already-installed active
# toolchain's real binaries instead.
toolchain_bin="$(dirname "$(rustup which cargo)")"
for tool in cargo rustc rustdoc; do
  test -x "$toolchain_bin/$tool" || { echo "active toolchain is missing $tool" >&2; exit 1; }
  ln -s "$toolchain_bin/$tool" "$tmp/bin/$tool"
done
cat >"$tmp/bin/ossctl" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >>"$OSSCTL_ARGV_LOG"
exec "$OSSCTL_REAL_BIN" "$@"
STUB
chmod +x "$tmp/bin/ossctl"
for tool in dist cargo-dist; do
  tool_path="$(command -v "$tool")" || { echo "test prerequisite missing: $tool" >&2; exit 1; }
  ln -s "$tool_path" "$tmp/bin/$tool"
done

cat >"$tmp/bin/git" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >>"$GIT_STUB_LOG"
case "$*" in
  "remote get-url origin"|"remote get-url --push --all origin")
    printf '%s\n' 'git@github.com:jarimustonen/orchestratectl.git'
    ;;
  "push origin refs/tags/v"*)
    printf '%s\n' "$*" >>"$TAG_PUSH_LOG"
    # Keep the transport local while supplying the production remote coordinate
    # to the wrapper-created hook. The wrapper already probes this hook through
    # a real local Git push before ossctl reaches this path.
    ref="${3:-}"
    local_ref="${ref%%:*}"
    remote_ref="${ref#*:}"
    [[ "$remote_ref" != "$ref" ]] || remote_ref="$local_ref"
    oid="$($REAL_GIT rev-parse --verify "$local_ref")"
    zeros="$(printf '%040d' 0)"
    printf '%s %s %s %s\n' "$local_ref" "$oid" "$remote_ref" "$zeros" |
      "$GIT_CONFIG_VALUE_0/pre-push" origin git@github.com:jarimustonen/orchestratectl.git
    ;;
  "push origin HEAD:refs/heads/main"|push\ */probe.git\ refs/tags/v0.0.0-ossctl-pretag-probe)
    exec "$REAL_GIT" "$@"
    ;;
  push\ *)
    echo "git protocol stub: unexpected push form: $*" >&2
    exit 97
    ;;
  *) exec "$REAL_GIT" "$@" ;;
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
echo "gh protocol stub: unexpected arguments: $*" >&2
exit 98
STUB
chmod +x "$tmp/bin/gh"

git init --bare -q "$tmp/origin.git"
git -C "$repo_root" push -q "$tmp/origin.git" HEAD:refs/heads/main
git clone -q --branch main "$tmp/origin.git" "$tmp/repo"
git -C "$tmp/repo" config user.name protocol-test
git -C "$tmp/repo" config user.email protocol-test@example.invalid

run_env=(
  env -i
  HOME="$tmp/home"
  CARGO_HOME="$tmp/cargo"
  TMPDIR="$tmp"
  PATH="$tmp/bin:/usr/bin:/bin"
  REAL_GIT="$real_git"
  OSSCTL_REAL_BIN="$real_ossctl"
  OSSCTL_ARGV_LOG="$tmp/ossctl.log"
  GIT_STUB_LOG="$tmp/git.log"
  TAG_PUSH_LOG="$tmp/tag-push.log"
  GH_STUB_LOG="$tmp/gh.log"
  GIT_CONFIG_GLOBAL=/dev/null
  GIT_CONFIG_NOSYSTEM=1
  GIT_TERMINAL_PROMPT=0
)

(
  cd "$tmp/repo"
  "${run_env[@]}" ossctl contract show --json --require-approved |
    jq -e '(.data.targets | length > 0) and
      (.data.targets | all(.adapter == "cargo-publish-ci" or .adapter == "cargo-dist"))' >/dev/null || {
      echo "refusing real protocol cut: every publish target must be delegated to CI" >&2
      exit 1
    }
  "${run_env[@]}" ./scripts/ossctl-release.sh plan minor >"$tmp/plan.json"
)
plan_id="$(jq -er '.data.plan_id' "$tmp/plan.json")"
jq -e '
  .schema_version == 1 and .data.bump.level == "minor" and
  (.data.bump.from_version | type == "string") and
  (.data.bump.to_version | type == "string") and
  .data.bump.from_version != .data.bump.to_version
' "$tmp/plan.json" >/dev/null

# Prove the engine, not the wrapper's JSON extraction, is the seal authority.
plan_file="$tmp/repo/.git/ossctl/plans/$plan_id.json"
cp "$plan_file" "$tmp/plan.original.json"
jq '.plan.bump.level = "major"' "$plan_file" >"$tmp/plan.tampered.json"
mv "$tmp/plan.tampered.json" "$plan_file"
set +e
(
  cd "$tmp/repo"
  "${run_env[@]}" ossctl release cut --plan "$plan_id" --bump major --json
) >"$tmp/tamper.stdout" 2>"$tmp/tamper.stderr"
tamper_status=$?
set -e
[[ "$tamper_status" -ne 0 ]] || { echo "ossctl accepted a plan with an invalid seal" >&2; exit 1; }
mv "$tmp/plan.original.json" "$plan_file"
(
  cd "$tmp/repo"
  "${run_env[@]}" ossctl release list --json |
    jq -e '.data.in_flight_count == 0 and (.data.unreadable | length) == 0' >/dev/null
)

set +e
(
  cd "$tmp/repo"
  "${run_env[@]}" ./scripts/ossctl-release.sh cut "$plan_id"
) >"$tmp/cut.stdout" 2>"$tmp/cut.stderr"
status=$?
set -e
[[ "$status" -eq 42 ]] || {
  echo "real ossctl cut did not stop at the isolated exact-SHA CI gate (status=$status)" >&2
  cat "$tmp/cut.stderr" >&2
  exit 1
}
grep -F reached-exact-sha-ci-gate "$tmp/cut.stderr" >/dev/null

(
  cd "$tmp/repo"
  "${run_env[@]}" ossctl release list --json >"$tmp/list.json"
)
run_id="$(jq -er --arg plan "$plan_id" '
  [.data.runs[] | select(.plan_id == $plan and .in_flight)] |
  if length == 1 then .[0].run_id else error("expected one held run") end
' "$tmp/list.json")"
(
  cd "$tmp/repo"
  "${run_env[@]}" ossctl release show "$run_id" --json >"$tmp/show.json"
)
tag="$(jq -er '.data.state.tags | keys | if length == 1 then .[0] else error("one tag required") end' "$tmp/show.json")"
bump_commit="$(jq -er '.data.state.bump.commit' "$tmp/show.json")"
expected_gh="run list -R jarimustonen/orchestratectl --workflow ci.yml --branch main --commit $bump_commit --event push --limit 1 --json databaseId -q .[0].databaseId"
grep -Fx "$expected_gh" "$tmp/gh.log" >/dev/null || {
  echo "real protocol test did not query push CI for the exact bump SHA" >&2
  cat "$tmp/gh.log" >&2
  exit 1
}
grep -Fx "release cut --plan $plan_id --bump minor --json" "$tmp/ossctl.log" >/dev/null || {
  echo "wrapper did not pass the sealed minor bump input to ossctl 0.10.1" >&2
  cat "$tmp/ossctl.log" >&2
  exit 1
}
[[ "$(wc -l <"$tmp/tag-push.log" | tr -d ' ')" == 1 ]] || {
  echo "expected exactly one held release-tag push attempt" >&2
  cat "$tmp/tag-push.log" >&2
  exit 1
}
jq -e --arg tag "$tag" '
  .data.state.status == "in_progress" and .data.state.current_phase == null and
  [.data.state.phases[] | {phase,outcome}] == [
    {phase:"bump",outcome:"ok"},{phase:"dry_run",outcome:"ok"},
    {phase:"build",outcome:"ok"},{phase:"publish",outcome:"ok"},
    {phase:"tag",outcome:"failed"}
  ] and
  .data.state.tags[$tag].created_local == true and
  .data.state.tags[$tag].pushed_remote == false and
  .data.state.tags[$tag].github_release == false and
  .data.state.tags[$tag].github_release_delegated == false and
  .data.last_seq == .data.state.applied_seq and
  [.data.recent_events[-3:][] | {kind,phase,tag,outcome}] == [
    {kind:"phase_entered",phase:"tag",tag:null,outcome:null},
    {kind:"tag_created_local",phase:null,tag:$tag,outcome:null},
    {kind:"phase_completed",phase:"tag",tag:null,outcome:"failed"}
  ]
' "$tmp/show.json" >/dev/null
[[ "$(git -C "$tmp/repo" rev-parse "$tag^{commit}")" == "$bump_commit" ]]
test -z "$(git -C "$tmp/repo" ls-remote --tags origin "refs/tags/$tag" "refs/tags/$tag^{}")"
test -s "$tmp/repo/.git/ossctl-held-tags/$run_id.json"

# The wrapper must have supplied 0.10.1's matching --bump argument: reaching
# the tag checkpoint proves cut revalidated and executed the sealed bump plan.
echo "ossctl 0.10.1 real protocol test passed (held $run_id at $tag; remote tag absent)"
