#!/usr/bin/env bash
# Fail when tracked paths or text contain a retired product identity.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
cd "$root"
long="orches""tratectl"
short="oc""tl"
pattern="${long}|${short}"

path_hits="$(git ls-files | grep -Ei "$pattern" || true)"
content_hits="$(git grep -I -i -E "$pattern" -- . || true)"
if [[ -n "$path_hits" || -n "$content_hits" ]]; then
  [[ -z "$path_hits" ]] || { printf 'retired identity in tracked paths:\n%s\n' "$path_hits" >&2; }
  [[ -z "$content_hits" ]] || { printf 'retired identity in tracked text:\n%s\n' "$content_hits" >&2; }
  exit 1
fi
printf 'canonical identity check passed: zero tracked path or text references\n'
