#!/usr/bin/env python3
"""Collect and validate the immutable public state required by ADR 0002 R11."""
from __future__ import annotations

import base64
import datetime as dt
import hashlib
import json
import os
import re
import sys
import urllib.request

UA = "taskfleet-r11-evidence/1.0 (https://github.com/jarimustonen/taskfleet)"
OLD = "jarimustonen/homebrew-orchestratectl"
NEW = "jarimustonen/homebrew-taskfleet"
OLD_ID = 1322902240
NEW_ID = 1355125556
OLD_HEAD = "20a70f463e699af5ddba6f6455c20a183c496ca5"
OLD_PARENT = "85ce830378f38cf17283efddd966d5754354e403"
OLD_TREE = "059ef99bd96fd0a89bb0e687c53dba2fe6d7a652"
NEW_HEAD = "c9e68594340b2b775d23159a3545d53f15306471"
NEW_TREE = "161020025cdc8f5e1f0a6a50ee00e0f0a9359b8c"
FORMULA_SHA = "44ab6275a66d85c5ab971ad3cc52e8b19451853bae2b00b4316a9fc71e7a9575"
MIGRATION_SHA = "472e25498a09e076c7f835a2f30aba355e75c7e7d9dc60828ca8e3347717874a"
VERSION = "0.6.1"
COMMIT = "7e93bd6195fbaf6de0b43d9161228ae2373ab5d1"
EXPECTED_URLS = {
    "aarch64-apple-darwin": ("https://github.com/jarimustonen/taskfleet/releases/download/v0.6.1/taskfleet-aarch64-apple-darwin.tar.xz", "4bbf3b023ae0377e8cdca41e07854cbb64165eba82ddc5ae70e1ba90386406a6"),
    "aarch64-unknown-linux-gnu": ("https://github.com/jarimustonen/taskfleet/releases/download/v0.6.1/taskfleet-aarch64-unknown-linux-gnu.tar.xz", "3aedcaec35b2ddc789bcee8d1f934e0641dba508edb6e08e09e8d7e63b5359a3"),
    "x86_64-unknown-linux-gnu": ("https://github.com/jarimustonen/taskfleet/releases/download/v0.6.1/taskfleet-x86_64-unknown-linux-gnu.tar.xz", "6e1645f0739b1fa528e0313a18a23113d7c146a2bbb23c72aed817e5950d4c71"),
}


def api(path: str) -> object:
    headers = {"Accept": "application/vnd.github+json", "User-Agent": UA, "X-GitHub-Api-Version": "2022-11-28"}
    token = os.environ.get("GH_TOKEN") or os.environ.get("GITHUB_TOKEN")
    if token:
        headers["Authorization"] = f"Bearer {token}"
    req = urllib.request.Request(f"https://api.github.com/{path}", headers=headers)
    with urllib.request.urlopen(req, timeout=60) as response:
        return json.load(response)


def content(repo: str, path: str, ref: str) -> bytes:
    data = api(f"repos/{repo}/contents/{path}?ref={ref}")
    assert isinstance(data, dict) and data.get("encoding") == "base64"
    return base64.b64decode(data["content"])


def repo_state(repo: str, expected_id: int, expected_head: str, expected_tree: str) -> tuple[dict, dict]:
    metadata = api(f"repos/{repo}")
    ref = api(f"repos/{repo}/git/ref/heads/main")
    commit = api(f"repos/{repo}/git/commits/{expected_head}")
    tree = api(f"repos/{repo}/git/trees/{expected_tree}?recursive=1")
    assert isinstance(metadata, dict) and metadata["id"] == expected_id and metadata["full_name"] == repo
    assert isinstance(ref, dict) and ref["object"]["sha"] == expected_head
    assert isinstance(commit, dict) and commit["tree"]["sha"] == expected_tree
    assert isinstance(tree, dict) and tree["sha"] == expected_tree and not tree.get("truncated")
    inventory = [{k: entry.get(k) for k in ("path", "type", "sha", "size") if entry.get(k) is not None} for entry in tree["tree"]]
    return ({"repository": repo, "repository_id": expected_id, "default_branch": metadata["default_branch"], "head": expected_head, "tree": expected_tree, "inventory": inventory}, commit)


def main() -> int:
    if len(sys.argv) != 2:
        print(f"usage: {sys.argv[0]} <output-json>", file=sys.stderr)
        return 2
    old, old_commit = repo_state(OLD, OLD_ID, OLD_HEAD, OLD_TREE)
    new, _ = repo_state(NEW, NEW_ID, NEW_HEAD, NEW_TREE)
    assert [x["path"] for x in old["inventory"]] == ["README.md", "tap_migrations.json"]
    assert [x["path"] for x in new["inventory"]] == ["Formula", "Formula/taskfleet.rb"]
    assert len(old_commit["parents"]) == 1 and old_commit["parents"][0]["sha"] == OLD_PARENT

    migration_raw = content(OLD, "tap_migrations.json", OLD_HEAD)
    formula_raw = content(NEW, "Formula/taskfleet.rb", NEW_HEAD)
    migration = json.loads(migration_raw)
    formula = formula_raw.decode()
    assert hashlib.sha256(migration_raw).hexdigest() == MIGRATION_SHA
    assert migration == {"orchestratectl": "jarimustonen/taskfleet/taskfleet"}
    assert hashlib.sha256(formula_raw).hexdigest() == FORMULA_SHA
    assert re.search(r'^class Taskfleet < Formula$', formula, re.M)
    assert re.search(r'^  version "0\.6\.1"$', formula, re.M)
    assert 'bin.install "taskfleet"' in formula and 'bin.install "orchestratectl"' not in formula
    assert "orchestratectl" not in formula
    pairs = re.findall(r'url "([^"]+)"\s+sha256 "([0-9a-f]{64})"', formula)
    actual_urls = {}
    for url, checksum in pairs:
        target = re.search(r"taskfleet-(.+)\.tar\.xz$", url)
        assert target
        actual_urls[target.group(1)] = (url, checksum)
    assert actual_urls == EXPECTED_URLS
    formula_paths = [x["path"] for x in old["inventory"] + new["inventory"] if x["path"].startswith("Formula/")]
    assert formula_paths == ["Formula/taskfleet.rb"]

    result = {
        "schema_version": 1,
        "collected_at": dt.datetime.now(dt.timezone.utc).isoformat().replace("+00:00", "Z"),
        "user_agent": UA,
        "overall": "pass",
        "old_tap": {**old, "parent": OLD_PARENT, "migration_sha256": MIGRATION_SHA, "migration": migration},
        "canonical_tap": {**new, "formula_sha256": FORMULA_SHA, "formula_version": VERSION, "formula_urls": {k: {"url": v[0], "sha256": v[1]} for k, v in sorted(EXPECTED_URLS.items())}},
        "ownership": {"formula_implementations": formula_paths, "count": 1, "canonical_identity": "jarimustonen/taskfleet/taskfleet", "orchestratectl_formula": "absent", "orchestratectl_binary_or_alias_in_formula": "absent"},
        "runtime_expected": {"version": VERSION, "embedded_commit": COMMIT},
    }
    with open(sys.argv[1], "w", encoding="utf-8") as out:
        json.dump(result, out, indent=2, sort_keys=True)
        out.write("\n")
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
