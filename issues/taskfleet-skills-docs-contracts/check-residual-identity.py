#!/usr/bin/env python3
"""Fail when an old product token is not covered by the ADR 0002 residual classes."""
from pathlib import Path
from typing import Optional
import re, sys

ROOT = Path(__file__).resolve().parents[2]
PAT = re.compile(r"orchestratectl|octl", re.I)
SKIP_DIRS = {".git", "target", "history"}


def classify(path: str, line: str) -> Optional[str]:
    low = line.lower()
    if path.startswith(".workmux/"):
        return "generated-persisted-old-prompt"
    if path.startswith("issues/") or path in {"CHANGELOG.md", "TODO.md"}:
        return "permanent-history"
    if path.startswith("fixtures/") or "/fixtures/" in path:
        return "fixture"
    if path == "Cargo.lock" or path.endswith(".snap") or path == ".github/workflows/release.yml":
        return "generated"
    if "octl_" in low or "octl-*" in low:
        return "permanent-protocol"
    if "orchestratectl.worker-telemetry-adapter" in low:
        return "permanent-protocol"
    if path.startswith("docs/decisions/"):
        return "permanent-history"
    if path.startswith((".github/workflows/", "scripts/")) or path in {"dist-workspace.toml"}:
        return "deferred-r6-r7-generated-release"
    if path.startswith(".github/"):
        return "deferred-r7-r9-public-identity"
    if path.startswith("compat/orchestratectl/") or (path == "Cargo.toml" and "compat/orchestratectl" in line):
        return "bounded-cli-wrapper"
    if path == "Cargo.toml":
        return "deferred-r9-public-url"
    if path in {"CLAUDE.md"}:
        return "symlinked-agent-documentation"
    if path.startswith("contracts/"):
        return "permanent-protocol"
    if path == "OSS-RELEASE.md":
        return "bounded-wrapper-or-deferred-public-location"
    if path in {"README.md", "SECURITY.md", "CONTRIBUTING.md", "AGENTS.md", "ARCHITECTURE.md"}:
        return "bounded-compatibility-or-deferred-url"
    if path.startswith("docs/"):
        return "permanent-safety-or-migration-history"
    if path.startswith("crates/taskfleet/tests/") or "#[test]" in line:
        return "fixture"
    if path in {"build.rs"} or path.startswith("crates/taskfleet/") and path.endswith(("build.rs", "LICENSE")):
        return "bounded-build-key-or-legal-history"
    if path == "crates/taskfleet/src/skill.rs":
        return "bounded-skill-provenance-migration"
    if path.startswith("crates/taskfleet/src/"):
        compat = (
            "orchestratectl::" in low
            or "orchestratectl_git_commit" in low
            or "orchestratectl_" in low
            or ".orchestratectl" in low
            or "compat/orchestratectl" in low
            or "invocationidentity::orchestratectl" in low
            or '"orchestratectl"' in low
            or "orchestratectl 0.5.1" in low
            or "__octl_" in low
            or "serial(octl" in low
            or "octl" in low
            and (
                "test" in path
                or "fixture" in low
                or path.endswith(
                    (
                        "run/spawn.rs",
                        "supervise/watchdog.rs",
                        "supervise/cleanup.rs",
                        "supervise/capture.rs",
                    )
                )
            )
            or path.endswith("src/lib.rs")
            or "orchestratectl" in low
            and ("legacy" in low or "compat" in low or "0.5.1" in low)
        )
        return "bounded-compatibility-or-permanent-safety" if compat else None
    if path.startswith("crates/taskfleet-core/"):
        return "permanent-schema-history-or-fixture"
    if path.startswith("crates/taskfleet/") and path.endswith(("AGENTS.md", "CLAUDE.md")):
        return "bounded-compatibility-documentation"
    return None

unclassified = []
counts: dict[str, int] = {}
for p in ROOT.rglob("*"):
    if not p.is_file() or any(part in SKIP_DIRS for part in p.parts):
        continue
    rel = p.relative_to(ROOT).as_posix()
    try:
        lines = p.read_text(errors="strict").splitlines()
    except (UnicodeDecodeError, OSError):
        continue
    for number, line in enumerate(lines, 1):
        if not PAT.search(line):
            continue
        category = classify(rel, line)
        if category is None:
            unclassified.append(f"{rel}:{number}:{line.strip()}")
        else:
            counts[category] = counts.get(category, 0) + 1

for category in sorted(counts):
    print(f"{category}\t{counts[category]}")
if unclassified:
    print("\nUNCLASSIFIED active old identity:", file=sys.stderr)
    print("\n".join(unclassified), file=sys.stderr)
    raise SystemExit(1)
print(f"classified_total\t{sum(counts.values())}")
