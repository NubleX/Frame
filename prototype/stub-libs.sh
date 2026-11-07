#!/usr/bin/env bash
# Shrinks a guest root without rebuilding Chromium. Libraries the browser never
# calls become stubs that stop the process and name the symbol if they ever are
# called, then anything no longer needed is deleted. libcef itself is untouched.
# Usage: stub-libs.sh <root> <library-name>...
set -euo pipefail
root=$1; shift
libs="$root/usr/lib/x86_64-linux-gnu"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
stubbed=" $* "

imports() { nm -D --undefined-only "$1" 2>/dev/null | awk '{print $2}' | sed 's/@.*//'; }

# Everything the real code imports, from anything that stays real.
for file in "$root/frame/libcef.so" "$root/frame/engine" "$libs"/*.so*; do
  case "$stubbed" in *" $(basename "$file") "*) continue ;; esac
  [ -f "$file" ] && imports "$file"
done | sort -u > "$work/wanted"

for name in "$@"; do
  nm -D --defined-only "$libs/$name" | awk '{print $3}' | sed 's/@.*//' | sort -u > "$work/defined"
  {
    echo '#include <unistd.h>'
    echo 'static void hit(const char *s, unsigned long n) { write(2, s, n); _exit(99); }'
    comm -12 "$work/wanted" "$work/defined" | grep -E '^[A-Za-z_][A-Za-z0-9_]*$' | while read -r symbol; do
      text="STUB_CALLED $name $symbol\n"
      echo "void $symbol(void) { hit(\"$text\", $((${#text} - 1))); }"
    done
  } > "$work/stub.c"
  gcc -shared -fPIC -O1 -w -Wl,-soname,"$name" -o "$libs/$name" "$work/stub.c"
done

# Keep only what is still reachable.
: > "$work/keep"
queue=("$root/frame/libcef.so" "$root/frame/engine" "$root/bin/busybox")
for module in libsoftokn3.so libfreebl3.so libfreeblpriv3.so libnssckbi.so libnssdbm3.so; do
  queue+=("$libs/$module")
done
while [ "${#queue[@]}" -gt 0 ]; do
  file=${queue[0]}; queue=("${queue[@]:1}")
  [ -f "$file" ] || continue
  for needed in $(readelf -d "$file" | awk -F'[][]' '/NEEDED/ {print $2}'); do
    grep -qx "$needed" "$work/keep" && continue
    echo "$needed" >> "$work/keep"
    queue+=("$libs/$needed")
  done
done
for module in libsoftokn3.so libfreebl3.so libfreeblpriv3.so libnssckbi.so libnssdbm3.so; do
  echo "$module" >> "$work/keep"
done
for file in "$libs"/*; do
  case "$(basename "$file")" in *.chk) continue ;; esac
  grep -qx "$(basename "$file")" "$work/keep" || rm -f "$file"
done
echo "libraries left: $(ls "$libs" | wc -l), $(du -sh "$libs" | cut -f1)"
