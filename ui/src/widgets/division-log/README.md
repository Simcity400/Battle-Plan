# Division log

**What it shows:** Observed division creations and witnessed losses/transfers, newest first, with recorded templates, countries, locations and known killers.

**Product(s) it reads:** [`division-log-manifest`, `division-log`, `division-log-totals`](../../../../crates/dashboard/src/products/division_log.rs); [`place-names`](../../../../crates/dashboard/src/products/place_names.rs) supplies drawn locations.

**Settings:** All countries by default; created/lost-and-transferred/both, branch-scoped template choices, numeric totals and shared log controls; Campaign, Follow or Range.

**Notable behaviour:**

- The initial complete census is a baseline, not creation of every existing division. Later appearances are **by** bounds; reveal-skip intelligence copies are excluded. Absence alone proves no loss: removals require native deletion witnesses and keep their distinct dispositions.
- Exact destruction time/location/killer requires one full-handle match in an adjacent complete native drain. Events wait for their supplying read; Jump uses it. Other removals retain bounded dates and last observed locations.
- Template handles belong to the campaign/branch; empty selection means all, while unavailable saved choices remain removable. Native artwork selection and stored-name language refusal live in the product.
- Countries match at the actor's supplying hour, following chosen native holders through aliases; pending identities stay loading. Unknown names, flags, locations and killers remain explicit.
- Block windows load only the playhead, prefetch and browsing/visible neighbours. Totals count the full selected history, not loaded rows. Drawn rows alone request dated looks and places. Missing row artwork uses a helmet fallback.

**Files:** [DivisionLogWidget.tsx](DivisionLogWidget.tsx) owns loading/browsing; [divisionLogData.ts](divisionLogData.ts) validates and shapes rows/totals, with adjacent tests. `divisionLog.css`, `definition.ts` and `index.ts` own styling, settings and registration. Window helpers are shared with [battles](../battles/README.md).

Shared framework conventions: [widgets](../README.md).
