# 0004: libkrun microVM compartments

**Status:** accepted, 2026-11-06. Spike 1 passed: CEF runs in a 1 GB guest

**Context.** Compartments need hardware isolation but must embed in an app and start fast. libkrun is a Rust library with KVM on Linux and Hypervisor.framework on macOS. crosvm has stronger device sandboxing and is the VMM in Android's stack.

**Decision.** Use libkrun behind the `frame-vmm` trait. Jail every VMM with no host paths, and no network except the gate's. Use software rendering, with no GPU passthrough.

**Consequences.** One library across Linux and macOS, with no GPU attack surface. libkrun's own docs warn that the VM boundary does not restrict host resources exposed through devices, so the jail is required and not optional. Upstream has no Windows backend, so Windows is decided in Phase 4. The trait keeps crosvm available for Metal and phone.
