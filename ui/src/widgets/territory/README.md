# Territory

**What it shows:** How much territory each country or faction holds, as a race, lines or a share of the world.

**Product(s) it reads:** [`territory`](../../../../crates/dashboard/src/products/territory.rs) supplies totals; [`faction-totals`](../../../../crates/dashboard/src/products/faction_totals.rs) sums [`country-groups`](../../../../crates/dashboard/src/products/country_groups.rs) rosters.

**Settings:** Top-ten race of controlled states by default; provinces, signed victory points or land Map area; owned/controlled; manual/smart groups, sums or factions with hour/end/ever membership; faction member drill and shared race/line controls; Campaign, Follow, Pin and Range.

**Notable behaviour:**

- Map area counts content land raster cells, without physical calibration or latitude correction. Tooltips state northern distortion. Unavailable saved area choices show their reason; other measures work without map assets.
- Complete reads prove exact totals/zero; unresolved non-null holders or incomplete tables leave affected measures unknown. Native holders follow selected roots through aliases; replacements/gaps stay separate and absent countries do not become individual zero lines. Capitulation markers do not end values.
- Share uses an independent world total, including unassigned territory, plus **Rest of world**. Missing totals/rosters/selected colours make it unavailable. Overlapping fixed faction rosters and negative values cannot form proportions; country group Sum applies only to Race/Lines.
- Faction sums require every member value or proven absent zero. End uses a dissolved faction's last complete roster only with proved dissolution; Ever requires complete membership history. Shared dated names/colours and counted-roster tooltips apply.
- Faction drill keeps its campaign/branch-scoped faction, full signed/unknown member histories and membership rule outside top N. Picking a day holds; Follow uses playback; Pin keeps faction/origin/measure. Shared race kits own easing, without adding readings.

**Files:** [TerritoryWidget.tsx](TerritoryWidget.tsx) loads/configures views; `territoryData.ts` validates/projects totals; `TerritoryShare.tsx` renders the single-ring sunburst and `territoryShareData.ts` shapes it; adjacent tests cover projections. `definition.ts` and `index.ts` declare settings and registration; faction details use [FactionRaceView](../../charts/race/FactionRaceView.tsx).

Shared framework conventions: [widgets](../README.md).
