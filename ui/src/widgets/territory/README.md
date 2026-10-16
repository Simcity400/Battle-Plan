# Territory

**What it shows:** How much territory each country or faction holds, as a race, lines or a share of the world.

**Product(s) it reads:** [`territory`](../../../../crates/dashboard-base/src/products/territory.rs) supplies totals; [`faction-totals`](../../../../crates/dashboard-base/src/products/faction_totals.rs) sums [`country-groups`](../../../../crates/dashboard-base/src/products/country_groups.rs) rosters.

**Settings:** Top-ten race of controlled states by default; provinces, signed victory points or land Map area; owned/controlled; manual/smart groups, sums or factions with hour/end/ever membership; faction member drill and shared race/line controls; Campaign, Follow, Pin and Range.

**Notable behaviour:**

- Prepared territory series supply reached values, width summaries and keyboard dates. Fine windows draw the legacy exact path; dense views draw the shared summary. Group sums use exact member runs within fine windows, aligned column sums for dense geometry, and exact reached-hour sums for tooltips. The parity model is **Territory values**.
- Lines draw beyond prepared coverage from transient block-keyframe samples, with the shared approximate style. Zooms touching at most thirteen unprepared blocks reconstruct every frame. Hover and end labels read exact target hours; temporary Top ranks use sampled country values. Native identity joins retain tail-only countries and aliases without writing prepared data.
- Map area counts content land raster cells, without physical calibration or latitude correction. Tooltips state northern distortion. Unavailable saved area choices show their reason; other measures work without map assets.
- Map area samples resolve the recorded province raster once per query and share the observer's land extraction. Missing content leaves area unknown while states, provinces and victory points remain independent.
- Complete reads prove exact totals/zero; unresolved non-null holders or incomplete tables leave affected measures unknown. Native holders follow selected roots through aliases; replacements/gaps stay separate and absent countries do not become individual zero lines. Capitulation markers do not end values.
- Share uses an independent world total, including unassigned territory, plus **Rest of world**. Missing totals/rosters/selected colours make it unavailable. Overlapping fixed faction rosters and negative values cannot form proportions; country group Sum applies only to Race/Lines.
- Faction sums require every member value or proven absent zero. End uses a dissolved faction's last complete roster only with proved dissolution; Ever requires complete membership history. Shared dated names/colours and counted-roster tooltips apply.
- Faction requests join prepared member series and roster facts in the window. Exact sums and native absence use the shared faction sweep; dense geometry uses the group sum and summary stack. Share uses exact reached totals. Parity models are **Faction totals** and **Faction member values**.
- Faction drill keeps its campaign/branch-scoped faction, full signed/unknown member histories and membership rule outside top N. Picking a day holds; Follow uses playback; Pin keeps faction/origin/measure. Shared race kits own easing, without adding readings.

**Files:** [TerritoryWidget.tsx](TerritoryWidget.tsx) loads/configures views; `territoryData.ts` validates/projects totals; `TerritoryShare.tsx` renders the single-ring sunburst and `territoryShareData.ts` shapes it; adjacent tests cover projections. `definition.ts` and `index.ts` declare settings and registration; faction details use [FactionRaceView](../../charts/race/FactionRaceView.tsx).

Shared framework conventions: [widgets](../README.md).
