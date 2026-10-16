# Wars timeline

**What it shows:** The native war overview from one viewpoint, as expandable war lanes or country lanes with participation, capitulations and exact casualty counters.

**Prepared data it reads:** [`wars`](../../../../crates/dashboard/src/products/wars.rs) supplies viewpoint-dependent groups from one branch-global prepared change stream; independent reached-hour reads supply exact casualties; its [product guide](../../../../crates/dashboard/src/products/wars/README.md) explains native grouping/app continuity. [`country-groups`](../../../../crates/dashboard-base/src/products/country_groups.rs) supplies faction and country history.

**Settings:** The widget's country is the viewpoint: the dashboard's country unless one is picked; Timeline/Country by country, country-view colours by war/side and Grow with playback; Campaign, Follow, Pin and Range.

**Notable behaviour:**

- The game has no global grouped-war identity. The viewpoint is never guessed from metadata. Changing branch/viewpoint collapses participant expansion; reply refreshes preserve it.
- Live shows only reached wars/participants, with no future placeholder rows. Observed gaps stay blank; **by** start/end/join/leave bounds never invent peace/departure dates. Still-observed bars retain open ends, including at recording end.
- Participant lanes follow native holders through aliases; reused tags split identities. Simultaneous wars use separate subrows. Country-view side colour uses a recorded faction only with a strict majority of distinct participants; unknown/mixed/ended sides keep explicit palette fallback.
- Displayed and hovered casualty values load independently while structural lanes stay mounted. Pending counters say Loading. Casualties use exact covering runs, with separate unattributed losses. Never sum overlapping native war groups. Ended/not-started/unrecorded and outside-participation states remain distinct.
- Names, flags and colours use proved shown/hovered holders. Loading/missing names retain explicit status, including when native text equals a status word or is empty. A whole-campaign measurement catalogue stabilises widths but cannot fill a dated label. Shared Timeline owns zoom, keyboard and Jump.

**Files:** [WarsWidget.tsx](WarsWidget.tsx) renders/loads; `warsData.ts` prepares lanes/bounds, `warsColours.ts` plans identity-bounded colours and `warsTooltip.ts` shapes dated explanations. Adjacent tests cover these rules. `definition.ts` and `index.ts` declare settings and registration.

Shared framework conventions: [widgets](../README.md).
