#!/usr/bin/env bash
# Reports what Spike 1 needs. Read-only: installs and changes nothing.
set -u

missing=0

report() {
  if [ "$2" = "ok" ]; then
    printf 'ok       %s\n' "$1"
  else
    printf 'MISSING  %s\n' "$1"
    missing=$((missing + 1))
  fi
}

have() {
  if command -v "$1" >/dev/null 2>&1; then report "$1" ok; else report "$1" no; fi
}

if [ -r /dev/kvm ] && [ -w /dev/kvm ]; then report "/dev/kvm access" ok; else report "/dev/kvm access" no; fi
if ldconfig -p 2>/dev/null | grep -q 'libkrun\.so'; then report "libkrun" ok; else report "libkrun" no; fi
if ldconfig -p 2>/dev/null | grep -q 'libkrunfw\.so'; then report "libkrunfw" ok; else report "libkrunfw" no; fi
for tool in cargo cmake ninja bwrap; do have "$tool"; done

available_mib=$(awk '/MemAvailable/ { print int($2 / 1024) }' /proc/meminfo)
if [ "$available_mib" -ge 3072 ]; then
  report "free RAM ${available_mib} MiB" ok
else
  report "free RAM ${available_mib} MiB (want 3072)" no
fi

printf '\n%s item(s) missing\n' "$missing"
