# Air kills & losses

**What it shows:** Signed aircraft loss bookkeeping and enemy credit per country, as a race or lines, with a loss-cause drill.

**Product(s) it reads:** [`military-air-ledger`](../../../../crates/dashboard/src/products/military_air_ledger.rs), using its aircraft-loss family; shared numerical loading also serves both bombing widgets.

**Settings:** Top-ten race by own losses by default; enemy losses, regions/causes, manual countries and smart groups, ratio, line/axis controls, one View selector for Race, Lines over time or Loss causes, and inline/pop-out drill. Chart controls appear in the views that use them; Loss causes keeps its future-history switch. Campaign, Follow, Pin and Range.

**Notable behaviour:**

- Booked counts are the native requested amounts, not necessarily aircraft actually removed. Own losses and enemy credit are separate; operational accidents and unknown opponents are not conflated. Signed corrections stay exact.
- Ranges select native event dates, while supplying reads gate visibility. Later publications never backfill earlier playback; missing intervals split histories and never become zero. Zero and unknown ratio denominators remain distinct.
- Cause rows keep signed values and complete-coverage shares; only positive parts draw bars/areas. An unavailable picked day stays unavailable with its history picker, without clamping the date.
- Drill selection fixes the native holder, measure and client, follows its aliases and full history after it leaves top N, and rejects stale controls. Follow uses the actual clock; Pin retains holder, filters and branch-relative time. The breakdown header names that holder at its displayed date or stays neutral if identity is unproven.

**Files:** [AirKillsLossesWidget.tsx](AirKillsLossesWidget.tsx) configures [MilitaryAirView](../../dashboard/MilitaryAirView.tsx); `AirLossCountryChip.tsx` supplies the standalone header. Shared [militaryAirLedger.ts](../../dashboard/militaryAirLedger.ts) owns exact projections; `definition.ts` and `index.ts` declare settings and registration.

Shared framework conventions: [widgets](../README.md).
