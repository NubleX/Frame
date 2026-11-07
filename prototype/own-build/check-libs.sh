#!/usr/bin/env bash
# Fails if a binary needs any library beyond what every Linux has, or what
# Frame ships itself. Usage: check-libs.sh <binary>...
set -u
common='libc.so.6 libm.so.6 libdl.so.2 libpthread.so.0 librt.so.1 libgcc_s.so.1 ld-linux-x86-64.so.2'
ours='libcef.so libkrun.so.1'
status=0
for binary in "$@"; do
  extra=()
  for library in $(readelf -d "$binary" | awk -F'[][]' '/NEEDED/ {print $2}'); do
    case " $common $ours " in
      *" $library "*) ;;
      *) extra+=("$library") ;;
    esac
  done
  if [ "${#extra[@]}" -eq 0 ]; then
    echo "PASS $(basename "$binary")"
  else
    echo "FAIL $(basename "$binary") needs ${#extra[@]} more: ${extra[*]}"
    status=1
  fi
done
exit "$status"
