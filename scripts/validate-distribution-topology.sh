#!/usr/bin/env bash
# Validate cargo-dist's R7 plan and all checked-in distribution coordinates.
set -euo pipefail

[[ $# -ge 1 && $# -le 2 ]] || { echo "usage: $0 <cargo-dist-plan.json> [prepared|active]" >&2; exit 2; }
plan="$1"
state="${2:-active}"
[[ "$state" == prepared || "$state" == active ]] || { echo "state must be prepared or active" >&2; exit 2; }
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
cd "$repo_root"

jq -e '
  (.releases | length) == 1 and
  .releases[0].app_name == "taskfleet" and
  .releases[0].display_name == "taskfleet" and
  .releases[0].hosting.github.owner == "jarimustonen" and
  .releases[0].hosting.github.repo == "taskfleet" and
  (.artifacts | keys | sort) == ([
    "orchestratectl-installer.sh",
    "sha256.sum",
    "source.tar.gz",
    "source.tar.gz.sha256",
    "taskfleet-aarch64-apple-darwin.tar.xz",
    "taskfleet-aarch64-apple-darwin.tar.xz.sha256",
    "taskfleet-aarch64-unknown-linux-gnu.tar.xz",
    "taskfleet-aarch64-unknown-linux-gnu.tar.xz.sha256",
    "taskfleet-installer.sh",
    "taskfleet-x86_64-unknown-linux-gnu.tar.xz",
    "taskfleet-x86_64-unknown-linux-gnu.tar.xz.sha256",
    "taskfleet.rb"
  ] | sort) and
  .artifacts["orchestratectl-installer.sh"].kind == "extra-artifact" and
  .artifacts["taskfleet-installer.sh"].kind == "installer" and
  .artifacts["taskfleet.rb"].kind == "installer" and
  ([.artifacts | to_entries[] | select(.key | test("orchestratectl")) | .key]) == ["orchestratectl-installer.sh"] and
  ([.artifacts | to_entries[] | select(.value.kind == "executable-zip") |
    .value.assets[] | select(.kind == "executable") | .name] | unique) == ["taskfleet"] and
  ([.releases[].artifacts[] | select(endswith(".rb"))]) == ["taskfleet.rb"] and
  ([.ci.github.artifacts_matrix.include[] | select(.targets == ["aarch64-apple-darwin"]) | .runner]) == ["macOS"] and
  ([.ci.github.artifacts_matrix.include[] | select(.targets != ["aarch64-apple-darwin"]) | .runner] | unique) == ["ubuntu-22.04"] and
  ([.ci.github.artifacts_matrix.include[] | .container // null] | all(. == null))
' "$plan" >/dev/null || { echo "cargo-dist plan is not the admitted Taskfleet-only artifact graph" >&2; exit 2; }

jq -e --arg state "$state" '
  .schema_version == 1 and
  (if $state == "active" then .activation == "ready" else .activation == "prepared-blocked-r10" end) and
  .cargo_dist == {
    version:"0.28.2",config:"dist-workspace.toml",workflow:".github/workflows/release.yml",
    trigger:"tag-push",pr_run_mode:"skip",apps:["taskfleet"],
    tap:"jarimustonen/homebrew-taskfleet",tap_secret:"HOMEBREW_TAP_TOKEN",
    tap_secret_state:(if $state == "active" then "active-proven-r10" else "pending-r10-proof" end),
    activation_gate:"scripts/verify-release-tag-authorization.sh",
    authorization:"wrapper-ref-exact-tag-main-green-ci",
    release_tag_ruleset:22234415,authorization_ref_ruleset:22234417,macos_runner:"macOS",
    stub_artifact:"orchestratectl-installer.sh",
    stub_sha256:"6d171a7e0e4be8dec9518d6a888ea73400c0ccebf0a0d2f68b0f41cf5414653b"
  } and
  .source_repository == {current:"jarimustonen/taskfleet",after_r9:"jarimustonen/taskfleet"} and
  .old_tap.repository == "jarimustonen/homebrew-orchestratectl" and
  .old_tap.activation == "blocked-r11" and
  .public_receipts.repository_id == 1355125556 and
  .public_receipts.proof_commit == "db12bb163e47617f0b941a35d3896b6ba0548892" and
  .public_receipts.proof_tree == "4b825dc642cb6eb9a060e54bf8d69288fbee4904"
' release/taskfleet-distribution.json >/dev/null || { echo "distribution authority is invalid" >&2; exit 2; }

expected_release_activation=ready
[[ "$state" == prepared ]] && expected_release_activation=blocked-r8-r9-r10
[[ "$(jq -r .activation release/taskfleet-release.json)" == "$expected_release_activation" ]] || {
  echo "release topology does not match requested $state state" >&2; exit 2;
}
[[ "$(grep -Fc 'repository: "jarimustonen/homebrew-taskfleet"' .github/workflows/release.yml)" == 1 ]] || {
  echo "generated workflow must contain exactly one canonical tap checkout" >&2; exit 2;
}
if grep -F 'repository: "jarimustonen/homebrew-orchestratectl"' .github/workflows/release.yml >/dev/null; then
  echo "generated workflow still targets the old tap" >&2; exit 2
fi
grep -A12 '^on:' .github/workflows/release.yml | grep -Eq '^[[:space:]]+push:' || {
  echo "R9 cargo-dist workflow lacks the canonical tag trigger" >&2; exit 2;
}
grep -A12 '^on:' .github/workflows/release.yml |
  grep -F -- "- '**[0-9]+.[0-9]+.[0-9]+*'" >/dev/null || {
  echo "generated workflow lacks the exact cargo-dist version-tag pattern" >&2; exit 2;
}
if grep -A12 '^on:' .github/workflows/release.yml | grep -Eq 'workflow_dispatch:|pull_request:'; then
  echo "cargo-dist release workflow must be tag-only" >&2; exit 2
fi
grep -F 'pr-run-mode = "skip"' dist-workspace.toml >/dev/null || {
  echo "cargo-dist must omit release workflow execution on pull requests" >&2; exit 2;
}
if grep -Eq 'custom-taskfleet-release-gate|secrets: inherit' .github/workflows/release.yml; then
  echo "generated workflow must not call a secret-inheriting reusable gate" >&2; exit 2
fi
grep -F './scripts/verify-release-tag-authorization.sh' .github/workflows/release.yml >/dev/null || {
  echo "generated artifact builds do not enforce wrapper authorization" >&2; exit 2;
}
grep -F './scripts/verify-release-tag-authorization.sh' .github/workflows/publish-crates.yml >/dev/null || {
  echo "crates.io release leg does not enforce wrapper authorization" >&2; exit 2;
}
./scripts/test-release-authorization.sh >/dev/null || {
  echo "structural release authorization fixtures failed" >&2; exit 2;
}
[[ "$(grep -Fc 'token: ${{ secrets.HOMEBREW_TAP_TOKEN }}' .github/workflows/release.yml)" == 1 ]] || {
  echo "generated workflow must use the one admitted tap secret" >&2; exit 2;
}
[[ "$(grep -Fc 'runs-on: ${{ matrix.runner }}' .github/workflows/release.yml)" == 1 ]] || {
  echo "generated workflow lost the cargo-dist runner matrix" >&2; exit 2;
}
grep -F 'cargo-dist/releases/download/v0.28.2/cargo-dist-installer.sh' .github/workflows/release.yml >/dev/null || {
  echo "generated workflow does not pin cargo-dist 0.28.2" >&2; exit 2;
}

grep -F 'build = ["./scripts/build-legacy-installer-stub.sh"]' dist-workspace.toml >/dev/null || {
  echo "cargo-dist stub producer is not the admitted inert builder" >&2; exit 2;
}
grep -F 'artifacts = ["target/taskfleet-extra/orchestratectl-installer.sh"]' dist-workspace.toml >/dev/null || {
  echo "cargo-dist stub output path drifted" >&2; exit 2;
}

jq -e '.package.metadata.dist.dist == false' < <(cargo metadata --locked --no-deps --format-version=1 |
  jq '.packages[] | select(.name == "orchestratectl") | {package:{metadata:.metadata}}') >/dev/null
jq -e '.package.metadata.dist.dist == true' < <(cargo metadata --locked --no-deps --format-version=1 |
  jq '.packages[] | select(.name == "taskfleet") | {package:{metadata:.metadata}}') >/dev/null

migration=issues/taskfleet-distribution-topology/old-tap-migration/manifest.json
jq -e '
  .required_head == "85ce830378f38cf17283efddd966d5754354e403" and
  .required_formula_blob == "c7d02e0e61f16e347f01bed09473fa7b86b5034f" and
  .prepared_tree == "059ef99bd96fd0a89bb0e687c53dba2fe6d7a652" and
  .deletes == ["Formula/orchestratectl.rb"] and .adds == ["tap_migrations.json"] and
  .push_authorized == false
' "$migration" >/dev/null
jq -e '.orchestratectl == "jarimustonen/taskfleet/taskfleet" and length == 1' \
  issues/taskfleet-distribution-topology/old-tap-migration/tap_migrations.json >/dev/null

jq -e '.full_name == "jarimustonen/homebrew-taskfleet" and .private == false and
  .owner.login == "jarimustonen" and .id == 1355125556 and .default_branch == "main"' \
  issues/taskfleet-distribution-topology/receipts/repository.json >/dev/null
jq -e '.sha == "db12bb163e47617f0b941a35d3896b6ba0548892" and
  .tree.sha == "4b825dc642cb6eb9a060e54bf8d69288fbee4904" and (.parents | length) == 0' \
  issues/taskfleet-distribution-topology/receipts/proof-commit.json >/dev/null
jq -e 'type == "array" and length == 0' \
  issues/taskfleet-distribution-topology/receipts/final-contents.json >/dev/null
jq -e '.name == "HOMEBREW_TAP_TOKEN" and .state == "deliberately-inert-after-r7-proof"' \
  issues/taskfleet-distribution-topology/receipts/final-secret-state-attestation.json >/dev/null
jq -e '.cargo_dist == "0.28.2" and .credentials == {GH_TOKEN:false,GITHUB_TOKEN:false} and
  .public_release_list_sha256_before == .public_release_list_sha256_after and
  .public_tag_refs_sha256_before == .public_tag_refs_sha256_after and
  .outcome == "local manifest only; no public mutation"' \
  issues/taskfleet-distribution-topology/receipts/host-create-no-mutation.json >/dev/null
jq -e '.cargo_dist == "0.28.2" and .native_target == "aarch64-apple-darwin" and
  .artifacts.stub_sha256 == "6d171a7e0e4be8dec9518d6a888ea73400c0ccebf0a0d2f68b0f41cf5414653b" and
  .formula_installs == ["taskfleet"] and .formula_aliases == [] and
  .shell_installs == ["taskfleet"] and .outcome == "passed"' \
  issues/taskfleet-distribution-topology/receipts/native-artifact-build.json >/dev/null

printf 'Taskfleet distribution topology validated\n'
