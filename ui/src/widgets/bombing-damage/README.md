# Bombing damage

**What it shows:** Damage suffered and inflicted per country in a butterfly chart, measured in building levels with exact amounts and dealt/taken ratios.

**Product(s) it reads:** [`military-air-ledger`](../../../../crates/dashboard/src/products/military_air_ledger.rs), using its bombing family. [Bombing damage breakdown](../bombing-damage-breakdown/README.md) uses the same scan.

**Settings:** Top ten by dealt plus taken by default; manual countries and smart groups; bar-end values, ratio column and inline/pop-out drill; Campaign, Follow, Pin and Range.

**Notable behaviour:**

- The quantity is actual applied building-health loss from the proved aircraft callers, not the native returned damage budget. Unknown callers make coverage uncertain; other damage writers are excluded.
- Unknown attackers remain in suffered totals with separately named unattributed inflicted damage. Complete empty intervals prove zero; missing intervals remain unknown. Ratios distinguish zero from missing denominators.
- Ranges use native event dates; the supplying read independently gates visibility. Later reads never backfill earlier playback. Country looks and flag refusal states use the dated native holder.
- Clicking either half opens building parts/history; selecting a type isolates it. Signed corrections remain selectable; only positive area draws. Incomplete coverage retains exact parts without shares. Picking holds; Follow uses the clock from any parent time mode; Pin keeps holder/direction. Missing dates keep the picker; stale clients cannot reuse controls.

**Files:** [BombingDamageWidget.tsx](BombingDamageWidget.tsx) configures [MilitaryAirView](../../dashboard/MilitaryAirView.tsx); shared [militaryAirLedger.ts](../../dashboard/militaryAirLedger.ts) owns exact projections. `definition.ts` and `index.ts` declare settings and registration.

Shared framework conventions: [widgets](../README.md).
