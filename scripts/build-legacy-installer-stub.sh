#!/usr/bin/env bash
# Build cargo-dist's bounded old latest-installer compatibility artifact.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
out="${TASKFLEET_STUB_OUT:-$repo_root/target/taskfleet-extra/orchestratectl-installer.sh}"
mkdir -p "$(dirname "$out")"
cat >"$out" <<'STUB'
#!/bin/sh
# Taskfleet 0.6.x-0.7.x migration stub. This file intentionally installs nothing.
cat >&2 <<'MESSAGE'
The orchestratectl binary installer has been retired.

Taskfleet is the canonical command. Finish or quiesce old orchestratectl work,
refresh automation and installed skills, then use the Taskfleet installer:
https://github.com/jarimustonen/taskfleet/releases/latest/download/taskfleet-installer.sh

No files or state were changed.
MESSAGE
exit 1
STUB
chmod 755 "$out"
printf '%s\n' "$out"
