#!/usr/bin/env bash
# Starts the barebone browser. Closing the window burns the compartment: the
# guest root is read-only and everything else lived in guest RAM.
# Usage: run.sh <work-dir> <url> [engine flags]
set -u
work=$1 url=$2; shift 2
run="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/frame-$$"
mkdir -m 700 "$run"
# The jail keeps only the real path, so a symlink must be resolved first.
krun=$(realpath "$work/prefix/krun")
export LD_LIBRARY_PATH="$krun"

"$work/target/release/frame-gate" "$run/gate.sock" 2> "$work/gate.log" &
gate=$!
"$work/target/release/viewer" "$run/pixels.sock" "$run/membrane.sock" > "$work/viewer.log" 2>&1 &
viewer=$!
sleep 1
"$work/target/release/launcher" "$work/root" 1024 2 "$krun" "$run" -- \
  /bin/start "--frame-url=$url" "$@" > "$work/guest.log" 2>&1 &
vm=$!
trap 'kill "$vm" "$viewer" "$gate" 2>/dev/null; rm -rf "$run"' EXIT
wait -n "$vm" "$viewer"
