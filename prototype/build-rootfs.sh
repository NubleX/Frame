#!/usr/bin/env bash
# Builds the bare guest root: the engine, a stripped CEF and only the shared
# libraries they need. Usage: build-rootfs.sh <engine> <cef-dir> <out>
set -euo pipefail
engine=$1 cef=$2 out=$3

rm -rf "$out"
mkdir -p "$out"/{frame,bin,proc,sys,dev,tmp,etc,lib64,usr/lib/x86_64-linux-gnu,usr/share/fonts}

install -m755 "$engine" "$out/frame/engine"
cp "$cef/libcef.so" "$out/frame/libcef.so"
strip --strip-all "$out/frame/libcef.so"
cp "$cef"/libvk_swiftshader.so "$cef"/libvulkan.so.1 "$cef"/vk_swiftshader_icd.json \
   "$cef"/v8_context_snapshot.bin "$cef"/icudtl.dat "$cef"/resources.pak \
   "$cef"/chrome_100_percent.pak "$cef"/chrome_200_percent.pak "$out/frame/"
mkdir -p "$out/frame/locales"
cp "$cef/locales/en-US.pak" "$out/frame/locales/"

for library in $(LD_LIBRARY_PATH="$cef" ldd "$cef/libcef.so" "$engine" | awk '/=> \//{print $3}' | sort -u); do
  case "$library" in "$cef"/*) continue ;; esac
  cp -L "$library" "$out/usr/lib/x86_64-linux-gnu/"
done
cp -L /lib64/ld-linux-x86-64.so.2 "$out/lib64/"

# Chromium loads NSS crypto modules at runtime, so ldd cannot see them.
libs=/usr/lib/x86_64-linux-gnu
for module in libsoftokn3 libfreebl3 libfreeblpriv3 libnssckbi libnssdbm3; do
  cp -L "$libs/$module.so" "$out$libs/"
  [ -f "$libs/$module.chk" ] && cp -L "$libs/$module.chk" "$out$libs/"
  for library in $(ldd "$libs/$module.so" | awk '/=> \//{print $3}'); do
    cp -Ln "$library" "$out$libs/" 2>/dev/null || true
  done
done

# A small busybox for the guest entry script, with its own needs.
cp "$(command -v busybox)" "$out/bin/busybox"
for applet in sh mount mkdir sleep cat; do ln -s busybox "$out/bin/$applet"; done
for library in $(ldd "$(command -v busybox)" | awk '/=> \//{print $3}'); do
  cp -Ln "$library" "$out/usr/lib/x86_64-linux-gnu/" 2>/dev/null || true
done

# One fixed font set, never the host's list.
cp -r /usr/share/fonts/truetype/dejavu "$out/usr/share/fonts/" 2>/dev/null || true
mkdir -p "$out/etc/fonts"
cat > "$out/etc/fonts/fonts.conf" <<'CONF'
<?xml version="1.0"?>
<!DOCTYPE fontconfig SYSTEM "fonts.dtd">
<fontconfig><dir>/usr/share/fonts</dir><cachedir>/tmp/fontconfig</cachedir></fontconfig>
CONF
# No resolver here: names go to the gate, so the guest never learns the host's.
echo 'hosts: files' > "$out/etc/nsswitch.conf"
echo 'root:x:0:0:root:/tmp:/bin/sh' > "$out/etc/passwd"

cat > "$out/bin/start" <<'SH'
#!/bin/sh
# Keep scratch data in guest RAM instead of writing through to the host.
mkdir -p /dev/shm
mount -t tmpfs -o size=128m tmpfs /dev/shm
mount -t tmpfs -o size=128m tmpfs /tmp
export LD_LIBRARY_PATH=/frame
# The flags live in the image, so every machine runs the same browser.
exec /frame/engine --ozone-platform=headless --disable-gpu --disable-gpu-compositing \
  --proxy-server=socks5://127.0.0.1:1080 '--proxy-bypass-list=<-loopback>' \
  --disk-cache-size=33554432 "$@"
SH
chmod 755 "$out/bin/start"
du -sh "$out"
