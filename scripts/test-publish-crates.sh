#!/usr/bin/env bash
# Credential-free registry reconciliation tests. Every mutating/network boundary
# is stubbed; the fixture never reaches crates.io or a real cargo publish.
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
tmp="$(mktemp -d "${TMPDIR:-/tmp}/publish-crates-test.XXXXXX")"
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/bin" "$tmp/repo/scripts" "$tmp/repo/release" "$tmp/repo/crates/taskfleet-core" "$tmp/repo/crates/taskfleet" "$tmp/repo/compat/orchestratectl"
fixture_root="$(cd "$tmp/repo" && pwd -P)"
cp "$repo_root/scripts/publish-crates.sh" "$tmp/repo/scripts/"
cp "$repo_root/release/taskfleet-release.json" "$tmp/repo/release/"
cat >"$tmp/repo/Cargo.toml" <<'EOF'
[workspace]
members = []
[workspace.package]
version = "1.2.3"
EOF
for tool in bash jq awk grep mktemp rm mkdir tar sha256sum cat cp dirname; do
  path="$(command -v "$tool")" || { echo "missing test prerequisite: $tool" >&2; exit 1; }
  ln -s "$path" "$tmp/bin/$tool"
done
cat >"$tmp/bin/git" <<'STUB'
#!/usr/bin/env bash
[[ "$*" == "rev-parse HEAD" ]] || exit 90
printf '%s\n' 1111111111111111111111111111111111111111
STUB
cat >"$tmp/bin/cargo" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >>"$CARGO_LOG"
if [[ "$1" == metadata ]]; then
  jq -n --arg root "$FIXTURE_ROOT" '{packages:[
    {name:"taskfleet-core",version:"1.2.3",manifest_path:($root+"/crates/taskfleet-core/Cargo.toml"),repository:"https://github.com/jarimustonen/orchestratectl",homepage:"https://github.com/jarimustonen/orchestratectl",license:"MIT",rust_version:"1.85",description:"core",dependencies:[]},
    {name:"taskfleet",version:"1.2.3",manifest_path:($root+"/crates/taskfleet/Cargo.toml"),repository:"https://github.com/jarimustonen/orchestratectl",homepage:"https://github.com/jarimustonen/orchestratectl",license:"MIT",rust_version:"1.85",description:"cli",dependencies:[{name:"taskfleet-core",req:"=1.2.3"}]},
    {name:"orchestratectl",version:"1.2.3",manifest_path:($root+"/compat/orchestratectl/Cargo.toml"),repository:"https://github.com/jarimustonen/orchestratectl",homepage:"https://github.com/jarimustonen/orchestratectl",license:"MIT",rust_version:"1.85",description:"compat",dependencies:[{name:"taskfleet",req:"=1.2.3"}]}
  ]}'
  exit 0
fi
if [[ "$1" == package ]]; then
  package="${*: -1}"
  [[ "$package" != --no-verify ]] || package=taskfleet-core
  root="$FIXTURE_ROOT/target/package/$package-1.2.3"
  mkdir -p "$root" "$FIXTURE_ROOT/target/package"
  printf '%s\n' '[package]' "name = \"$package\"" 'version = "1.2.3"' 'version = "=1.2.3"' >"$root/Cargo.toml"
  jq -n --arg sha "${ARCHIVE_COMMIT:-1111111111111111111111111111111111111111}" '{git:{sha1:$sha}}' >"$root/.cargo_vcs_info.json"
  tar -czf "$FIXTURE_ROOT/target/package/$package-1.2.3.crate" -C "$FIXTURE_ROOT/target/package" "$package-1.2.3"
  rm -rf "$root"
  exit 0
fi
if [[ "$1" == publish ]]; then
  [[ -z "${CARGO_REGISTRY_TOKEN:-}" ]] || { echo 'test unexpectedly received credentials' >&2; exit 91; }
  echo 'error: crate taskfleet@1.2.3 already exists on crates.io index' >&2
  exit "${PUBLISH_STATUS:-101}"
fi
exit 92
STUB
cat >"$tmp/bin/curl" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
output=''; url=''
while [[ $# -gt 0 ]]; do
  case "$1" in
    -o) output="$2"; shift 2 ;;
    -A|-w) shift 2 ;;
    -sS|-L) shift ;;
    *) url="$1"; shift ;;
  esac
done
[[ -n "$output" && -n "$url" ]] || exit 93
package=taskfleet
archive="$FIXTURE_ROOT/target/package/$package-1.2.3.crate"
case "$url" in
  */crates/taskfleet/1.2.3)
    count=0; [[ -f "$CURL_COUNT" ]] && count="$(cat "$CURL_COUNT")"; count=$((count+1)); printf '%s' "$count" >"$CURL_COUNT"
    if [[ "$REGISTRY_MODE" == absent || ("$REGISTRY_MODE" == duplicate-match && "$count" -eq 1) ]]; then : >"$output"; printf 404; exit 0; fi
    checksum="$(sha256sum "$archive" | awk '{print $1}')"
    description=cli; [[ "$REGISTRY_MODE" != metadata-mismatch ]] || description=wrong
    jq -n --arg checksum "$checksum" --arg description "$description" '{version:{checksum:$checksum,license:"MIT",rust_version:"1.85"},crate:{repository:"https://github.com/jarimustonen/orchestratectl",homepage:"https://github.com/jarimustonen/orchestratectl",description:$description}}' >"$output"
    printf 200 ;;
  */crates/taskfleet/owners)
    owner=jarimustonen; [[ "$REGISTRY_MODE" != owner-mismatch ]] || owner=intruder
    jq -n --arg owner "$owner" '{users:[{login:$owner}]}' >"$output"; printf 200 ;;
  */crates/taskfleet/1.2.3/dependencies)
    req='=1.2.3'; [[ "$REGISTRY_MODE" != dependency-mismatch ]] || req='^1.2.3'
    jq -n --arg req "$req" '{dependencies:[{crate_id:"taskfleet-core",req:$req}]}' >"$output"; printf 200 ;;
  */crates/taskfleet/1.2.3/download)
    cp "$archive" "$output"
    [[ "$REGISTRY_MODE" != checksum-mismatch ]] || printf 'corrupt' >>"$output"
    printf 200 ;;
  *) exit 94 ;;
esac
STUB
cat >"$tmp/bin/sleep" <<'STUB'
#!/usr/bin/env bash
exit 0
STUB
chmod +x "$tmp/bin/"*

run_case() {
  local mode="$1" expected="$2" diagnostic="${3:-}"
  rm -rf "$tmp/repo/target"; : >"$tmp/cargo.log"; rm -f "$tmp/curl-count"
  set +e
  env -i HOME="$tmp" PATH="$tmp/bin" FIXTURE_ROOT="$fixture_root" CARGO_LOG="$tmp/cargo.log" CURL_COUNT="$tmp/curl-count" \
    REGISTRY_MODE="$mode" RELEASE_RECEIPT_DIR="$tmp/receipts-$mode" SOURCE_COMMIT=1111111111111111111111111111111111111111 \
    "$tmp/repo/scripts/publish-crates.sh" publish taskfleet >"$tmp/$mode.out" 2>"$tmp/$mode.err"
  status=$?
  set -e
  [[ "$status" -eq "$expected" ]] || { echo "$mode expected $expected, got $status" >&2; cat "$tmp/$mode.err" >&2; exit 1; }
  if [[ -n "$diagnostic" ]]; then grep -F "$diagnostic" "$tmp/$mode.err" >/dev/null || { cat "$tmp/$mode.err" >&2; exit 1; }; fi
}

run_case match 0
! grep -q '^publish ' "$tmp/cargo.log" || { echo 'matching existing crate was republished' >&2; exit 1; }
[[ -s "$tmp/receipts-match/taskfleet-1.2.3.json" ]]
run_case duplicate-match 0
grep -q '^publish ' "$tmp/cargo.log" || { echo 'absent crate did not attempt publish' >&2; exit 1; }
run_case metadata-mismatch 2 'registry metadata mismatch'
run_case owner-mismatch 2 'registry owner set mismatch'
run_case dependency-mismatch 2 'registry dependency requirements mismatch'
run_case checksum-mismatch 2 'registry archive differs from the sealed local archive'
run_case absent 1 'is not index-visible after publish'

# A mismatched source receipt fails before any registry or publish side effect.
rm -rf "$tmp/repo/target"; : >"$tmp/cargo.log"
set +e
env -i HOME="$tmp" PATH="$tmp/bin" FIXTURE_ROOT="$fixture_root" CARGO_LOG="$tmp/cargo.log" CURL_COUNT="$tmp/curl-count" \
  REGISTRY_MODE=match ARCHIVE_COMMIT=2222222222222222222222222222222222222222 SOURCE_COMMIT=1111111111111111111111111111111111111111 \
  "$tmp/repo/scripts/publish-crates.sh" publish taskfleet >"$tmp/source.out" 2>"$tmp/source.err"
status=$?
set -e
[[ "$status" -eq 2 ]]
grep -F 'archive source commit' "$tmp/source.err" >/dev/null
! grep -q '^publish ' "$tmp/cargo.log"

echo 'crates.io reconciliation tests passed'
