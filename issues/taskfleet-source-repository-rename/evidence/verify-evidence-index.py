#!/usr/bin/env python3
"""Write or verify R9's immutable evidence index."""
from pathlib import Path
import hashlib, json, sys
issue = Path(__file__).resolve().parent.parent
evidence = issue / "evidence"
index_path = evidence / "index.json"

def sha(path): return hashlib.sha256(path.read_bytes()).hexdigest()
def artifacts():
    paths=[p for p in evidence.rglob("*") if p.is_file() and p != index_path]
    paths += [issue / "item.md", issue / "validation.md"]
    return [{"path":str(p.relative_to(issue)),"bytes":p.stat().st_size,"sha256":sha(p)}
            for p in sorted(paths)]
def load(name): return json.loads((evidence / name).read_text())

before, after = load("before-repository.json"), load("after-repository.json")
if not (before["id"] == after["id"] == 1265770191 and
        before["node_id"] == after["node_id"] == "R_kgDOS3Iezw" and
        before["full_name"] == "jarimustonen/orchestratectl" and
        after["full_name"] == "jarimustonen/taskfleet"):
    raise SystemExit("repository identity continuity failed")
ops, residual = load("canonical-operations.json"), load("residual-source-identity.json")
if not (ops["canonical_api"] == ops["canonical_ssh_clone"] == ops["canonical_fetch"] ==
        ops["canonical_candidate_branch_push"] == "pass" and not ops["redirect_dependency"]):
    raise SystemExit("canonical operation proof failed")
if residual["unclassified"] or residual["maintained_exact_old_source_urls"]:
    raise SystemExit("old source identity residue is unclassified")
runs, jobs = load("candidate-runs.json"), load("candidate-jobs.json")
if len(runs) != 2 or any(r["headSha"] != "076f983c498de1ca2fc8fe0b919130ffbd52dc27" or
                         r["event"] != "pull_request" or r["conclusion"] != "success" for r in runs):
    raise SystemExit("candidate runs do not attest the exact source commit")
self_hosted=[j for j in jobs if j["name"] == "test (self-hosted-macos-arm64)"]
if len(self_hosted) != 1 or self_hosted[0]["runner_id"] != 21 or self_hosted[0]["conclusion"] != "success" or not {"self-hosted","macOS","ARM64"}.issubset(self_hosted[0]["labels"]):
    raise SystemExit("self-hosted macOS ARM64 proof failed")
required={"rustfmt","version-snapshots","clippy","test (ubuntu-latest)","test (macos-latest)",
          "test (self-hosted-macos-arm64)","msrv (1.85)","docs","cargo-deny"}
if not required.issubset({j["name"] for j in jobs if j["conclusion"] == "success"}):
    raise SystemExit("candidate CI job set is incomplete")
no_release=load("after-no-release-no-tap.json")
if not (no_release["tag_refs_sha256"] == "16ac4238a89bf6108ec7564dc054ef3daa723185a805bdcfb3590753b5673e4a" and
        no_release["tag_ref_lines"] == 28 and no_release["github_releases"] == 17 and
        no_release["shipshape_in_flight"] == no_release["shipshape_unreadable"] ==
        no_release["release_workflow_in_flight"] == 0 and no_release["no_release_authorized"]):
    raise SystemExit("no-release proof failed")
if load("before-secret-names.json") != load("after-secret-names.json"):
    raise SystemExit("secret-name metadata changed")
for name in ("actions-permissions", "workflow-permissions", "rulesets", "main-protection"):
    if load(f"before-{name}.json") != load(f"after-{name}.json"):
        raise SystemExit(f"repository setting changed unexpectedly: {name}")
if load("candidate-pr.json")["state"] != "CLOSED" or load("candidate-pr.json")["mergedAt"] is not None:
    raise SystemExit("temporary candidate PR was not closed unmerged")
base={
  "schema_version":1,
  "repository_id":1265770191,
  "repository_node_id":"R_kgDOS3Iezw",
  "before_repository":"jarimustonen/orchestratectl",
  "after_repository":"jarimustonen/taskfleet",
  "candidate_commit":"076f983c498de1ca2fc8fe0b919130ffbd52dc27",
  "candidate_tree":"06aaf232a85833ac1762e7a2fcf89b38cf9e6572",
  "candidate_ci_run":33814447787,
  "candidate_release_plan_run":33814447929,
  "candidate_pr":1,
  "candidate_gate":"pass",
  "final_main_ci":"conductor-pending",
  "r9_candidate_authorized":True,
  "release_authorized":False,
  "artifacts":artifacts(),
}
if len(sys.argv)==2 and sys.argv[1]=="--write":
    index_path.write_text(json.dumps(base,indent=2,sort_keys=True)+"\n")
elif len(sys.argv)!=1:
    raise SystemExit("usage: verify-evidence-index.py [--write]")
if not index_path.exists(): raise SystemExit("index missing")
actual=json.loads(index_path.read_text())
if actual != base: raise SystemExit("R9 evidence index drift")
if actual["release_authorized"] or actual["final_main_ci"] != "conductor-pending":
    raise SystemExit("R9 evidence must not authorize release or claim final main CI")
print(f"R9 evidence index verified: {len(base['artifacts'])} artifacts; candidate pass; release_authorized=false")
