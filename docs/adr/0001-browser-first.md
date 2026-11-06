# 0001: Browser first, hardware later

**Status:** accepted, 2026-11-06

**Context.** An OS inside a stock browser tab cannot be hardened, because it trusts the browser and host beneath it. A bootable OS gives the strongest isolation but delivers nothing usable for a long time.

**Decision.** Build a standalone browser that runs on a host OS and owns its compartments. Bootable and phone editions come last and reuse the same images.

**Consequences.** Usable far sooner, and the first product is achievable. A compromised host defeats this edition, and the threat model says so.
