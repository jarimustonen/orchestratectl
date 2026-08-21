#!/usr/bin/env bash
# Non-mutating repository-preflight regression tests with a minimal PATH.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
tmp="$(mktemp -d)"
cleanup() {
  rm -rf "$tmp"
}
trap cleanup EXIT

mkdir -p "$tmp/bin" "$tmp/home" "$tmp/work"
for tool in bash jq sed; do
  tool_path="$(command -v "$tool")" || {
    echo "test prerequisite missing: $tool" >&2
    exit 1
  }
  ln -s "$tool_path" "$tmp/bin/$tool"
done

cat >"$tmp/bin/git" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >>"$GIT_STUB_LOG"
case "$*" in
  "rev-parse --show-toplevel")
    printf '%s\n' "$GIT_STUB_ROOT"
    ;;
  "remote get-url origin"|"remote get-url --push --all origin")
    printf '%s\n' "$GIT_STUB_ORIGIN"
    ;;
  *)
    echo "stub git: unexpected arguments: $*" >&2
    exit 96
    ;;
esac
STUB
chmod +x "$tmp/bin/git"

cat >"$tmp/bin/gh" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >>"$GH_STUB_LOG"

# repo view accepts the repository as its documented positional argument. Reject
# its unsupported forms explicitly without condemning valid `gh run -R` calls.
if [[ "${1:-}" == repo && "${2:-}" == view ]]; then
  for arg in "$@"; do
    case "$arg" in
      -R|-R*|--repo|--repo=*)
        echo "stub gh: repo view repository flags are unsupported" >&2
        exit 97
        ;;
    esac
  done
fi

if [[ $# -eq 7 && "$1" == repo && "$2" == view &&
      "$3" == jarimustonen/orchestratectl && "$4" == --json &&
      "$5" == nameWithOwner && "$6" == -q && "$7" == .nameWithOwner ]]; then
  printf '%s\n' "$GH_STUB_REPO"
  exit 0
fi

echo "stub gh: unexpected arguments: $*" >&2
exit 98
STUB
chmod +x "$tmp/bin/gh"

cat >"$tmp/bin/ossctl" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >>"$OSSCTL_STUB_LOG"
if [[ "$*" == "version --json" ]]; then
  jq -n --arg version "${OSSCTL_STUB_VERSION:-0.10.0}" --arg commit "${OSSCTL_STUB_COMMIT:-a35b9917fc65a6354fe855b7c956521b47669907}" \
    '{schema_version:1,data:{version:$version,commit:$commit,schema_version:1}}'
  exit 0
fi
if [[ "${1:-}" == release && "${2:-}" == show ]]; then
  # Sentinel: reaching this point proves repository preflight passed. Stop before
  # any release state, Git refs, or remote operations can be touched.
  exit 42
fi
echo "stub ossctl: unexpected arguments: $*" >&2
exit 99
STUB
chmod +x "$tmp/bin/ossctl"

run_wrapper() {
  local gh_repo="${1:-jarimustonen/orchestratectl}"
  local origin="${2:-git@github.com:jarimustonen/orchestratectl.git}"
  env -i \
    HOME="$tmp/home" \
    PATH="$tmp/bin" \
    GH_STUB_LOG="$tmp/gh.log" \
    GIT_STUB_LOG="$tmp/git.log" \
    GIT_STUB_ROOT="$tmp/work" \
    GIT_STUB_ORIGIN="$origin" \
    OSSCTL_STUB_LOG="$tmp/ossctl.log" \
    OSSCTL_STUB_VERSION="${OSSCTL_STUB_VERSION:-0.10.0}" \
    OSSCTL_STUB_COMMIT="${OSSCTL_STUB_COMMIT:-a35b9917fc65a6354fe855b7c956521b47669907}" \
    GH_STUB_REPO="$gh_repo" \
    "$repo_root/scripts/ossctl-release.sh" resume test-run \
    >"$tmp/stdout" 2>"$tmp/stderr"
}

reset_logs() {
  : >"$tmp/gh.log"
  : >"$tmp/git.log"
  : >"$tmp/ossctl.log"
}

assert_supported_gh_call() {
  local expected="repo view jarimustonen/orchestratectl --json nameWithOwner -q .nameWithOwner"
  local actual
  actual="$(cat "$tmp/gh.log")"
  [[ "$actual" == "$expected" ]] || {
    printf 'unexpected gh invocation\nexpected: %s\nactual:   %s\n' "$expected" "$actual" >&2
    exit 1
  }
}

assert_git_calls() {
  local expected=$'rev-parse --show-toplevel\nremote get-url origin\nremote get-url --push --all origin'
  local actual
  actual="$(cat "$tmp/git.log")"
  [[ "$actual" == "$expected" ]] || {
    printf 'unexpected git preflight surface\nexpected:\n%s\nactual:\n%s\n' "$expected" "$actual" >&2
    exit 1
  }
}

assert_no_release_show() {
  if grep -F 'release show' "$tmp/ossctl.log" >/dev/null; then
    echo "repository mismatch reached ossctl release show" >&2
    cat "$tmp/ossctl.log" >&2
    exit 1
  fi
}

reset_logs
set +e
run_wrapper
status=$?
set -e
[[ "$status" -eq 42 ]] || {
  echo "supported gh repo view invocation did not pass preflight (status=$status)" >&2
  cat "$tmp/stderr" >&2
  exit 1
}
assert_supported_gh_call
assert_git_calls
grep -Fx 'release show test-run --json' "$tmp/ossctl.log" >/dev/null || {
  echo "successful repository preflight did not reach release show sentinel" >&2
  cat "$tmp/ossctl.log" >&2
  exit 1
}

reset_logs
set +e
run_wrapper unrelated-owner/unrelated-repo
status=$?
set -e
[[ "$status" -eq 1 ]] || {
  echo "GitHub repository mismatch did not fail closed (status=$status)" >&2
  cat "$tmp/stderr" >&2
  exit 1
}
assert_supported_gh_call
assert_git_calls
grep -Fx 'release repository mismatch: origin=jarimustonen/orchestratectl push=jarimustonen/orchestratectl gh=unrelated-owner/unrelated-repo expected=jarimustonen/orchestratectl' "$tmp/stderr" >/dev/null || {
  echo "GitHub repository mismatch did not emit the expected diagnostic" >&2
  cat "$tmp/stderr" >&2
  exit 1
}
assert_no_release_show

reset_logs
set +e
run_wrapper jarimustonen/orchestratectl https://github.com/unrelated-owner/unrelated-repo.git
status=$?
set -e
[[ "$status" -eq 1 ]] || {
  echo "origin repository mismatch did not fail closed (status=$status)" >&2
  cat "$tmp/stderr" >&2
  exit 1
}
assert_supported_gh_call
assert_git_calls
grep -Fx 'release repository mismatch: origin=unrelated-owner/unrelated-repo push=unrelated-owner/unrelated-repo gh=jarimustonen/orchestratectl expected=jarimustonen/orchestratectl' "$tmp/stderr" >/dev/null || {
  echo "origin repository mismatch did not emit the expected diagnostic" >&2
  cat "$tmp/stderr" >&2
  exit 1
}
assert_no_release_show

for unsupported in 0.10.1 0.11.0 1.0.0; do
  reset_logs
  set +e
  OSSCTL_STUB_VERSION="$unsupported" run_wrapper
  status=$?
  set -e
  [[ "$status" -eq 1 ]] || {
    echo "unsupported ossctl $unsupported did not fail closed (status=$status)" >&2
    exit 1
  }
  grep -F "ossctl 0.9.0 or validated 0.10.0 required; found $unsupported" "$tmp/stderr" >/dev/null || {
    echo "unsupported ossctl $unsupported emitted the wrong diagnostic" >&2
    cat "$tmp/stderr" >&2
    exit 1
  }
  test ! -s "$tmp/gh.log" || { echo "unsupported ossctl $unsupported reached repository preflight" >&2; exit 1; }
done

reset_logs
set +e
OSSCTL_STUB_COMMIT=0000000000000000000000000000000000000000 run_wrapper
status=$?
set -e
[[ "$status" -eq 1 ]] || { echo "unvalidated ossctl 0.10.0 commit did not fail closed" >&2; exit 1; }
grep -F "ossctl 0.10.0 protocol was validated at commit" "$tmp/stderr" >/dev/null

reset_logs
set +e
run_wrapper jarimustonen/orchestratectl
status=$?
set -e
[[ "$status" -eq 42 ]] || { echo "valid version did not recover after version rejection tests" >&2; exit 1; }

for abandoned in 01M0FD8FSTMGYG8YTV92WMWC87 01M0FG88NAKBJ7Y3QNFZEHRM4K; do
  reset_logs
  set +e
  env -i HOME="$tmp/home" PATH="$tmp/bin" GH_STUB_LOG="$tmp/gh.log" GIT_STUB_LOG="$tmp/git.log" \
    GIT_STUB_ROOT="$tmp/work" GIT_STUB_ORIGIN=git@github.com:jarimustonen/orchestratectl.git \
    OSSCTL_STUB_LOG="$tmp/ossctl.log" OSSCTL_STUB_VERSION=0.10.0 \
    OSSCTL_STUB_COMMIT=a35b9917fc65a6354fe855b7c956521b47669907 \
    GH_STUB_REPO=jarimustonen/orchestratectl "$repo_root/scripts/ossctl-release.sh" resume "$abandoned" \
    >"$tmp/stdout" 2>"$tmp/stderr"
  status=$?
  set -e
  [[ "$status" -eq 2 ]] || { echo "abandoned run $abandoned was not blocked (status=$status)" >&2; exit 1; }
  grep -F "release run $abandoned is permanently abandoned and must never be resumed" "$tmp/stderr" >/dev/null
  ! grep -F "release show" "$tmp/ossctl.log" >/dev/null || { echo "abandoned run $abandoned reached release show" >&2; exit 1; }
done

set +e
env -i PATH="$tmp/bin" GH_STUB_LOG="$tmp/gh.log" GH_STUB_REPO=unused \
  "$tmp/bin/gh" repo view -R jarimustonen/orchestratectl \
  --json nameWithOwner -q .nameWithOwner >/dev/null 2>&1
status=$?
set -e
[[ "$status" -eq 97 ]] || {
  echo "gh fixture did not explicitly reject the old repo view -R form (status=$status)" >&2
  exit 1
}

"$repo_root/scripts/test-ossctl-release-held-tag.sh"
echo "release wrapper tests passed"
