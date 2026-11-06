# Roadmap

Durations are rough estimates, not measurements. Revise them after Phase 0.

| Phase | Delivers | Exit gate | Estimate |
|---|---|---|---|
| 0 Spikes | ADRs, workspace, five spikes | All answered | 3-6 weeks |
| 1 Alpha (Linux) | Broker with control API, gate, one web compartment, Aurelia shell | Daily browsing works; invariants 1-4 tested | 2-4 months |
| 2 **Beta, first product** | Identities, Shield SIEM with Suricata, snapshot-forked disposables, routes, RAM-only mode, signed updates | External pentest passed; pilot clients | 5-8 months |
| 3 Add-ons and Roles | Wasm SDK, Profiles, Katarakt analysis, Stagehand agent, own Chromium build, first membrane proofs | Third party ships an add-on; two Profiles run from one image | 4-6 months |
| 4 Agile | macOS, Windows, portable USB app, remote node, roaming mesh, draw-command streaming | Same print on every edition | 6-9 months |
| 5 Metal | Bootable appliance OS on the SteamOS pattern, then phone and car, crowd-following shaping research | Hardware research confirmed, Pixel first | 12+ months |

## Phase 0 spikes

1. CEF in libkrun, off-screen frames in a Rust window. See `spikes/001-cef-in-microvm/`.
2. Print: JA4, JA4H, HTTP/2 and JS surface of CEF against stock Chromium of the same version. Check the default User-Agent has no CEF token. Packet capture must show no Google calls.
3. Gate path through tun2socks, kill switch, throughput.
4. Response stripping: which headers are safe to remove, native CEF handlers versus a patch, speed cost.
5. Servo renders Aurelia panels. Fallback is a Rust-native UI.

## Overkill track

| Approach | Complexity | Phase | Why it earns its place |
|---|---|---|---|
| Suricata in the gate | 5/10 | 2 | Real detection behind Shield SIEM |
| Snapshot-forked compartments | 7/10 | 2 | A fresh VM per link. Forks must reseed RNG state |
| Verified membrane | 9/10 | 3 onward | Proofs for the trusted core, using Verus |
| Roaming compartments over a mesh | 8/10 | 4 | Sessions move between devices, using iroh or gonc |
| Draw-command streaming | 9/10 | 4 | Lower CPU and bandwidth than pixels, but adds a hostile parser |
| Crowd-following traffic shaping | 9/10 | 5 | Research only |

## Open items

- Legal check on H.264 and AAC before Beta. Codec support is visible to sites, and shipping them in a paid product can need patent licences.
- Trademark check on "Frame" before the first paid release. Valve sells a product called Steam Frame.
- A maximum CEF lag behind Chrome stable, as a release rule.
- A build machine. Chromium cannot be built on the current dev box.
