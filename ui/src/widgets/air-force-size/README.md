# Air force size

**What it shows:** Planes or wings in each country's recorded air-wing census, as a race, lines or one country's equipment-type mix. National stockpiles are excluded.

**Product(s) it reads:** Prepared air-force series provide bounded viewport geometry, exact reached censuses and keyboard changes. [`air-force-size`](../../../../crates/dashboard-forces/src/products/air_force.rs) remains the history reference.

**Settings:** Top-ten race of plane counts by default; manual countries and smart groups; planes/wings, roster owner/country it fights for, type parts, Chart/Breakdown and inline/pop-out drill. Campaign, Follow, Pin and Range plus shared race/line controls.

**Notable behaviour:**

- Types are content-defined equipment archetypes resolved from the recorded variants. A multi-type wing counts once as **Mixed types** when measuring wings.
- The recorded plane count is independent of equipment attribution. Missing or non-reconciling banks leave **Other** while a known total stays exact. A complete empty census proves zero; absent countries and missing reads stay unknown.
- Clicking a country or mix opens its full type history independently of top-N membership. Overlapping line picks require a country choice. Pins save the native holder and origin hour; renames follow that holder and tag reuse cannot replace it. Missing or conflicting identity stays unavailable.
- Local date picks and Follow update the pinned header at the body date. Follow uses the track-aligned playback clock independently of the parent time mode, with full-branch detail bounds. Part rows select their history; All parts restores the stack.
- Shared PartsRaceView owns loading, country identity, dated looks and interactions. Drawn easing creates no additional measurements; gaps break histories.
- Total lines draw approximate unprepared keyframes immediately and exact frames when a zoom touches at most thirteen unprepared blocks. The native ranking sweep ranks transient tail values. Dense lines and prepared mixes use width summaries; exact short views keep native reply drawing. Playback, hover and visible line ends read exact native totals. Type mixes and part drills wait for their dated variant spelling catalogue to be prepared. The parity build exposes **Air force size exact values**.

**Files:** [AirForceSizeWidget.tsx](AirForceSizeWidget.tsx) configures [PartsRaceView](../../charts/race/PartsRaceView.tsx); [airForceData.ts](airForceData.ts) owns request shaping and validation, with adjacent tests. `definition.ts` and `index.ts` declare settings and registration.

Shared framework conventions: [widgets](../README.md).
