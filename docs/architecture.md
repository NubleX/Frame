# Architecture

## Goal

Frame is a standalone browser that runs on a host OS and can later run on bare hardware. To the web it is an ordinary embedded Chromium on Linux. Web code runs on virtual hardware in a disposable compartment. The inner side, which faces the user, can be extended without the web ever seeing it.

## Principle: nothing to hide

Privacy browsers mask, and their masking is itself a print. Frame takes the opposite route. The engine is bare, with no privacy features to detect, no accounts and no browser identity. Protection sits around the engine, in the compartment and the gate, where the web cannot see it.

The persona is true by construction, not a spoof. Frame really is embedded Chromium on Linux with fixed virtual hardware, the same family as the CEF software in many desktop products.

Frame does not claim to be another product. A forged Googlebot fails reverse DNS checks, and naming a vendor in the User-Agent contradicts everything else a site can see. Both are loud, which is the opposite of the goal.

## Layers

```
 THE WEB  sees only: ordinary embedded Chromium on Linux
 ------------------------------------------------------------ OUTER
 gate microVM     tun2socks (gVisor stack), DNS, routes, kill switch
 web-* microVMs   CEF engine, real Linux TCP/IP, all-virtual
                  hardware, native request and response taps
 ------------------------------------------------------------ MEMBRANE
 frame broker     Rust. Strict parsers. Observations flow in;
                  only typed commands flow out
 ------------------------------------------------------------ INNER
 Aurelia shell | Shield SIEM | add-ons (Wasm) | agents
 ------------------------------------------------------------
 HOST OS          lends CPU, RAM, NIC, one window. No web code.
```

## Invariants

1. Nothing on the inner side is observable from the web.
2. All editions produce the same print in a release, matching the same Chromium version.
3. Web compartments have no route except the gate. If the gate dies, traffic stops.
4. No host identifier enters a compartment: hostname, fonts, GPU, locale, files.

| Invariant | Enforced today | Still to build |
|---|---|---|
| 1 | No page capability exists in `frame-policy` or the WIT interface | Print test with add-ons loaded |
| 2 | Fixed `CANONICAL_HARDWARE` in `frame-vmm`; Profiles have no print keys | Print gate against stock Chromium |
| 3 | `VmSpec` derives the network from the kind; web cannot have an uplink | Real gate and kill switch test |
| 4 | `VmSpec` has no field for host paths or devices | Boot test probing the guest |

## Ownership

| State | Owner |
|---|---|
| Compartment table | Broker, via `frame-compartment` |
| Routes | Gate |
| Security events | `frame-siem` |
| Policy decisions | `frame-policy`, one guard for every caller |

The guard answers "may you do this?". The owning service still clamps what the caller may do. The broker's control API is the product. The Aurelia shell is only its first client, and a headless server is Frame with no shell.

## The membrane

The membrane is the only path between the outer and inner sides, and any compartment may be compromised.

- `frame-membrane` is strict: one version, known kinds, hard limits, no trailing bytes, no control or direction characters in text.
- Flow is one way by type. The outer side sends observations and the inner side sends commands. Each decoder refuses the other's kinds.
- Messages name no compartment. The broker stamps the id from the channel, so a guest cannot impersonate another.

**Pixels only.** The shell shows a framebuffer and sends input through a virtual keyboard and mouse in the guest. Web code never reaches the inner side.

## Compartments

A compartment is a microVM per identity, disposable by default, run by libkrun behind the `frame-vmm` trait. Each VMM runs jailed with no host paths and, except the gate's, no network, because libkrun's documentation warns that the VM boundary does not restrict host resources exposed through devices.

Lifecycle is desired-state: intent is recorded first, then the system converges. Repeated operations are harmless and a restart finishes interrupted work.

## Network

A web compartment runs real Linux TCP/IP on a virtual NIC that reaches only the gate. The gate terminates that traffic with tun2socks and forwards it by route.

| Route | Sites see | Notes |
|---|---|---|
| Direct, the default | Host IP | Plain TCP and UDP/QUIC. The shell shows that the IP is visible. On Windows or macOS hosts, TCP metadata shows the host OS |
| Own exit | Your node | TLS or QUIC on 443 to a Frame node you own |
| Tor | A Tor exit | Opt-in add-on. Hides more, looks less boring |

Outbound bytes are never edited, because that would change the print. Header stripping happens on responses only, inside the engine, which avoids TLS interception. Spike 4 selects the mechanism and the header list.

## Engine

The base is CEF, driven from Rust through `cef-rs`. Three stages lead away from it:

1. A pinned CEF binary inside the microVM.
2. A reproducible Chromium build with a small Frame patch set.
3. A `frame-engine` trait. Any part or a whole engine, Servo included, may replace Chromium only by passing the print gate.

The print is not frozen. It tracks upstream through CEF, and the release gate compares Frame with the same Chromium version.

## Inner side

The Aurelia shell is a Rust window, with Servo rendering panels from local signed files using the Aurelia OS design tokens. Add-ons are Wasm components, defined in `spec/wit/`, and every call passes the guard.

First add-ons:

- **Shield SIEM.** Ported to `frame-siem`. Fed by engine taps and gate flows, so it also sees DNS and non-HTTP traffic.
- **ID-Spoofer.** Runs as a binary inside the gate guest for tunnels and wire personas, with root only inside that guest.
- **Katarakt.** Its suspicious-script patterns feed analysis, and its probes become a print test. Its canvas and WebGL page patches do not ship, because pages can see them.

## Roles

A Frame Profile is a small manifest choosing add-ons, compartments, routes and shell. `frame-profile` refuses unknown keys, and no key exists for anything the web can see, so a role cannot change the print. Signature checking arrives with the update system.

| Role | What the Profile sets |
|---|---|
| Browser | Identities, disposables, Shield SIEM |
| Marketing | Identities per campaign, monitoring add-ons, agent |
| Dev | Linux app compartments, disposable test browsers |
| Honeypot | Decoy compartments, inbound listener, capture |
| Headless | No shell, control API only |
| Kiosk or car | Fixed shell, single purpose. Infotainment only, never driving functions |

Honeypot is the only role that accepts inbound traffic. Its inbound compartments must be disposable and have no path to others.

## Later: Frame Metal

The bootable edition follows the SteamOS appliance pattern: one read-only image, atomic updates, a single full-screen compositor. SteamOS is a model and not a base, since it has no compartment model. See the [roadmap](roadmap.md).
