#!/usr/bin/env bash
# Offline verifier for the immutable ADR 0002 R11 evidence bundle.
set -euo pipefail
dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
python3 - "$dir" <<'PY'
import hashlib, json, pathlib, re, sys
root = pathlib.Path(sys.argv[1])
index = json.loads((root / "index.json").read_text())
assert index["schema_version"] == 1 and index["overall"] == "pass"
for artifact in index["artifacts"]:
    path = root / artifact["path"]
    data = path.read_bytes()
    assert len(data) == artifact["bytes"], artifact["path"]
    assert hashlib.sha256(data).hexdigest() == artifact["sha256"], artifact["path"]

public = json.loads((root / "public-state.json").read_text())
assert public["overall"] == "pass"
assert public["old_tap"]["repository_id"] == 1322902240
assert public["old_tap"]["head"] == "20a70f463e699af5ddba6f6455c20a183c496ca5"
assert public["old_tap"]["parent"] == "85ce830378f38cf17283efddd966d5754354e403"
assert public["old_tap"]["tree"] == "059ef99bd96fd0a89bb0e687c53dba2fe6d7a652"
assert [x["path"] for x in public["old_tap"]["inventory"]] == ["README.md", "tap_migrations.json"]
assert public["old_tap"]["migration"] == {"orchestratectl": "jarimustonen/taskfleet/taskfleet"}
assert public["canonical_tap"]["repository_id"] == 1355125556
assert public["canonical_tap"]["head"] == "c9e68594340b2b775d23159a3545d53f15306471"
assert public["canonical_tap"]["tree"] == "161020025cdc8f5e1f0a6a50ee00e0f0a9359b8c"
assert public["canonical_tap"]["formula_sha256"] == "44ab6275a66d85c5ab971ad3cc52e8b19451853bae2b00b4316a9fc71e7a9575"
assert public["canonical_tap"]["formula_version"] == "0.6.1"
assert public["ownership"] == {"canonical_identity":"jarimustonen/taskfleet/taskfleet", "count":1, "formula_implementations":["Formula/taskfleet.rb"], "orchestratectl_binary_or_alias_in_formula":"absent", "orchestratectl_formula":"absent"}

paths = json.loads((root / "homebrew-paths.json").read_text())
assert paths["overall"] == "pass"
assert paths["isolation"]["cleanup"] == "all-roots-removed"
expected = ("0.6.1", "7e93bd6195fbaf6de0b43d9161228ae2373ab5d1")
for key in ("fresh_canonical", "old_tap_qualified", "old_receipt_automatic_migration", "old_receipt_explicit_migration", "direct_after_migration"):
    item = paths[key]
    assert item["result"] == "pass", key
    assert (item["runtime"]["data"]["version"], item["runtime"]["data"]["commit"]) == expected, key
    receipt = item["receipt"]
    assert receipt["source"]["tap"] == "jarimustonen/taskfleet", key
    assert receipt["source"]["tap_git_head"] == "c9e68594340b2b775d23159a3545d53f15306471", key
    assert receipt["source"]["versions"]["stable"] == "0.6.1", key
    ownership = item["ownership"]
    assert ownership["physical_racks"] == ["taskfleet"] and ownership["physical_versions"] == ["0.6.1"], key
    assert ownership["formula_files"] == ["jarimustonen/homebrew-taskfleet/Formula/taskfleet.rb"], key
    assert ownership["installed_binaries"] == ["taskfleet"] and ownership["orchestratectl_binary_or_alias"] == "absent", key
assert paths["fresh_canonical"]["uninstall_residue"] == "absent"
assert paths["old_tap_qualified"]["uninstall_residue"] == "absent"
assert paths["direct_after_migration"]["final_uninstall_residue"] == "absent"
for key in ("old_receipt_automatic_migration", "old_receipt_explicit_migration"):
    assert paths[key]["baseline_version"] == "0.5.1"
    baseline = paths[key]["baseline_receipt"]
    assert baseline["source"]["tap"] == "jarimustonen/orchestratectl"
    assert baseline["source"]["tap_git_head"] == "85ce830378f38cf17283efddd966d5754354e403"
    assert baseline["source"]["versions"]["stable"] == "0.5.1"
    assert paths[key]["orchestratectl_binary_or_alias"] == "absent"
assert paths["old_receipt_automatic_migration"]["post_update_formulae"] == "orchestratectl\ntaskfleet"
assert paths["old_receipt_automatic_migration"]["post_upgrade_formulae"] == "orchestratectl\ntaskfleet"
assert paths["old_receipt_automatic_migration"]["ownership"]["brew_list_formula_projection"] == ["orchestratectl", "taskfleet"]
assert paths["explicit_migrate_after_automatic"]["semantics"] == "automatic update already consumed migration"
assert paths["old_receipt_explicit_migration"]["update_upgrade_before_trust"] == "preserved old keg"

for path in root.rglob("*"):
    if path.is_file():
        data = path.read_bytes()
        if path.suffix in {".json", ".md"}:
            assert b"/Users/" not in data and b"taskfleet-r11-homebrew." not in data
        assert not re.search(rb"(?:gh[pousr]_|github_pat_)[A-Za-z0-9_]+", data)
print("Taskfleet R11 offline evidence verification passed")
PY
