#!/usr/bin/env bash
# Runs the spike. Usage: run.sh <work-dir> <memory-mib> <seconds> <url> [engine flags]
set -u
work=$1 memory=$2 seconds=$3 url=$4; shift 4
socket="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/frame-spike1.sock"
export LD_LIBRARY_PATH="$work/prefix/krun"

"$work/target/release/viewer" "$socket" > "$work/viewer.log" 2>&1 &
viewer=$!
sleep 1
timeout "$seconds" "$work/target/release/launcher" "$work/root" "$memory" 2 --frames "$socket" -- \
  /bin/start "--frame-url=$url" "$@" > "$work/guest.log" 2>&1 &
vm=$!
peak=0 ticks=0 samples=0
while kill -0 "$vm" 2>/dev/null; do
  pid=$(pgrep -P "$vm" -f launcher | head -1)
  if [ -n "$pid" ]; then
    rss=$(awk '/VmRSS/ {print int($2 / 1024)}' "/proc/$pid/status" 2>/dev/null || echo 0)
    [ "${rss:-0}" -gt "$peak" ] && peak=$rss
    ticks=$(awk '{print $14 + $15}' "/proc/$pid/stat" 2>/dev/null || echo "$ticks")
    samples=$((samples + 1))
  fi
  sleep 1
done
wait "$vm"
echo "launcher exit $? host_vmm_peak_rss=${peak}MiB avg_cpu=$((ticks / (samples > 0 ? samples : 1)))% of one core"
kill "$viewer" 2>/dev/null
