#!/usr/bin/env python3
"""Write or verify R8's fail-closed evidence and harness index."""
import hashlib
import json
import sys
from datetime import datetime, timezone
from pathlib import Path

TESTED_COMMIT = "c3ef8b740ac531f12ce81c759ed209d178cf36bd"
TESTED_TREE = "b7d07d9df3308fb33afdfab892f949f46ef810d4"
REQUIRED_IDS = {
    "ci-api", "rust-fmt", "rust-clippy", "rust-nextest", "rust-doctest", "rustdoc",
    "insta", "stripped-path", "wrapper-parity", "legacy-baseline", "legacy-current",
    "state-config-migration", "registry-protocol", "shipshape-protocol",
    "release-activation", "package", "install-channels", "cargo-dist-homebrew-resolution",
    "legacy-installer-stub", "homebrew-old-receipt", "shipshape-contract", "shipshape-plan", "public-facts",
    "identity-ledger", "issue-gates", "diff-residue", "evidence-review",
}
REQUIRED_RESULTS = {command_id: {"pass"} for command_id in REQUIRED_IDS}
# Freeze the outcomes actually reviewed; warnings are historical evidence, not
# alternatives a future run may silently normalize to plain pass.
REQUIRED_RESULTS["stripped-path"] = {"pass-with-disclosed-warning"}
REQUIRED_RESULTS["issue-gates"] = {"pass-with-known-warnings"}
REQUIRED_RESULTS["release-activation"] = {"expected-refusal"}
root = Path(__file__).resolve().parent
evidence = root / "evidence"
index_path = evidence / "index.json"


def fail(message):
    raise SystemExit(message)


def artifact_paths():
    paths = [p for p in evidence.rglob("*") if p.is_file() and p != index_path]
    paths += [
        root / "item.md",
        root / "validation.md",
        root / "verify-command-parity.sh",
        root / "verify-install-channels.sh",
        root / "verify-homebrew-prelive.sh",
        root / "verify-evidence-index.py",
        root / "sanitize-evidence.py",
        root / "scan-evidence.py",
    ]
    missing = [str(p) for p in paths if not p.is_file()]
    if missing:
        fail(f"missing indexed artifact: {missing}")
    return {p.relative_to(root).as_posix(): p for p in paths}


def digest_rows(paths):
    rows = []
    for rel, path in sorted(paths.items()):
        data = path.read_bytes()
        rows.append({"path": rel, "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()})
    return rows


def validate_manifest(final):
    manifest = json.loads((evidence / "command-manifest.json").read_text())
    if manifest.get("tested_commit") != TESTED_COMMIT:
        fail("manifest tested_commit mismatch")
    commands = manifest.get("commands", [])
    ids = [row.get("id") for row in commands]
    if len(ids) != len(set(ids)):
        fail("duplicate command id")
    missing = sorted(REQUIRED_IDS - set(ids))
    if missing:
        fail(f"missing required commands: {missing}")
    bad = {
        row["id"]: row.get("result")
        for row in commands
        if row.get("result") not in REQUIRED_RESULTS.get(row["id"], set())
    }
    if final and bad:
        fail(f"overall=pass with non-passing commands: {bad}")
    if final and manifest.get("overall") != "pass":
        fail("final evidence requires manifest overall=pass")
    if not final and manifest.get("overall") == "pass":
        fail("manifest overall=pass but final validation was not requested")
    if final:
        for row in commands:
            for pattern in [part.strip() for part in row.get("output", "").split(",") if part.strip()]:
                if not list(evidence.glob(pattern)):
                    fail(f"{row['id']}: missing referenced output {pattern}")
        matrix = json.loads((evidence / "acceptance-matrix.json").read_text())
        unfinished = [row["criterion"] for row in matrix.get("criteria", []) if row.get("result") != "pass"]
        if unfinished:
            fail(f"unfinished acceptance criteria: {unfinished}")
        assessment = json.loads((evidence / "assessment.json").read_text())
        required_assessment = {"schema_version", "tested_commit", "overall", "models", "findings"}
        if not required_assessment <= set(assessment) or assessment.get("tested_commit") != TESTED_COMMIT or assessment.get("overall") != "pass" or len(assessment.get("models", [])) != 4 or len(set(assessment.get("models", []))) != 4 or not all(assessment.get("models", [])):
            fail("review assessment is incomplete or does not authorize pass")
        residue = json.loads((evidence / "residue.json").read_text())
        required_residue = {"schema_version", "tested_commit", "overall", "production_diff", "unexpected_candidate_processes", "unexpected_tracked_paths", "public_mutation", "tag_mutation", "unrelated_worktree_touched"}
        if not required_residue <= set(residue) or residue.get("tested_commit") != TESTED_COMMIT:
            fail("residue schema/identity is incomplete")
        if residue.get("overall") != "pass" or residue.get("production_diff") or residue.get("unexpected_candidate_processes") or residue.get("unexpected_tracked_paths") or residue.get("public_mutation") or residue.get("tag_mutation") or residue.get("unrelated_worktree_touched"):
            fail("residue is not clean")
        exceptions = json.loads((evidence / "exceptions.json").read_text())
        exception_ids = {row.get("id") for row in exceptions.get("exceptions", [])}
        if exceptions.get("tested_commit") != TESTED_COMMIT or exception_ids != {"exploratory-legacy-dispatch-log-write", "stripped-path-advisories"}:
            fail("disclosed exceptions lack exact recorded disposition")
        sanitization = json.loads((evidence / "sanitization-report.json").read_text())
        if sanitization.get("tested_commit") != TESTED_COMMIT or sanitization.get("result") != "pass" or sanitization.get("findings"):
            fail("evidence sanitization did not pass")
        toolchain = json.loads((evidence / "toolchain.json").read_text())
        homebrew = json.loads((evidence / "homebrew-acceptance.json").read_text())
        if toolchain.get("homebrew") != homebrew.get("homebrew_version") or toolchain.get("homebrew_git_commit") != homebrew.get("homebrew_git_commit"):
            fail("Homebrew identity evidence disagrees")
    return manifest


writing = sys.argv[1:] == ["--write"]
if sys.argv[1:] not in ([], ["--write"]):
    fail("usage: verify-evidence-index.py [--write]")
manifest_data = json.loads((evidence / "command-manifest.json").read_text())
is_final = manifest_data.get("overall") == "pass"
manifest = validate_manifest(final=is_final)
paths = artifact_paths()
if writing:
    passed = manifest.get("overall") == "pass"
    index = {
        "schema_version": 1,
        "tested_commit": TESTED_COMMIT,
        "tested_tree": TESTED_TREE,
        "generated_at": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
        "overall": "pass" if passed else "pending-review-and-residue",
        "r9_authorized": passed,
        "release_authorized": False,
        "artifacts": digest_rows(paths),
    }
    index_path.write_text(json.dumps(index, indent=2) + "\n")

index = json.loads(index_path.read_text())
if index.get("schema_version") != 1 or index.get("tested_commit") != TESTED_COMMIT or index.get("tested_tree") != TESTED_TREE:
    fail("index identity mismatch")
if index.get("release_authorized") is not False:
    fail("R8 must never authorize release")
listed = {row["path"]: row for row in index.get("artifacts", [])}
if set(listed) != set(paths):
    fail(f"index mismatch missing={sorted(set(paths)-set(listed))} extra={sorted(set(listed)-set(paths))}")
for row in digest_rows(paths):
    rel = row["path"]
    if listed[rel].get("bytes") != row["bytes"] or listed[rel].get("sha256") != row["sha256"]:
        fail(f"artifact digest mismatch: {rel}")
expected_pass = manifest.get("overall") == "pass"
expected_overall = "pass" if expected_pass else "pending-review-and-residue"
if index.get("r9_authorized") != expected_pass or index.get("overall") != expected_overall:
    fail("index authorization/overall disagrees with manifest")
print(f"evidence index verified: {len(paths)} artifacts; r9_authorized={index['r9_authorized']}")
