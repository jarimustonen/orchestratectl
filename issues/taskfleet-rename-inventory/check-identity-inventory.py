#!/usr/bin/env python3
"""Generate/check the deterministic old-identity classification ledger for ADR 0002."""
from __future__ import annotations
import argparse, pathlib, re, subprocess, sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
OUT = ROOT / "issues/taskfleet-rename-inventory/identity-occurrences.tsv"
SELF_PREFIX = "issues/taskfleet-rename-inventory/"
TERM = re.compile(r"orchestratectl|octl(?:-core|_core)?|ORCHESTRATECTL_[A-Z0-9_]+|OCTL_[A-Z0-9_]+", re.I)
BRANDED_INPUT = re.compile(r"ORCHESTRATECTL_(?:HOME|PROFILE|HARNESS|LOG)\b|\.orchestratectl(?:\.toml)?|~/\.orchestratectl", re.I)
CONTROL = re.compile(r"OCTL_(?:TEST_|CREATE_SH|MERGE_SH|SUPERVISE_BIN|READY_|PID_FILE_|DEATH_|WATCHDOG_|NO_WORKER_|STILLBORN_|AGENT_|TMUX_|IDEMPOTENCY_|CHILD_|READINESS_FD)")


def files() -> list[str]:
    raw = subprocess.check_output(
        ["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"], cwd=ROOT
    )
    return sorted({p for p in raw.decode().split("\0") if p})


def classify(path: str, line: str, token: str) -> tuple[str, str]:
    low = path.lower()
    if path.startswith(SELF_PREFIX) or low.startswith("history/"):
        return "G", "R0-history"
    if low.startswith("fixtures/orchestratectl-0.5.1/"):
        return "F", "R2/R3/R5/R8"
    if low.startswith("issues/") or path == "CHANGELOG.md" or low.startswith("docs/decisions/"):
        return "G", "history-reader"
    if path == "Cargo.lock" or "tests/snapshots/" in low or path == ".github/workflows/release.yml":
        return "G", "generator-owner"
    if "orchestratectl.worker-telemetry-adapter" in line or "orchestratectl.dev/schemas/" in line:
        return "P", "protocol-reader"
    if low.startswith("contracts/worker-telemetry-v1/") and token.upper().startswith("OCTL_"):
        return "P", "worker-telemetry-v1"
    if token.upper().startswith("OCTL_"):
        return ("F", "test/control-seam") if CONTROL.search(token.upper()) else ("P", "protocol-reader")
    if BRANDED_INPUT.search(line):
        return "B", "R2/R3"
    if "target:" in line and ("orchestratectl::" in line or "octl_core::" in line):
        return "B", "R2/R4-logging"
    if low.startswith("scripts/") or low.startswith(".github/") or path in {"OSS-RELEASE.md", "dist-workspace.toml"}:
        return "A", "R6/R7"
    if "/skills/" in low or path in {"README.md", "AGENTS.md", "CONTRIBUTING.md", "SECURITY.md", "LICENSE", "TODO.md"}:
        return "A", "R5"
    if low.startswith("contracts/"):
        return "A", "R5"
    if low.startswith("crates/octl-core/schemas/"):
        return "P", "schema-history"
    if low.startswith("crates/") or path in {"Cargo.toml", "deny.toml", "clippy.toml", "rustfmt.toml"}:
        return "A", "R1/R4/R5"
    return "A", "R5"


def generate() -> str:
    rows = ["path\tline\tterm\tclass\towner\n"]
    for path in files():
        # The classifier, ledger and explanatory R0 documents describe old
        # identities rather than being product surfaces; excluding this whole
        # directory also prevents the generated ledger from inventorying itself.
        if path.startswith(SELF_PREFIX):
            continue
        p = ROOT / path
        if not p.is_file():
            continue
        try:
            text = p.read_text(encoding="utf-8")
        except (UnicodeDecodeError, OSError):
            continue
        for lineno, line in enumerate(text.splitlines(), 1):
            for match in TERM.finditer(line):
                token = match.group(0)
                cls, owner = classify(path, line, token)
                rows.append(f"{path}\t{lineno}\t{token}\t{cls}\t{owner}\n")
    return "".join(rows)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--write", action="store_true")
    args = parser.parse_args()
    rendered = generate()
    if args.write:
        OUT.write_text(rendered, encoding="utf-8")
        return 0
    current = OUT.read_text(encoding="utf-8") if OUT.exists() else ""
    if current != rendered:
        print("identity ledger is stale; run check-identity-inventory.py --write", file=sys.stderr)
        return 1
    counts: dict[str, int] = {}
    for row in rendered.splitlines()[1:]:
        cls = row.split("\t")[3]
        counts[cls] = counts.get(cls, 0) + 1
    print("identity inventory valid: " + ", ".join(f"{k}={counts[k]}" for k in sorted(counts)))
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
