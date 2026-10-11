# Territory breakdown

**What it shows:** Territory as a sunburst grouped by recorded faction, country and content strategic region, with independently measured totals.

**Product(s) it reads:** [`territory-breakdown`](../../../../crates/dashboard/src/products/territory_breakdown.rs) uses [`territory`](../../../../crates/dashboard/src/products/territory.rs) weights/totals; [`country-groups`](../../../../crates/dashboard/src/products/country_groups.rs) supplies faction rosters.

**Settings:** All countries by default or manual/smart groups; Territory's states/provinces/signed victory points/land map cells and owned/controlled rule; optional hierarchy rings and shared sunburst settings; Campaign, Follow, Pin and Range.

**Notable behaviour:**

- A state splits proportionally by its native province roster's strategic-region membership, retaining repeated entries and exact fractions. Hidden levels/folding cannot round a fraction into a whole state. Missing/conflicting membership shows **Unknown region** with counted diagnostics; no centroid or vanilla region list is used.
- Unheld, non-aligned and unresolved holders are distinct. Missing faction rosters cannot prove nonalignment. Native holder spans follow manual roots/aliases and keep reused tags and gaps separate; groups preserve counted dates.
- Independent world/selected totals stay authoritative. Missing active attribution makes selected totals unknown; a complete measure read can prove an absent country's zero. A missing reading cannot prove zero or an empty selection.
- Signed victory points remain exact selectable values; positive amounts supply slice geometry. Gaps/incomplete tables break runs. Names/affiliations use their own shown dates and recorded content; only native country nodes carry flags/refusal states.
- Shared Sunburst owns zoom, breadcrumbs, keyboard, legends, Share/Fill and expandable Other folding. Jump targets the shown moment; continents are not a hierarchy level.

**Files:** [TerritoryBreakdownWidget.tsx](TerritoryBreakdownWidget.tsx) renders; [breakdownData.ts](breakdownData.ts) validates and projects exact regional terms, with adjacent tests. `definition.ts` and `index.ts` declare settings and registration. Measure limits are shared with [Territory](../territory/README.md).

Shared framework conventions: [widgets](../README.md).
