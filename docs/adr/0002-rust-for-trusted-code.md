# 0002: Rust for trusted code

**Status:** accepted, 2026-11-06

**Context.** The broker, shell and parsers handle hostile input and must be fast. Memory-safety bugs here defeat everything else.

**Decision.** All trusted code is Rust with `unsafe` forbidden workspace-wide. Go stays inside guests, where it is not trusted. C and C++ exist only in third-party code inside VMs or the VMM.

**Consequences.** Memory safety at native speed, and libkrun, cef-rs, Servo, Wasmtime and iroh are all Rust-friendly. Calling libkrun and CEF needs FFI, so a thin binding crate will need a justified exception to the `unsafe` ban.
