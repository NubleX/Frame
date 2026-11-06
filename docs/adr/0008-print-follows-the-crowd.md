# 0008: The print follows the crowd

**Status:** accepted, 2026-11-06

**Context.** A print frozen forever would go stale and stand out. The future of the crowd cannot be planned.

**Decision.** No frozen print and no scheduled fingerprint epochs. Frame tracks upstream Chromium through CEF. Every edition ships the same guest image. The release gate compares Frame with the same Chromium version instead of a golden file.

**Consequences.** Editions stay identical at any moment and the print moves with the crowd. Arm hosts may differ from x86_64, so "identical" stays a test and not a promise until Phase 4 proves it.
