# 0005: One gate, and response-side stripping only

**Status:** accepted, 2026-11-06, pending Spikes 3 and 4

**Context.** All traffic must pass one controlled point. Stripping headers helps safety, but editing what the engine sends changes the print, and intercepting TLS would replace Chromium's TLS print with ours.

**Decision.** A gate microVM runs xjasonlyu/tun2socks, chosen for its memory-safe Go and gVisor stack. It is the only VM with an uplink. Direct is the default route. Header stripping applies to responses only, inside the engine.

**Consequences.** One place for routes, monitoring and the kill switch, and outbound bytes stay stock. The Direct route shows the host IP. Host TCP metadata shows on non-Linux hosts until a privileged wire mode exists.
