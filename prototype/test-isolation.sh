#!/usr/bin/env bash
# Proves a compartment cannot see past itself. Real listeners run on the host;
# a real guest tries to reach them, and the jailed VMM is inspected from
# outside. Exits non-zero on any leak. Usage: test-isolation.sh <work-dir>
set -u
work=$1
root="$work/root-probe"
run="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/frame-test-$$"
fails=0
pass() { echo "PASS $1"; }
fail() { echo "FAIL $1"; fails=$((fails + 1)); }

lan=$(ip -4 -o addr show scope global | awk '{print $4}' | cut -d/ -f1 | head -1)
www="$work/probe-www"
mkdir -p "$www" "$run"
echo host-reached > "$www/index.html"
(cd "$www" && exec python3 -I -m http.server 18741 --bind 127.0.0.1) > /dev/null 2>&1 &
loopback=$!
(cd "$www" && exec python3 -I -m http.server 18742 --bind "$lan") > /dev/null 2>&1 &
local_net=$!
trap 'kill $loopback $local_net 2>/dev/null; rm -rf "$run" "$www" "$root"' EXIT
sleep 1

rm -rf "$root"
mkdir -p "$root"/{bin,proc,sys,dev,tmp,etc,lib64,usr/lib/x86_64-linux-gnu}
cp "$(command -v busybox)" "$root/bin/busybox"
ln -s busybox "$root/bin/sh"
for library in $(ldd "$(command -v busybox)" 2>/dev/null | awk '/=> \//{print $3}'); do
  cp -L "$library" "$root/usr/lib/x86_64-linux-gnu/"
done
cp -L /lib64/ld-linux-x86-64.so.2 "$root/lib64/" 2>/dev/null
cat > "$root/bin/probe" <<PROBE
#!/bin/sh
B=/bin/busybox
\$B mount -t tmpfs t /tmp; \$B mkdir -p /tmp/b; \$B --install -s /tmp/b; export PATH=/tmp/b:/bin
mount -t proc proc /proc 2>/dev/null; mount -t sysfs sysfs /sys 2>/dev/null
# A real card has a device behind it. The kernel's dummy0 stub has none.
echo "NICS \$(ls -d /sys/class/net/*/device 2>/dev/null | wc -l)"
echo "ROUTES \$(ip route 2>/dev/null | wc -l)"
echo "PCI \$(ls /sys/bus/pci/devices 2>/dev/null | wc -l) USB \$(ls /sys/bus/usb/devices 2>/dev/null | wc -l)"
echo "CMDLINE_TSI \$(grep -c tsi_hijack /proc/cmdline)"
echo "LOOPBACK [\$(wget -T 3 -q -O - http://127.0.0.1:18741/ 2>/dev/null)]"
echo "LAN [\$(wget -T 3 -q -O - http://$lan:18742/ 2>/dev/null)]"
echo "PUBLIC [\$(wget -T 3 -q -O - http://1.1.1.1/ 2>/dev/null | head -c 20)]"
touch /root-write-test 2>/dev/null && echo ROOT_WRITE yes || echo ROOT_WRITE no
sleep 4
echo PROBE_DONE
PROBE
chmod +x "$root/bin/probe"

LD_LIBRARY_PATH="$work/prefix/krun" "$work/target/release/launcher" "$root" 256 2 \
  "$work/prefix/krun" "$run" -- /bin/probe > "$work/isolation.log" 2>&1 &
launcher=$!
sleep 2
vmm=$(pgrep -P "$launcher" | head -1)
if [ -n "$vmm" ]; then
  [ "$(readlink /proc/$vmm/ns/net)" != "$(readlink /proc/self/ns/net)" ] \
    && pass "VMM has its own empty network namespace" || fail "VMM shares the host network"
  [ "$(readlink /proc/$vmm/ns/pid)" != "$(readlink /proc/self/ns/pid)" ] \
    && pass "VMM cannot see host processes" || fail "VMM shares host PIDs"
  mounts=$(wc -l < "/proc/$vmm/mountinfo" 2>/dev/null || echo 0)
  grep -q ' /home\| /etc\| /usr ' "/proc/$vmm/mountinfo" 2>/dev/null \
    && fail "VMM can see host files" || pass "VMM sees no host files ($mounts mounts)"
  [ "$(awk '/^CapEff/ {print $2}' "/proc/$vmm/status")" = 0000000000000000 ] \
    && pass "VMM holds no capabilities" || fail "VMM still holds capabilities"
  [ "$(awk '/^NoNewPrivs/ {print $2}' "/proc/$vmm/status")" = 1 ] \
    && pass "VMM cannot gain privileges" || fail "no_new_privs is off"
else
  fail "VMM did not start"
fi
wait "$launcher"
log="$work/isolation.log"
grep -q PROBE_DONE "$log" || fail "guest did not finish: $(tail -3 "$log")"
line() { grep -m1 "^$1" "$log"; }
[ "$(line NICS)" = "NICS 0" ] && pass "guest has no network card" || fail "guest NIC: $(line NICS)"
[ "$(line ROUTES)" = "ROUTES 0" ] && pass "guest has no routes" || fail "$(line ROUTES)"
[ "$(line 'PCI')" = "PCI 0 USB 0" ] && pass "guest has no PCI or USB devices" || fail "$(line PCI)"
[ "$(line CMDLINE_TSI)" = "CMDLINE_TSI 0" ] && pass "TSI is off" || fail "TSI is on"
[ "$(line LOOPBACK)" = "LOOPBACK []" ] && pass "host loopback unreachable" || fail "$(line LOOPBACK)"
[ "$(line LAN)" = "LAN []" ] && pass "host LAN address unreachable" || fail "$(line LAN)"
[ "$(line PUBLIC)" = "PUBLIC []" ] && pass "no direct internet, gate is the only way" || fail "$(line PUBLIC)"
[ "$(line ROOT_WRITE)" = "ROOT_WRITE no" ] && pass "guest root is read-only" || fail "guest wrote to its root"
[ -e "$root/root-write-test" ] && fail "guest write reached the host"
echo "failures: $fails"
exit "$fails"
