#!/usr/bin/env python3
"""Fail closed on credentials and machine-private paths in committed R9 evidence."""
from pathlib import Path
import re, sys
root = Path(__file__).resolve().parent
patterns = {
    "private-home": re.compile(rb"/(?:Users|home)/jari(?:/|\\b)"),
    "authorization-header": re.compile(rb"(?i)authorization:\s*(?:bearer|token)\s+"),
    "github-token": re.compile(rb"(?:ghp_|github_pat_)[A-Za-z0-9_]+"),
    "private-key": re.compile(rb"-----BEGIN (?:RSA |OPENSSH |EC )?PRIVATE KEY-----"),
}
bad=[]
for path in sorted(root.rglob("*")):
    if not path.is_file() or path.name == "index.json":
        continue
    data=path.read_bytes()
    for name, regex in patterns.items():
        if regex.search(data): bad.append({"path":str(path.relative_to(root)),"pattern":name})
if bad:
    print(bad, file=sys.stderr)
    raise SystemExit(2)
print("R9 evidence scan passed: no credentials or machine-private paths")
