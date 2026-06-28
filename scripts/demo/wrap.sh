#!/usr/bin/env bash
# wrap.sh — call count-files.sh and report the repo's file count in a sentence.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

n="$("${SCRIPT_DIR}/count-files.sh")"

printf 'The orchestratectl repo has %s files.\n' "$n"
