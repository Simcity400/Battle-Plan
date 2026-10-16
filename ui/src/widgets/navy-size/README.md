# Navy size

**What it shows:** Ships or current hull HP in each country's task-force roster, as a race, lines, class/role mix or full breakdown. This is not tonnage.

**Product(s) it reads:** [`navy-size`](../../../../crates/dashboard-forces/src/products/navy.rs).

**Settings:** Top-ten ship-count race by ship class and country holding the fleet by default; HP, assigned country, role, manual countries/smart groups, Chart/Breakdown and inline/pop-out drill; Campaign, Follow, Pin and Range plus shared race/line controls.

**Notable behaviour:**

- Race and drill values read exact hours. Unprepared lines and mixes draw approximate keyframe spans immediately; zooms touching at most thirteen unprepared blocks read exact frames. Native class/role projection resolves recorded content once per query. Dense views use width summaries; exact short views retain native runs and their drawing. Top countries use prepared rank positions in the accepted prefix and the native ranking sweep over transient tail values.

- Current HP uses exact recorded fixed-point amounts. Maximum HP is independently known or unknown; it never replaces current HP. Axis entries in HP convert to raw units. Missing/overflowing HP affects that measure while ship counts remain available.
- Count fleets for selects the country whose roster holds the fleet or the fleet’s assigned country. The separate expeditionary sender field is not either choice. Ships outside the recorded roster, including other convoy/stockpile inventory, are excluded.
- Classes and roles resolve dated authored sub-unit definitions through recorded content. Unresolved classes stay visible parts; names use content localisation or unavailable wording. Internal keys remain in diagnostics. No vanilla class list is assumed.
- Complete present empty rosters prove zero; absent countries/missing inputs stay unknown and gaps break history. Shared easing creates no new measurements.
- The shared drill keeps the selected native holder's full history after it leaves top N or renames. Pins save that holder and its origin hour; reused tags, missing identities and conflicting aliases cannot replace it. Current and maximum HP retain independent availability.
- Local date picks and Follow update the pinned header at the body date. Follow uses the track-aligned playback clock independently of the parent time mode, with full-branch detail bounds. Part rows select their history; All parts restores the stack.

**Files:** [NavySizeWidget.tsx](NavySizeWidget.tsx) configures [PartsRaceView](../../charts/race/PartsRaceView.tsx); [navyData.ts](navyData.ts) shapes requests and validates replies, with adjacent tests. `definition.ts` and `index.ts` declare settings and registration.

Shared framework conventions: [widgets](../README.md).
