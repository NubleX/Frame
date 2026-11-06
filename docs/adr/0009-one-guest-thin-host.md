# 0009: One guest, thin host

**Status:** accepted, 2026-11-07

**Context.** Frame must run on any host OS, and isolation must come from Frame's own code, not from tools or settings a particular machine happens to have.

**Decision.** The web only ever meets one thing: the same Linux guest image, whatever the host. Everything else is safe Rust that does not care about the OS: the gate's rules, the membrane, the policy. What is written per host OS is kept to three small seams:

| Seam | Linux today | What other OSes need |
|---|---|---|
| Hypervisor, behind `frame-vmm` | libkrun on KVM | macOS: libkrun on Hypervisor.framework. Windows: libkrun does not run there, so a backend on Windows Hypervisor Platform must be chosen |
| Self-jail, `jail::enter` | Namespaces, private root, no capabilities | The OS's own sandbox, such as App Sandbox or AppContainer |
| Channel, `frame_gate::Channel` | Unix sockets | Whatever carries bytes to the guest there |

An OS with no jail refuses to start. A compartment never runs unjailed.

**Consequences.** Porting Frame means writing three seams, not a browser. Only Linux is built and tested so far, so "runs everywhere" is a goal until each host passes `test-isolation.sh` or its equivalent. Two limits remain. Arm hosts need an Arm guest, so identical prints there must be proven, not assumed. And the gate's connections leave through the host's TCP stack, so TCP metadata can still show the host OS, as the threat model says.
