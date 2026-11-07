# Own Chromium build

Optional, and not needed to run Frame. `stub-libs.sh` already cuts the guest from 89 libraries to 26 real ones without a build. What only a real build can do is shrink `libcef.so` itself and remove the stubs, and stop the image depending on the libraries of whoever built it.

The goal is an engine that needs only what every Linux has: `libc`, `libm`, `libdl`, `libpthread`, `libgcc_s` and the loader. Frame's host programs already meet that. `check-libs.sh` is the gate:

```bash
./check-libs.sh <work>/target/release/frame-gate <work>/target/release/viewer <cef-dir>/libcef.so
```

Today `libcef.so` fails it, needing 26 more libraries. That is expected until this build exists.

## Building

Not done yet. It needs a bigger machine than the dev laptop: Chromium's docs ask for at least 100 GB of free disk, and 8 GB of RAM only with 32 GB of swap. More than 16 GB of RAM is what they recommend.

1. Follow CEF's `automate-git.py` for branch 156 with `args.gn` from here. Chromium's source comes from Google's git hosts, at build time only.
2. Run `check-libs.sh` on the new `libcef.so`.
3. Rebuild the guest with `build-rootfs.sh` and run `test-isolation.sh`.
4. Pass the print gate from Spike 2 against stock Chromium of the same version.

## Unknowns

- CEF may need some of these libraries itself, for example X11 or GLib. Only a build shows that. Expect to keep a few at first.
- A smaller build can change what pages see, such as audio devices. The print gate decides what may be removed, not this list.
- H.264 and AAC stay off here. They need the legal check in the roadmap first.
