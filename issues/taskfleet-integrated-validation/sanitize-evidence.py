#!/usr/bin/env python3
"""Create bounded R8 logs with machine-private paths replaced deterministically."""
import re
import sys
from pathlib import Path

if len(sys.argv) != 3:
    raise SystemExit("usage: sanitize-evidence.py INPUT OUTPUT")
source, destination = map(Path, sys.argv[1:])
text = source.read_text(errors="replace")
rules = [
    (re.escape(str(Path.cwd())), "<REPO>"),
    (r"/Users/jari(?:/[^\s:'\"`]+)*", "<USER_PATH>"),
    (r"/private/var/folders/[^\s:'\"`]+", "<TMP_PATH>"),
    (r"/var/folders/[^\s:'\"`]+", "<TMP_PATH>"),
    (r"/tmp/(?:taskfleet-r8|r8-)[^\s:'\"`]+", "<TMP_PATH>"),
]
for pattern, replacement in rules:
    text = re.sub(pattern, replacement, text)
# Preserve line structure while removing terminal-padding whitespace emitted by
# curl/progress renderers and Markdown hard-breaks from model responses.
had_final_newline = text.endswith("\n")
text = "\n".join(line.rstrip() for line in text.splitlines()) + ("\n" if had_final_newline else "")
destination.parent.mkdir(parents=True, exist_ok=True)
destination.write_text(text)
