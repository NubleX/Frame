# Frame

Standalone browser with a boring outside. Read `docs/architecture.md` first, then the ADR for whatever you are changing.

## Commands

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

Fuzzing needs `cargo-fuzz` and a nightly toolchain: `cargo +nightly fuzz run membrane`.

Proofs need [Kani](https://github.com/model-checking/kani). Run `cargo kani` inside `crates/frame-gate` or `crates/frame-membrane`. See `docs/proofs.md`.

## The four invariants

Every change must keep these. Each needs a test that fails if it breaks.

1. Nothing on the inner side is observable from the web. Add-ons attach to the membrane, never to a page.
2. All editions produce the same print in a release, matching the same Chromium version.
3. Web compartments have no route except the gate. If the gate dies, traffic stops.
4. No host identifier enters a compartment: hostname, fonts, GPU, locale, files.

## Rules

- All data from the outer side is hostile, including from our own guest code. Parse strictly and fail closed.
- One owner per state: the broker owns the compartment table, the gate owns routes, `frame-siem` owns the event store.
- Never edit outbound bytes. Header stripping is response side only.
- Do not add masking: no canvas noise, no page patches, no spoofed values. Frame has nothing to hide. Katarakt's page patches must not ship.
- Do not claim to be another product, in a User-Agent or anywhere else.
- `unsafe` is forbidden workspace-wide. A crate that truly needs it must justify the exception in an ADR.
- No `unwrap`, `expect` or `panic` outside tests. Security claims in docs must match what a test or audit shows.

## How to work

- Think first: state assumptions, name what is unclear, and ask rather than guess.
- Keep it simple: write the least code that solves the problem, with nothing speculative.
- Change only what the task needs, and clean up only your own mess.
- Work to a check: define the test that proves it, then make it pass.
