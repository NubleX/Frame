# Proofs

Machine-checked with [Kani](https://github.com/model-checking/kani) on the shipped Rust, not on a model of it. Unlike a test, a proof covers every value of its inputs. Run `cargo kani` inside the crate.

## Proven

| Crate | Property | Covers |
|---|---|---|
| `frame-gate` | Each address is classified exactly as the forbidden list says | All 2^32 IPv4 addresses, all 2^128 IPv6 addresses |
| `frame-gate` | `PublicAddr`, the only thing that can open a connection, never holds a forbidden address or port zero | All addresses and ports |
| `frame-gate` | An accepted host name holds only safe bytes | Names up to 8 bytes |
| `frame-membrane` | No kind belongs to both directions | All 256 kinds |
| `frame-membrane` | A read never leaves the buffer | Any size requested, buffers up to 8 bytes |
| `frame-membrane` | A frame stays inside the bytes received and the size limit | Inputs up to 10 bytes |
| `frame-membrane` | Modifiers round-trip and unknown bits are refused | All 256 values |
| `frame-membrane` | A typed character is never a control or direction character | All `u32` values |

The address rules are written a second way in the proofs, as bit arithmetic, so a mistake in the code cannot also hide in the proof. Deliberately broken code was caught: an allowed loopback range and a one-off range error in the gate; a read bound off by one, overlapping kinds and allowed control characters in the membrane.

## Not proven

- **The full decoders**, `decode_command` and `decode_observation`. Kani did not finish on them: 9 minutes at 15 bytes, and 100 seconds at 10 bytes, both timed out. It has to model the string handling. They are covered by tests only. The fuzz target exists but has never been run.
- **So these are tested, not proven:** that hostile bytes cannot crash a decoder, that the decoders enforce direction, and that a message has one encoding.
- **SOCKS parsing** in `frame-gate`, and anything outside these two crates: the VMM, KVM, Chromium, the jail.

A proof shows the code meets the stated property. It says nothing about a property nobody stated.

## Next

Split the fixed-size fields from the text handling in the decoders, so the checker can finish on each part. Verus is the option for proofs without a size limit, but needs the code rewritten in its dialect.
