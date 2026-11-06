# 0007: Wasm add-ons and Frame Profiles

**Status:** accepted, 2026-11-06

**Context.** Frame must become anything: browser, marketing tool, dev environment, honeypot, headless server, car. Normal browser extensions can touch pages and are visible to them.

**Decision.** Add-ons are Wasm components with WIT interfaces that attach to the membrane and have no page access. A Frame Profile selects role, add-ons, compartments, routes and shell. The broker's control API is the product, and the shell is one client of it. Profiles have no key for anything the web can see.

**Consequences.** One image, many machines, with a stable print across all of them. Existing browser extensions do not run directly and must be ported, as Shield SIEM was. Profile signatures are not implemented yet.
