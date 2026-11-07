#!/usr/bin/env bash
# Builds Frame into one folder from nothing. Versions are pinned and downloads
# are checked. Nothing is installed system-wide.
# Usage: setup.sh <work-dir>
set -euo pipefail

libkrunfw_version=v5.6.2
libkrunfw_sha256=016c32ddb2a28aa300382cab33352d63a41a901eb36ee8d1ca24b389791c8b91
libkrun_version=v1.19.8
libkrun_commit=c4d283c233bc0ae1993c289ec2a1b4da2242f1f8

repo=$(cd "$(dirname "$0")/.." && pwd)
mkdir -p "$1"
work=$(cd "$1" && pwd)
mkdir -p "$work/dl" "$work/src" "$work/prefix/krun" "$work/cef-store"

for tool in cargo cc make cmake ninja git curl busybox nm readelf ldd; do
  command -v "$tool" > /dev/null || { echo "setup: missing $tool"; exit 1; }
done
[ -w /dev/kvm ] || { echo "setup: no write access to /dev/kvm"; exit 1; }

echo '== libkrunfw, prebuilt'
archive="$work/dl/libkrunfw-x86_64.tgz"
curl -fsSL -o "$archive" \
  "https://github.com/containers/libkrunfw/releases/download/$libkrunfw_version/libkrunfw-x86_64.tgz"
echo "$libkrunfw_sha256  $archive" | sha256sum -c -
tar xzf "$archive" -C "$work/dl"
cp -a "$work/dl"/lib/x86_64-linux-gnu/libkrunfw.so* "$work/prefix/krun/"

echo '== libkrun, from source'
if [ ! -d "$work/src/libkrun" ]; then
  git clone -q --depth 1 --branch "$libkrun_version" https://github.com/containers/libkrun "$work/src/libkrun"
fi
[ "$(git -C "$work/src/libkrun" rev-parse HEAD)" = "$libkrun_commit" ] || { echo 'setup: libkrun commit changed'; exit 1; }
make -C "$work/src/libkrun"
cp -a "$work/src/libkrun/target/release"/libkrun.so* "$work/prefix/krun/"
ln -sf "libkrun.so.${libkrun_version#v}" "$work/prefix/krun/libkrun.so.1"

echo '== Frame'
export CEF_PATH="$work/cef-store" CARGO_TARGET_DIR="$work/target" KRUN_LIB_DIR="$work/prefix/krun"
(cd "$repo" && cargo build --release -p frame-gate)
(cd "$repo/prototype" && cargo build --release)

echo '== guest image'
"$repo/prototype/build-rootfs.sh" "$work/target/release/engine" \
  "$(echo "$work"/cef-store/*/cef_linux_x86_64)" "$work/root"

echo "Done. Start it with: $repo/prototype/run.sh $work https://example.com"
