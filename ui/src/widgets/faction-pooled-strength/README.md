# Faction pooled strength

**What it shows:** What a faction pools at the shown hour: its shared resources (faction totals), each member's recorded manpower contribution, research sharing (the scientist base and the sharing members) and its shared doctrines. Moment widget: Follow or Pin.

**Data it reads:** [`faction-details`](../../../../crates/dashboard-base/src/products/faction_details.rs) at the shown hour for the chosen faction ([factionChoice.ts](../../dashboard/factionChoice.ts)). The bars reuse Resource balance's row layout.

**Settings:** Faction; **Pool** (also the caption switch), offering only the pools the recorded state supplies: a locked section or an unavailable read is not offered, and is never shown as "nothing pooled".

**Notable behaviour:**

- Only manpower has recorded member contributions; resources are faction totals (the recording names no member provenance), research names sharing members without shares, and doctrines list the shared folders.
- Manpower keeps every native occurrence; shares are against the recorded pool total, none when it is not recorded.
- The manpower drill keeps the member clicked as the holder its tag named at that hour, matched by slot and country id, wherever playback goes; another holder of the tag, or an earlier holder's unknown tail, never joins it. Flags are read only for the rows the mounted manpower list shows, and released when it goes.
