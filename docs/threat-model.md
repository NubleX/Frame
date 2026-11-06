# Threat model

Nothing here has been audited. Each claim is a design goal until a test or an external review backs it.

## Adversaries

| Adversary | In scope |
|---|---|
| Malicious or compromised websites | Yes |
| Browser exploit chains, including zero-days | Yes |
| Trackers and fingerprinting scripts | Yes, within the limits below |
| Malicious add-ons | Yes, limited by the guard and Wasm sandbox |
| Prompt injection against agents | Yes |
| Local network and ISP observers | Partly, depends on route |
| Malware already on the host | **No** in the hosted edition |
| Firmware implants, physical attacks on a running machine | **No** |

## What Frame aims to protect

- The host, from web code. A full engine exploit should end inside a disposable microVM.
- Identities from each other. Compartments share no state.
- The inner side from the outer side. All data crossing the membrane is treated as hostile.
- Host identity from the web: hostname, fonts, GPU, locale, files.

## What Frame does not protect

**A compromised host defeats hosted Frame.** The host can read guest memory and the screen. Only a future Metal edition can address this.

**The Direct route shows your IP.** It hides the machine, not the address. On Windows and macOS hosts, TCP metadata also shows the host OS.

**Bare is not invisible.** Sites can track Frame by IP, cookies and login, like any browser. Compartments and disposables limit that; they do not stop a site recognising a returning identity.

**Hosted Frame is not Tails.** Host swap, hibernation and crash dumps can keep traces of RAM-only sessions.

**Traffic analysis.** An observer can see traffic volume and timing. A long-lived flow to one exit is itself a pattern.

**Small crowd.** Fixed 1920x1080 with software rendering narrows the crowd. Spike 2 measures how far.

## Boundaries

| Boundary | Mechanism | Main risk |
|---|---|---|
| Web code to compartment | Chromium sandbox | Engine exploit, assumed to happen |
| Compartment to host | KVM microVM, jailed VMM | VMM or kernel bug; exposed virtio devices |
| Compartment to inner side | `frame-membrane`, pixels only | Parser or framebuffer handling bug |
| Compartment to network | Gate only, kill switch | Gate compromise |
| Add-on to everything | Wasm sandbox, `frame-policy` | Over-broad grants |
| Agent to outward action | User approval | Approval fatigue |

## Known risks

- **Stale engine.** CEF can trail Chrome stable. A release rule must cap the lag.
- **Log injection.** Shield SIEM shows attacker-controlled URLs. The membrane refuses control and direction characters, CSV export neutralises formulas, and panels must render data as text under strict CSP.
- **Monitoring data.** The event store holds browsing history. It stays local, is off until the user starts it, and redacts credentials. URL query strings are stored as they are, and can hold tokens.
- **Shield SIEM labels are heuristics.** Substring matches give false positives and miss real threats. They are labels, not detections.
- **Honeypot role.** It accepts hostile inbound traffic by design, so it must stay isolated.
- **Dependencies.** Each third-party component is reviewed before use. `gonc` uses public STUN and MQTT servers by default, so it needs self-hosted rendezvous.

## Before making claims

No "Fort Knox", "undetectable" or "Qubes-grade" statement ships before an external audit supports it.
