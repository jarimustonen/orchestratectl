#!/usr/bin/env python3
"""Fail closed on unexplained post-R9 old GitHub source coordinates."""
import json, pathlib, re, subprocess, sys
root = pathlib.Path(__file__).resolve().parents[3]
pattern = re.compile(r"(https://github\.com/|git@github\.com:)?jarimustonen/orchestratectl")
allowed = {
    "legacy-homebrew": ["AGENTS.md", "CHANGELOG.md", "README.md", "scripts/test-distribution-topology.sh"],
    "accepted-decision-history": ["docs/decisions/0002-taskfleet-rename-migration.md"],
    "frozen-fixture": ["fixtures/", "crates/taskfleet/tests/fixtures/legacy-skills/"],
    "issue-history-evidence": ["issues/"],
}
proc = subprocess.run(["git", "grep", "-n", "-I", "-E", pattern.pattern], cwd=root,
                      text=True, stdout=subprocess.PIPE)
if proc.returncode not in (0, 1):
    raise SystemExit(proc.returncode)
rows=[]; bad=[]
for line in proc.stdout.splitlines():
    path, lineno, text = line.split(":", 2)
    if path.endswith("check-residual-source-identity.py"):
        continue
    category = next((cat for cat, prefixes in allowed.items()
                     if any(path == p or path.startswith(p) for p in prefixes)), None)
    row={"path":path,"line":int(lineno),"category":category,"text":text.strip()}
    rows.append(row)
    if category is None: bad.append(row)
# Maintained exact old web/SSH source URLs are forbidden even if a broad coordinate allowlist evolves.
maintained_exact=[]
for row in rows:
    if row["category"] not in {"issue-history-evidence","frozen-fixture","accepted-decision-history"} and (
        "https://github.com/jarimustonen/orchestratectl" in row["text"] or
        "git@github.com:jarimustonen/orchestratectl" in row["text"]):
        maintained_exact.append(row)
result={"schema_version":1,"classified_total":len(rows),"unclassified":bad,
        "maintained_exact_old_source_urls":maintained_exact,"residuals":rows}
print(json.dumps(result,indent=2,sort_keys=True))
if bad or maintained_exact: sys.exit(2)
