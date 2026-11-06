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
| Burn | Closing the window ends it. Everything the guest wrote was in guest RAM |

The guest image also carries Chromium's flags and no host resolver, so every machine runs the same browser.

## Use

Needs `/dev/kvm`, libkrun, libkrunfw and a CEF binary, set up as in [Spike 1](../spikes/001-cef-in-microvm/README.md). `<work>` holds `prefix/krun`, `target/release` and `root`.

```bash
CEF_PATH=<work>/cef-store CARGO_TARGET_DIR=<work>/target KRUN_LIB_DIR=<work>/prefix/krun cargo build --release
```

Run the same build from the repo root with `-p frame-gate`, then:

```bash
./build-rootfs.sh <work>/target/release/engine <cef-dir> <work>/root
./run.sh <work> https://example.com
./test-isolation.sh <work>
```

| Key | Does |
|---|---|
| Ctrl+L, then Enter | Go to an address, shown in the window title. Esc cancels |
| Alt+Left, Alt+Right | Back, forward |
| F5 | Reload |

There is no search engine built in: what you type is where you go.

## Measured

On one Linux x86_64 laptop, in a 1 GiB guest.

- `test-isolation.sh`: a real guest cannot reach listeners on host loopback or the LAN address, or reach the internet directly. The VMM shares no network or PIDs with the host, sees no host files, and holds no capabilities.
- Through the browser, the gate refused `192.168.1.185`, `127.0.0.1` and `localhost`.
- Address entry, wheel, Page Down, back, forward, clicking links and typing a search into a real page all worked.
- The guest kernel reports seccomp filters on sandboxed processes, including the network service, and no browser process as root.
- With the gate killed, the next page did not load within 5 seconds. It hung rather than showing an error page.

## Not done yet

- **No seccomp filter on the VMM.** The jail limits what it can see, not which system calls it can make.
- **The gate is a host process**, not the planned gate microVM. It runs unjailed, because it needs the network.
- **A proxy, not a virtual NIC.** Chromium reaches the gate through SOCKS5. UDP does not pass, so there is no QUIC or WebRTC, and that changes the print. tun2socks in the guest is the planned fix, in Spike 3.
- **A host with a public address on its own interface** is not refused. Laptops behind NAT are covered by the private ranges.
- **Built and tested on Linux only.** Frame is designed to run on any host, and [ADR 0009](../docs/adr/0009-one-guest-thin-host.md) keeps the per-OS code to three seams, but none of the others is written yet. An OS with no jail refuses to start. On Linux the jail needs unprivileged user namespaces, which some AppArmor setups restrict.
- **Missing browser features:** tabs, downloads, clipboard, file upload and Ctrl shortcuts. Select dropdowns are untested and likely need their popup drawn.

"Isolated" here means no path exists in this code. It does not remove bugs in KVM, libkrun or the kernel. Nothing has been audited.
