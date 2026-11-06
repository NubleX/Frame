# Spike 1: CEF in a microVM

**Question.** Can a CEF engine run inside a libkrun microVM and show a real page in a Rust window on the host?

**Result: passed.** See [RESULTS.md](RESULTS.md).

**Pass.** A real page from CEF in a microVM appears in the Rust window, confirmed by screenshot. Also record guest RAM use, start time and CPU use.

**Fail is useful.** If libkrun cannot do this, try crosvm next. See ADR 0004.

## Steps

1. Run `./check-prereqs.sh` to see what is missing.
2. Fetch the prebuilt `libkrunfw` release and build `libkrun` from source, since neither is packaged here.
3. Build a minimal Linux guest image holding the CEF runtime and a small `cef-rs` program using off-screen rendering.
4. Send frames from guest to host over vsock.
5. Show them in a small Rust window on the host.
6. Measure, take a screenshot, and write the result in this folder.

## Constraints

The dev box has 7 GB of RAM, and each guest gets 1 GB. Nothing is installed system-wide: everything lives in a local work folder. Chromium itself is not built here.
