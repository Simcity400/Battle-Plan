# Wars timeline

**What it shows:** The native war overview from one viewpoint, as expandable war lanes or country lanes with participation, capitulations and exact casualty counters.

Participant hovers show the war-screen participation percentage and score, equipment damage, occupation credit, air damage, strategic bombing, ship/convoy damage, known/unknown casualty contributions and signed lend-lease credit. Requested-hour native snapshots load independently of timeline preparation and remain available after it completes. Missing inputs retain an explicit status. These values use directional participation accounting; the native representative-selection score is a different fact.

**Prepared data it reads:** [`wars`](../../../../crates/dashboard-wars/src/products/wars.rs) supplies viewpoint-dependent groups from one branch-global prepared change stream; independent reached-hour reads supply exact casualties; its [product guide](../../../../crates/dashboard-wars/src/products/wars/README.md) explains native grouping/app continuity. [`country-groups`](../../../../crates/dashboard-base/src/products/country_groups.rs) supplies faction and country history.

**Settings:** The widget's country is the viewpoint: the dashboard's country unless one is picked; Timeline/Country by country, country-view colours by war/side and Grow with playback; Campaign, Follow, Pin and Range.

**Notable behaviour:**

- The game has no global grouped-war identity. The viewpoint is never guessed from metadata. Changing branch/viewpoint collapses participant expansion; reply refreshes preserve it.
- Live shows only reached wars/participants, with no future placeholder rows. Observed gaps stay blank; **by** start/end/join/leave bounds never invent peace/departure dates. Still-observed bars retain open ends, including at recording end.
- Participant lanes follow native holders through aliases; reused tags split identities. Simultaneous wars use separate subrows. Country-view side colour uses a recorded faction only with a strict majority of distinct participants; unknown/mixed/ended sides keep explicit palette fallback.
- Before preparation covers the branch, the lanes are the prepared prefix's, with the hours after it banded and keyed **Still being prepared**; a war open at the prefix's end has no end yet. A shown hour past the prefix lists that hour's wars, read from its frame alone, under "History still being prepared. At <date>:" with their names, sides and casualty counters.
- Displayed and hovered casualty values load independently while structural lanes stay mounted. Pending counters say Loading. Casualties use exact covering runs, with separate unattributed losses. Never sum overlapping native war groups. Ended/not-started/unrecorded and outside-participation states remain distinct.
- Names, flags and colours use proved shown/hovered holders. Loading/missing names retain explicit status, including when native text equals a status word or is empty. A whole-campaign measurement catalogue stabilises widths but cannot fill a dated label. Shared Timeline owns zoom, keyboard and Jump.

**Files:** [WarsWidget.tsx](WarsWidget.tsx) renders/loads; `warsData.ts` prepares lanes/bounds, `warsColours.ts` plans identity-bounded colours and `warsTooltip.ts` shapes dated explanations. Adjacent tests cover these rules. `definition.ts` and `index.ts` declare settings and registration.

Shared framework conventions: [widgets](../README.md).

An unfinished timeline is marked Approximate over its prepared hours because
capitulations retain native dates and can be learned by a later observation.
Those lanes fade with dashed edges and an Approximate key until complete; the
independently read groups at a requested hour retain their own exact operands.

A cold lane inventory does not hide independently observed current war groups.
Those groups remain visible at their own requested hour while history is pending.

An expandable War goals panel requests the viewpoint country's complete inventory and justifications at the displayed hour. It shares its data and presentation with the Diplomacy page's War goals widget.
