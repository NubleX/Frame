# Frame

A standalone browser with a boring outside. Your frame into the data world of Aurelia.

To the web, Frame is an ordinary embedded Chromium on Linux. Web code runs on virtual hardware inside a disposable microVM, and your machine only lends CPU, RAM, a network link and one window. On the inner side, facing you and never the web, anything can be added.

**Status: pre-alpha foundation.** There is no working browser yet. This repository holds the architecture, the threat model and the tested core libraries. No security claim here has been audited.

## How it works

```
 THE WEB  sees only: ordinary embedded Chromium on Linux
 ------------------------------------------------------------ OUTER
 gate microVM     tun2socks, DNS, routes, kill switch
 web-* microVMs   CEF engine, all-virtual hardware
 ------------------------------------------------------------ MEMBRANE
 frame broker     strict parsers; observations in, typed commands out
 ------------------------------------------------------------ INNER
 Aurelia shell | Shield SIEM | add-ons | agents
 ------------------------------------------------------------
 HOST OS          lends CPU, RAM, NIC, one window. No web code.
```

Frame does not mask anything. Privacy browsers stand out through how hard they hide. Frame ships the bare minimum, and the protection sits around the engine, where the web cannot see it.

One skeleton, many machines: a Frame Profile turns the same base into a browser, marketing tool, dev environment, honeypot, headless server or kiosk.

## Read more

- [Architecture](docs/architecture.md)
- [Threat model](docs/threat-model.md): what Frame does and does not protect against
- [Roadmap](docs/roadmap.md)
- [Decisions](docs/adr/)
- [Add-on interface](spec/wit/addon.wit)

## Build and test

```bash
cargo test --workspace
```

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

## Core crates

| Crate | Owns |
|---|---|
| `frame-policy` | The ordered default-deny guard |
| `frame-membrane` | The strict one-way wire format |
| `frame-vmm` | The VMM boundary and fixed virtual hardware |
| `frame-compartment` | Idempotent compartment lifecycle and crash recovery |
| `frame-gate` | The only way out of a compartment. It cannot connect to the host or the local network |
| `frame-siem` | Shield SIEM event store, ported from the browser extensions |
| `frame-profile` | Role manifests |

## Licence

GPL-3.0. See [LICENSE](LICENSE).
