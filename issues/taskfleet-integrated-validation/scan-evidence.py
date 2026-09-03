#!/usr/bin/env python3
"""Fail closed when R8 artifacts contain likely credentials or machine-private paths."""
import json
import re
import sys
from datetime import datetime, timezone
from pathlib import Path

root = Path(__file__).resolve().parent
report_path = root / "evidence" / "sanitization-report.json"
patterns = {
    "github_token": re.compile(rb"(?:github_pat_|gh[pousr]_|ghu_)[A-Za-z0-9_]{12,}"),
    "authorization_header": re.compile(rb"Authorization:\s*(?:Bearer|token)\s+\S+", re.I),
    "registry_token_value": re.compile(rb"(?:CARGO_REGISTRY_TOKEN|CRATES_IO_TOKEN)=[^<\s\"']+"),
    "macos_private_path": re.compile(rb"/(?:Users/[^/<\s]+|private/var/folders|var/folders)/"),
    "linux_private_home": re.compile(rb"(?<![A-Za-z0-9_.-])/home/[^/<\s]+/"),
}
paths = [p for p in root.rglob("*") if p.is_file() and p not in {report_path, Path(__file__), root / "sanitize-evidence.py", root / "verify-evidence-index.py"}]
findings = []
for path in paths:
    data = path.read_bytes()
    for name, pattern in patterns.items():
        if pattern.search(data):
            findings.append({"path": path.relative_to(root).as_posix(), "pattern": name})
report = {
    "schema_version": 1,
    "tested_commit": "c3ef8b740ac531f12ce81c759ed209d178cf36bd",
    "generated_at": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
    "scanned_files": len(paths),
    "patterns": sorted(patterns),
    "findings": findings,
    "result": "pass" if not findings else "fail",
}
if sys.argv[1:] == ["--write"]:
    report_path.write_text(json.dumps(report, indent=2) + "\n")
elif sys.argv[1:]:
    raise SystemExit("usage: scan-evidence.py [--write]")
if findings:
    raise SystemExit(json.dumps(findings, indent=2))
print(f"evidence sanitization scan passed: {len(paths)} files")
