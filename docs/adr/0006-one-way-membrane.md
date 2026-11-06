# 0006: A one-way membrane, pixels only

**Status:** accepted, 2026-11-06

**Context.** Any compartment may be fully compromised, including our own guest code.

**Decision.** Observations flow in and typed commands flow out, enforced by separate types and decoders. Messages carry no compartment id; the broker stamps it from the channel. The shell receives pixels only.

**Consequences.** A compromised guest can lie about its own traffic, but it cannot impersonate another compartment or reach the inner side with code. Pixels cost CPU and bandwidth. Draw-command streaming may replace them later, and would add a hostile parser.
