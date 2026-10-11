# Navy size

**What it shows:** Ships or current hull HP in each country's task-force roster, as a race, lines, class/role mix or full breakdown. This is not tonnage.

**Product(s) it reads:** [`navy-size`](../../../../crates/dashboard/src/products/navy.rs).

**Settings:** Top-ten ship-count race by authored class/roster owner by default; HP, logical country, broad role, manual countries/smart groups, Chart/Breakdown and inline/pop-out drill; Campaign, Follow, Pin and Range plus shared race/line controls.

**Notable behaviour:**

- Current HP uses exact recorded fixed-point amounts. Maximum HP is independently known or unknown; it never replaces current HP. Axis entries in HP convert to raw units. Missing/overflowing HP affects that measure while ship counts remain available.
- Roster and logical ownership are distinct native links. Expeditionary ownership identifies the sender and makes no host claim. Ships outside the recorded roster, including other convoy/stockpile inventory, are excluded.
- Classes and roles resolve dated authored sub-unit definitions through recorded content. Unresolved classes stay visible parts; names use content localisation or unavailable wording. Internal keys remain in diagnostics. No vanilla class list is assumed.
- Complete present empty rosters prove zero; absent countries/missing inputs stay unknown and gaps break history. Shared easing creates no new measurements.
- The shared drill keeps the selected native holder's full history after it leaves top N or renames. Pins save that holder and its origin hour; reused tags, missing identities and conflicting aliases cannot replace it. Current and maximum HP retain independent availability.
- Local date picks and Follow update the pinned header at the body date. Follow uses the track-aligned playback clock independently of the parent time mode, with full-branch detail bounds. Part rows select their history; All parts restores the stack.

**Files:** [NavySizeWidget.tsx](NavySizeWidget.tsx) configures [PartsRaceView](../../charts/race/PartsRaceView.tsx); [navyData.ts](navyData.ts) shapes requests and validates replies, with adjacent tests. `definition.ts` and `index.ts` declare settings and registration.

Shared framework conventions: [widgets](../README.md).
