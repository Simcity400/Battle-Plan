# Bombing damage breakdown

**What it shows:** One country's applied bombing damage as a sunburst, grouped by building type then target state.

**Product(s) it reads:** [`military-air-ledger`](../../../../crates/dashboard/src/products/military_air_ledger.rs), using the same bombing totals as [Bombing damage](../bombing-damage/README.md); [`place-names`](../../../../crates/dashboard-base/src/products/place_names.rs) names states at their recorded dates.

**Settings:** Suffered damage by default or Inflicted; building/state levels, Share/Fill growth and shared sunburst presentation; Campaign, Follow, Pin and Range.

Prepared queries load the shared ledger event pages; availability reads the saved family flag.

**Notable behaviour:**

- Applied loss, signs, native event dates, read-gated visibility, unknown callers and gaps follow the parent bombing measure. The independent country total remains authoritative when part names are unavailable.
- Buildings resolve the event's recorded spelling through content. States use dated recorded IDs/names; native state zero has no containing state. Missing names retain reasons alongside measured amounts.
- Selection fixes one native holder through renames. A campaign/branch-scoped pin never retargets a replacement using the same tag. The header uses the holder's unique alias at the displayed date, or stays neutral when identity is missing/conflicting.
- Fill uses the exact available end total and falls back to Share when it is unknown. The shared sunburst owns zoom, folding, keyboard, tooltip and Jump behavior.

**Files:** [BombingDamageBreakdownWidget.tsx](BombingDamageBreakdownWidget.tsx) loads the ledger and configures [BombingBreakdown](../../dashboard/BombingBreakdown.tsx); `BombingCountryChip.tsx` supplies the header. `definition.ts` and `index.ts` declare settings and registration.

Shared framework conventions: [widgets](../README.md).
