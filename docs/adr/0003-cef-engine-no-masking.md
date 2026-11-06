# 0003: CEF as the engine, and no masking

**Status:** accepted, 2026-11-06

**Context.** Frame must look like something common and boring. Privacy browsers such as Brave, Opera and Mullvad Browser are recognisable by how they mask. Software that is invisible but everywhere commonly embeds Chromium through CEF or Electron. A unique engine cannot have a known print, because the print comes from how the engine behaves.

**Decision.** Use CEF on Chromium, bare, with no masking. Protection sits around the engine. Do not impersonate Googlebot, Adobe, Oracle, Steam or anything else. Replace parts of the engine only when they pass the print gate.

**Consequences.** Nothing is forged, so layers cannot contradict each other, and off-screen rendering removes the need for a display server. CEF's native API avoids CDP automation tells. CEF can lag Chrome stable. That CEF's print equals Chrome's is an inference until Spike 2 measures it. Sites can still track Frame by IP, cookies and login.
