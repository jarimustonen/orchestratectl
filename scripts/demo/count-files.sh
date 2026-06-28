#!/usr/bin/env bash
# count-files.sh — print the number of files tracked in this git repo.
# Output is a bare integer so downstream scripts can capture it.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

git ls-files | wc -l | tr -d '[:space:]'
echo
