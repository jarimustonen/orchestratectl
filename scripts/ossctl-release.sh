#!/usr/bin/env bash
# Safe wrapper around ossctl 0.9's resumable release engine.
# It pauses a bump cut at tag push, advances main to the bump commit, waits for
# CI on that exact SHA, then resumes the journalled cut.
set -euo pipefail

readonly expected_repo="jarimustonen/orchestratectl"
run_id=""
tag=""
bump_commit=""

usage() {
  cat >&2 <<'EOF'
usage:
  scripts/ossctl-release.sh plan <major|minor|patch>
  scripts/ossctl-release.sh cut <plan-id>
  scripts/ossctl-release.sh resume <run-id>
  scripts/ossctl-release.sh verify <run-id>
EOF
  exit 1
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || { echo "required command not found: $1" >&2; exit 1; }
}

require_ossctl_0_9() {
  local version
  version="$(ossctl version --json | jq -er '.data.version')"
  jq -en --arg version "$version" '
    ($version | capture("^(?<major>[0-9]+)\\.(?<minor>[0-9]+)\\.(?<patch>[0-9]+)$")) as $v |
    ($v.major | tonumber) == 0 and ($v.minor | tonumber) == 9
  ' >/dev/null || {
    echo "ossctl 0.9.x required; found $version (revalidate the pre-tag protocol before widening this range)" >&2
    exit 1
  }
}

validate_level() {
  case "$1" in major|minor|patch) ;; *) usage ;; esac
}

canonical_github_repo() {
  sed -E 's#^(git@github.com:|https://github.com/|ssh://git@github.com/)##; s#\.git$##' <<<"$1"
}

assert_repo_identity() {
  local origin_repo gh_repo
  origin_repo="$(canonical_github_repo "$(git remote get-url origin)")"
  gh_repo="$(gh repo view "$expected_repo" --json nameWithOwner -q .nameWithOwner)"
  [[ "$origin_repo" == "$expected_repo" && "$gh_repo" == "$expected_repo" ]] || {
    echo "release repository mismatch: origin=$origin_repo gh=$gh_repo expected=$expected_repo" >&2
    exit 1
  }
}

show_run() {
  ossctl release show "$1" --json
}

read_run_coordinates() {
  local show_json="$1"
  bump_commit="$(jq -er '.data.state.bump.commit' <<<"$show_json")"
  tag="$(jq -er '
    .data.state.tags | keys |
    if length == 1 then .[0] else error("expected exactly one release tag") end
  ' <<<"$show_json")"
  [[ "$bump_commit" =~ ^[0-9a-f]{40}$ ]] || {
    echo "journal contains an invalid bump commit: $bump_commit" >&2
    exit 2
  }
  [[ "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+([+-][0-9A-Za-z.-]+)?$ ]] || {
    echo "journal contains an invalid release tag: $tag" >&2
    exit 2
  }
  git cat-file -e "$bump_commit^{commit}" 2>/dev/null || {
    echo "journalled bump commit is unavailable locally: $bump_commit" >&2
    exit 2
  }
  [[ "$(git rev-parse "$tag^{commit}")" == "$bump_commit" ]] || {
    echo "local release tag $tag does not point at journalled bump commit $bump_commit" >&2
    exit 2
  }
}

validate_bump_tree() {
  local version="${tag#v}" workspace_version core_pin
  workspace_version="$(awk -F'"' '
    /^\[workspace\.package\]/ { in_package=1; next }
    /^\[/ { in_package=0 }
    in_package && /^version[[:space:]]*=/ { print $2; exit }
  ' Cargo.toml)"
  core_pin="$(sed -nE 's/^octl-core = \{[^}]*version = "=([^"]+)".*/\1/p' crates/octl-cli/Cargo.toml)"
  [[ "$workspace_version" == "$version" && "$core_pin" == "$version" ]] || {
    echo "bump tree mismatch: tag=$version workspace=$workspace_version octl-core-pin=$core_pin" >&2
    exit 2
  }
  grep -F "## [$version] - " CHANGELOG.md | grep -Eq '^## \[[0-9]+\.[0-9]+\.[0-9]+\] - [0-9]{4}-[0-9]{2}-[0-9]{2}$' || {
    echo "CHANGELOG is not finalized with a dated section for $version" >&2
    exit 2
  }
  awk -v version="$version" '
    /^name = "octl-core"$/ || /^name = "orchestratectl"$/ { wanted=1; next }
    wanted && /^version = / {
      seen++
      if ($0 != "version = \"" version "\"") bad=1
      wanted=0
    }
    END { exit !(seen == 2 && !bad) }
  ' Cargo.lock || {
    echo "Cargo.lock does not carry both workspace packages at $version" >&2
    exit 2
  }
  ./scripts/check-version-snapshots.sh
  test -z "$(git status --porcelain)" || { echo "bump tree must be clean" >&2; exit 2; }
}

advance_main_to_bump() {
  local base origin_main
  base="$(git rev-parse HEAD)"
  git merge-base --is-ancestor "$base" "$bump_commit" || {
    echo "bump commit is not a descendant of local main" >&2
    exit 2
  }
  git fetch origin +refs/heads/main:refs/remotes/origin/main
  origin_main="$(git rev-parse origin/main)"
  [[ "$origin_main" == "$base" || "$origin_main" == "$bump_commit" ]] || {
    echo "origin/main moved to $origin_main during the release; reconcile deliberately" >&2
    exit 1
  }
  if [[ "$base" != "$bump_commit" ]]; then
    git merge --ff-only "$bump_commit"
  fi
  validate_bump_tree
  if [[ "$origin_main" != "$bump_commit" ]]; then
    # Explicitly disable follow-tags: the held local release tag must not ride the
    # branch push before CI. A normal non-force push rejects any concurrent move.
    git -c push.followTags=false push origin HEAD:refs/heads/main
  fi
}

wait_for_exact_main_ci() {
  local sha="$1" id="" run_json
  for ((attempt = 0; attempt < 60; attempt++)); do
    id="$(gh run list -R "$expected_repo" --workflow ci.yml --branch main --commit "$sha" --event push --limit 1 --json databaseId -q '.[0].databaseId')"
    test -n "$id" && test "$id" != null && break
    sleep 5
  done
  test -n "$id" && test "$id" != null || { echo "no main CI run for $sha" >&2; exit 1; }
  run_json="$(gh run view -R "$expected_repo" "$id" --json headSha,headBranch,event)"
  jq -e --arg sha "$sha" '
    .headSha == $sha and .headBranch == "main" and .event == "push"
  ' <<<"$run_json" >/dev/null || {
    echo "GitHub run $id does not attest exact main SHA $sha" >&2
    exit 2
  }
  if ! gh run watch -R "$expected_repo" "$id" --exit-status; then
    echo "main CI failed for $sha; release $run_id remains untagged remotely" >&2
    exit 1
  fi
}

remote_tag_commit() {
  git ls-remote --tags origin "refs/tags/$tag" "refs/tags/$tag^{}" |
    awk '{ if (substr($2, length($2)-2) == "^{}") peeled=$1; else direct=$1 }
         END { print (peeled != "" ? peeled : direct) }'
}

assert_remote_tag_absent() {
  local remote_tag
  remote_tag="$(remote_tag_commit)"
  test -z "$remote_tag" || {
    echo "remote tag $tag already exists at $remote_tag; the pre-tag gate can no longer be established" >&2
    echo "publishing may be underway; inspect run $run_id and do not retag or publish manually" >&2
    exit 2
  }
}

resume_after_gate() {
  local show_json remote_tag pushed_remote
  show_json="$(show_run "$run_id")"
  read_run_coordinates "$show_json"
  pushed_remote="$(jq -er --arg tag "$tag" '.data.state.tags[$tag].pushed_remote' <<<"$show_json")"

  if [[ "$pushed_remote" == false ]]; then
    jq -e --arg tag "$tag" '
      .data.state.current_phase == "tag" and .data.state.tags[$tag].created_local == true
    ' <<<"$show_json" >/dev/null || {
      echo "run $run_id is not at the held pre-tag checkpoint" >&2
      exit 2
    }
    assert_remote_tag_absent
    advance_main_to_bump
    git fetch origin +refs/heads/main:refs/remotes/origin/main
    [[ "$(git rev-parse HEAD)" == "$bump_commit" && "$(git rev-parse origin/main)" == "$bump_commit" ]] || {
      echo "local and remote main must both equal journalled bump commit $bump_commit" >&2
      exit 1
    }
    validate_bump_tree
    wait_for_exact_main_ci "$bump_commit"
    assert_remote_tag_absent
    ossctl release resume "$run_id" --json
  else
    remote_tag="$(remote_tag_commit)"
    [[ "$remote_tag" == "$bump_commit" ]] || {
      echo "remote tag $tag points at ${remote_tag:-<missing>}, expected $bump_commit" >&2
      exit 2
    }
    # The irreversible boundary is already crossed. Continue only this journal;
    # never create or push a replacement tag.
    ossctl release resume "$run_id" --json
  fi

  remote_tag="$(remote_tag_commit)"
  [[ "$remote_tag" == "$bump_commit" ]] || {
    echo "remote tag $tag points at ${remote_tag:-<missing>}, expected CI-validated $bump_commit" >&2
    exit 2
  }
  ossctl release verify "$run_id" --json
}

require_command git
repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"
require_command ossctl
require_command jq
require_ossctl_0_9

command="${1:-}"
case "$command" in
  plan)
    [[ $# -eq 2 ]] || usage
    validate_level "$2"
    ossctl release list --json | jq -e '
      .data.in_flight_count == 0 and (.data.unreadable | length) == 0
    ' >/dev/null || { echo "an active or unreadable release run must be reconciled before planning" >&2; exit 1; }
    ossctl contract show --json --require-approved >/dev/null
    ossctl contract validate --json >/dev/null
    ossctl audit --json | jq -e '[.data.gaps[] | select(.severity == "blocking")] | length == 0' >/dev/null || {
      echo "blocking OSS readiness gaps prevent a release" >&2
      exit 1
    }
    test -z "$(git status --porcelain)" || { echo "working tree must be clean" >&2; exit 1; }
    exec ossctl release plan --bump "$2" --json
    ;;

  verify)
    [[ $# -eq 2 ]] || usage
    exec ossctl release verify "$2" --json
    ;;

  resume)
    [[ $# -eq 2 ]] || usage
    require_command gh
    assert_repo_identity
    run_id="$2"
    resume_after_gate
    ;;

  cut)
    [[ $# -eq 2 ]] || usage
    plan_id="$2"
    require_command gh
    assert_repo_identity

    test -n "$plan_id" || usage
    test -z "$(git status --porcelain)" || { echo "working tree must be clean" >&2; exit 1; }
    branch="$(git symbolic-ref --quiet --short HEAD)" || { echo "release cut must run on a branch" >&2; exit 1; }
    [[ "$branch" == main ]] || { echo "release cut must run on main (found $branch)" >&2; exit 1; }
    [[ "$(git config --bool --get push.followTags || echo false)" != true ]] || {
      echo "push.followTags=true can leak the held release tag; unset it before cutting" >&2
      exit 1
    }

    git fetch origin +refs/heads/main:refs/remotes/origin/main
    [[ "$(git rev-parse HEAD)" == "$(git rev-parse origin/main)" ]] || {
      echo "main must exactly match origin/main before the cut" >&2
      exit 1
    }
    base_commit="$(git rev-parse HEAD)"

    list_json="$(ossctl release list --json)"
    jq -e '.data.in_flight_count == 0 and (.data.unreadable | length) == 0' <<<"$list_json" >/dev/null || {
      echo "an active or unreadable release run must be reconciled before cutting" >&2
      exit 1
    }

    git_common="$(cd "$(git rev-parse --git-common-dir)" && pwd -P)"
    hooks="$(mktemp -d "$git_common/ossctl-pretag.XXXXXX")"
    marker="$hooks/tag-push-blocked"
    probe_tag="v0.0.0-ossctl-pretag-probe"
    cleanup() {
      git tag -d "$probe_tag" >/dev/null 2>&1 || true
      rm -rf "$hooks"
    }
    trap cleanup EXIT
    cat >"$hooks/pre-push" <<'HOOK'
#!/usr/bin/env bash
set -euo pipefail
while read -r local_ref local_oid remote_ref remote_oid; do
  if [[ "$local_ref" == refs/tags/v* || "$remote_ref" == refs/tags/v* ]]; then
    printf '%s\n' "$1" "$2" "$local_ref" "$local_oid" "$remote_ref" "$remote_oid" >"$OSSCTL_PRETAG_MARKER"
    echo "release tag held locally until main CI is green on its exact commit" >&2
    exit 75
  fi
done
exit 0
HOOK
    chmod +x "$hooks/pre-push"

    # Prove Git resolves the absolute hooksPath on a real (local-only) tag push.
    git init --quiet --bare "$hooks/probe.git"
    git tag "$probe_tag" HEAD
    if OSSCTL_PRETAG_MARKER="$marker" \
      GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=core.hooksPath GIT_CONFIG_VALUE_0="$hooks" \
      git push "$hooks/probe.git" "refs/tags/$probe_tag" >/dev/null 2>&1; then
      echo "pre-push safety hook did not reject a version tag" >&2
      exit 2
    fi
    test -s "$marker" || { echo "pre-push safety hook did not run on a real Git push" >&2; exit 2; }
    rm -f "$marker"
    git tag -d "$probe_tag" >/dev/null
    rm -rf "$hooks/probe.git"

    if OSSCTL_PRETAG_MARKER="$marker" \
      GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=core.hooksPath GIT_CONFIG_VALUE_0="$hooks" \
      ossctl release cut --plan "$plan_id" --json; then
      echo "safety stop failed: release cut passed the tag boundary before exact-SHA CI" >&2
      exit 2
    fi
    test -s "$marker" || {
      echo "cut failed before the pre-tag checkpoint; inspect ossctl output and release list" >&2
      exit 1
    }

    list_json="$(ossctl release list --json)"
    run_id="$(jq -er --arg plan "$plan_id" '
      [.data.runs[] | select(.plan_id == $plan and .in_flight)] |
      if length == 1 then .[0].run_id else error("expected one in-flight run for plan") end
    ' <<<"$list_json")"
    show_json="$(show_run "$run_id")"
    read_run_coordinates "$show_json"
    held_remote="$(canonical_github_repo "$(sed -n '2p' "$marker")")"
    [[ "$held_remote" == "$expected_repo" ]] || {
      echo "ossctl attempted to push the release tag to $held_remote, expected $expected_repo" >&2
      exit 2
    }
    assert_remote_tag_absent
    jq -e --arg tag "$tag" '
      .data.state.current_phase == "tag" and
      .data.state.tags[$tag].created_local == true and
      .data.state.tags[$tag].pushed_remote == false
    ' <<<"$show_json" >/dev/null || {
      echo "cut did not stop at the expected pre-push checkpoint; inspect run $run_id" >&2
      exit 2
    }
    git merge-base --is-ancestor "$base_commit" "$bump_commit" || {
      echo "journalled bump commit is not descended from sealed main" >&2
      exit 2
    }

    advance_main_to_bump
    resume_after_gate
    ;;

  *) usage ;;
esac
