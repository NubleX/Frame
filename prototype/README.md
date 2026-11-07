# Barebone browser

A working browser in an isolated microVM: browse, click, scroll, type, search. It is a prototype on the way to Phase 1, built so isolation comes from code, not from host setup.

## How it is isolated

| Layer | What the code does |
|---|---|
| Guest | No network card, no routes, no TSI, no PCI or USB devices, read-only root |
| Browser | Gives up root as its first act and refuses to run if it cannot. Chromium's sandbox is on, including for the network service |
| Way out | One channel to the gate. If the gate dies, traffic stops: there is nothing to fall back to |
| Gate | `crates/frame-gate`, safe Rust. Only a `PublicAddr` can open a connection, and that type cannot hold the host, the LAN or any other non-public address |
| Inner side | `crates/frame-membrane`. Pixels in, a closed set of typed commands out |
| VMM | Jails itself before any VM code runs: own network, mount, PID, IPC and UTS namespaces, a root holding only what it needs, no capabilities, no new privileges. If a step fails, the compartment does not start |
| Burn | The window owns the gate and the VM through a pipe on their stdin. However the window ends, even `kill -9`, the kernel closes the pipe and they exit. If the VM or gate dies, the window closes. Everything the guest wrote was in guest RAM |

The guest image also carries Chromium's flags and no host resolver, so every machine runs the same browser.

## Lean image

Chromium is the prebuilt CEF binary, unchanged. Its 89 libraries are cut to 26 real ones by `stub-libs.sh`, without building anything: libraries a screenless browser never calls become stubs, and whatever only they needed is deleted. That removes Kerberos, GnuTLS, Avahi, real CUPS and X11, and takes the libraries from 38 MB to 16 MB.

The list came from booting with stubs that report any call. GLib, D-Bus and udev had to stay. If a page ever reaches a stub, that process stops and `STUB_CALLED <library> <symbol>` appears in the guest log, so the fix is to take that library off the list in `build-rootfs.sh`.

The image is still 345 MB, because `libcef.so` itself is most of it. Only a real build shrinks that.

## Use

Needs `/dev/kvm`, Rust, a C toolchain, `cmake`, `ninja`, `git`, `curl`, `busybox`, and Chromium's shared libraries on the build machine. `setup.sh` fetches pinned, hash-checked dependencies and builds everything into one folder. Nothing is installed system-wide.

```bash
./setup.sh <work>
```

```bash
./run.sh <work> https://example.com
```

```bash
./test-isolation.sh <work>
```

| Key | Does |
|---|---|
| Click the bar, or Ctrl+L | Edit the address, then Enter. Esc cancels |
| `<` `>` `R`, or Alt+Left, Alt+Right, F5 | Back, forward, reload |
| Ctrl with `+`, `-` or `0`, or Ctrl+wheel | Zoom in, out, reset |

There is no search engine built in: what you type is where you go. Chromium's internal pages work, such as `about:about`, `chrome://version` and `chrome://sandbox`.

The page is always rendered at 1920x1080, whatever the window size, so the window cannot affect what pages see. It is scaled to fit, with filtering, and never enlarged. On a smaller screen that makes text small, which is what zoom is for. Zoom is visible to pages, as in any browser: `devicePixelRatio` and the page width change.

The bar is drawn on the host, so a page cannot paint over it. Anything outside ASCII shows as `?`, which also exposes look-alike letters in an address.

## Measured

On one Linux x86_64 laptop, in a 1 GiB guest.

- `test-isolation.sh`: a real guest cannot reach listeners on host loopback or the LAN address, or reach the internet directly. The VMM shares no network or PIDs with the host, sees no host files, and holds no capabilities.
- Through the browser, the gate refused the host's LAN address, `127.0.0.1` and `localhost`.
- Address entry, wheel, Page Down, back, forward, clicking links and typing a search into a real page all worked.
- The guest kernel reports seccomp filters on sandboxed processes, including the network service, and no browser process as root.
- On the lean image, no stub was reached across idarti.com, Wikipedia, a DuckDuckGo search, GitHub, or a page using audio, canvas and WebGL. Real video was not tried.
- Closing the window, and `kill -9` on the window, the VM or the gate, each left no process running. After `kill -9` on the window, one small socket folder stays behind.
- `chrome://sandbox` reports "You are adequately sandboxed". Only Yama ptrace protection is missing, because the guest kernel does not have it.
- `chrome://version` shows a stock User-Agent with no CEF token.
- With the gate killed, the next page did not load within 5 seconds. It hung rather than showing an error page.

## Not done yet

- **No seccomp filter on the VMM.** The jail limits what it can see, not which system calls it can make.
- **The gate is a host process**, not the planned gate microVM. It runs unjailed, because it needs the network.
- **A proxy, not a virtual NIC.** Chromium reaches the gate through SOCKS5. UDP does not pass, so there is no QUIC or WebRTC, and that changes the print. tun2socks in the guest is the planned fix, in Spike 3.
- **A host with a public address on its own interface** is not refused. Laptops behind NAT are covered by the private ranges.
- **Built and tested on Linux only.** Frame is designed to run on any host, and [ADR 0009](../docs/adr/0009-one-guest-thin-host.md) keeps the per-OS code to three seams, but none of the others is written yet. An OS with no jail refuses to start. On Linux the jail needs unprivileged user namespaces, which some AppArmor setups restrict.
- **Missing browser features:** tabs, downloads, clipboard, file upload, and Ctrl shortcuts other than zoom. The address bar has no text cursor: it edits from the end. Select dropdowns are untested and likely need their popup drawn.

"Isolated" here means no path exists in this code. It does not remove bugs in KVM, libkrun or the kernel. Nothing has been audited.
