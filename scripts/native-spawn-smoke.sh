#!/bin/sh
# Checked, bounded real-pi smoke test for Taskfleet's native materializer.
# Everything mutable lives in one disposable root and one private tmux socket.
set -eu

binary=${TASKFLEET_SMOKE_BIN:-"$(pwd)/target/release/taskfleet"}
case "$binary" in /*) ;; *) binary="$(pwd)/$binary" ;; esac
[ -x "$binary" ] || { echo "build first: cargo build --release -p taskfleet" >&2; exit 2; }
command -v git >/dev/null
command -v tmux >/dev/null
command -v workmux >/dev/null
command -v pi >/dev/null
command -v python3 >/dev/null

source_repo=$(git rev-parse --show-toplevel)
root=$(mktemp -d "${TMPDIR:-/tmp}/taskfleet-native-smoke.XXXXXXXX")
token=$(basename "$root" | tr -cd '[:alnum:]')
socket="taskfleet-smoke-$token"
session="smoke-$token"
run_id=
cleaned=0

inventory() {
  out=$1
  {
    echo '# source worktrees'
    git -C "$source_repo" worktree list --porcelain
    echo '# source fixture refs'
    git -C "$source_repo" for-each-ref --format='%(refname)' 'refs/heads/wt/*'
    echo '# default tmux'
    tmux list-windows -a -F '#{socket_path}\t#{session_name}\t#{window_id}\t#{pane_current_path}' 2>/dev/null || true
    echo '# supervisors'
    ps -axo command= | grep -E '[/]taskfleet supervise ' | sort || true
    echo '# external taskfleet homes'
    for home in "${TASKFLEET_HOME:-$HOME/.taskfleet}"; do
      [ "$home" = "$root/home" ] && continue
      if [ -d "$home/runs" ]; then
        find "$home/runs" -mindepth 1 -maxdepth 1 -type d -print | sort
      fi
    done
  } > "$out"
}

cleanup() {
  status=$?
  trap - EXIT HUP INT TERM
  if [ -n "$run_id" ] && [ -d "$root/home/runs/$run_id" ]; then
    HOME="$root/user" TASKFLEET_HOME="$root/home" TMUX_BIN="$root/bin/tmux" \
      "$binary" --output json run cancel "$run_id" >/dev/null 2>&1 || true
  fi
  # The private server is the final process/window containment boundary. tmux
  # can leave a stale socket inode after kill-server, so remove that exact
  # token-bound socket only after the private server has stopped.
  tmux -L "$socket" kill-server >/dev/null 2>&1 || true
  if [ -n "${socket_path:-}" ]; then
    case "$socket_path" in
      */tmux-$(id -u)/taskfleet-smoke-$token)
        tmux -S "$socket_path" has-session >/dev/null 2>&1 || rm -f "$socket_path"
        ;;
      *) echo "refusing unexpected smoke socket path: $socket_path" >&2; status=1 ;;
    esac
  fi
  if [ -d "$root/home/runs" ]; then
    find "$root/home/runs" -name supervisor.pid -type f -exec sh -c '
      for f do p=$(sed -n "s/ .*//p" "$f"); [ -n "$p" ] && kill "$p" 2>/dev/null || true; done
    ' sh {} +
  fi
  if [ -n "${socket_path:-}" ] && [ -e "$socket_path" ]; then
    echo "ERROR: private smoke socket survived cleanup: $socket_path" >&2
    status=1
  fi
  inventory "$root/after"
  if ! cmp -s "$root/before" "$root/after"; then
    echo 'ERROR: native smoke changed resources outside its sandbox:' >&2
    diff -u "$root/before" "$root/after" >&2 || true
    status=1
  fi
  cleaned=1
  rm -rf "$root"
  exit "$status"
}
trap cleanup EXIT HUP INT TERM

inventory "$root/before"
mkdir -p "$root/bin" "$root/home" "$root/user"
git init -q -b main "$root/repo"
git -C "$root/repo" -c user.name='Taskfleet Smoke' -c user.email=smoke@example.invalid \
  commit --allow-empty -qm base

tmux -L "$socket" new-session -d -s "$session" -c "$root/repo"
server_pid=$(tmux -L "$socket" display-message -p -t "$session" '#{pid}')
socket_path=$(tmux -L "$socket" display-message -p -t "$session" '#{socket_path}')
cat > "$root/bin/tmux" <<EOF
#!/bin/sh
exec tmux -L '$socket' "\$@"
EOF
chmod 755 "$root/bin/tmux"

pi_path=$(command -v pi)
cat > "$root/home/config.toml" <<EOF
[profiles.smoke]
description = "bounded real-pi native spawn smoke"
capability = "fast"
residency = "local"
agents = [{ harness = "pi", command = ["$pi_path"], telemetry = "worker-v1" }]
[profile]
default = "smoke"
EOF
cat > "$root/prompt.md" <<'EOF'
This is a bounded smoke test in a disposable repository. Do not invoke tools or
modify files. Reply with exactly SMOKE_OK, then exit.
EOF

created=$(cd "$root/repo" && \
  HOME="$root/user" TASKFLEET_HOME="$root/home" TMUX="$socket_path,$server_pid,0" \
  TMUX_BIN="$root/bin/tmux" \
  "$binary" --output json run create --kind spinoff --tmux-session "$session" \
    --title native-pi-smoke --prompt-file "$root/prompt.md" --agent-startup-timeout 30)
run_id=$(printf '%s' "$created" | python3 -c 'import json,sys; print(json.load(sys.stdin)["data"]["run_id"])')
worktree=$(printf '%s' "$created" | python3 -c 'import json,sys; print(json.load(sys.stdin)["data"]["worktree_path"])')
branch=$(printf '%s' "$created" | python3 -c 'import json,sys; print(json.load(sys.stdin)["data"]["branch"])')

# Native materialization deliberately archives its generated prompt as an
# untracked worktree diagnostic. In this disposable smoke only, prove that it is
# the sole change and remove that exact fixture-owned file before cancellation;
# production cleanup must continue preserving arbitrary dirty work.
archive="history/.worktree/$branch.md"
status=$(git -C "$worktree" status --porcelain --untracked-files=all)
[ "$status" = "?? $archive" ] || {
  echo "unexpected real-pi smoke worktree changes: $status" >&2
  exit 1
}
rm -f "$worktree/$archive"

# Creation plus the private PID/pane identity is the smoke assertion. Cancellation
# deliberately exercises non-merge cleanup of a now-clean disposable worktree.
HOME="$root/user" TASKFLEET_HOME="$root/home" TMUX_BIN="$root/bin/tmux" \
  "$binary" --output json run cancel "$run_id" >/dev/null

# Give the supervisor a bounded opportunity to finish its own cleanup. The trap
# remains the backstop on timeout, child failure, interruption, and assertion.
i=0
while [ "$i" -lt 100 ]; do
  live=0
  if [ -f "$root/home/runs/$run_id/supervisor.pid" ]; then
    pid=$(sed -n 's/ .*//p' "$root/home/runs/$run_id/supervisor.pid")
    kill -0 "$pid" 2>/dev/null && live=1
  fi
  [ "$live" -eq 0 ] && [ ! -e "$worktree" ] && break
  i=$((i + 1))
  sleep 0.05
done
if [ "$i" -ge 100 ]; then
  echo 'smoke resources did not clean up within 5 seconds' >&2
  HOME="$root/user" TASKFLEET_HOME="$root/home" "$binary" --output json run show "$run_id" >&2 || true
  git -C "$worktree" status --porcelain --untracked-files=all >&2 2>/dev/null || true
  exit 1
fi

echo "native real-pi smoke passed in private socket $socket"
