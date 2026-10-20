# Faction influence

**What it shows:** Each member's share of its faction's influence: stacked over time (**Influence over time**) or ranked at the shown hour (**Member ranking**, with power projection share and contribution). A member opens its own share against the rest of the faction in the shared drill.

**Data it reads:** `faction-state` (`view: 'influence'`) for the recorded influence-share history of every account of the chosen faction, and [`faction-details`](../../../../crates/dashboard-base/src/products/faction_details.rs) at the shown hour for the ranking's current members. The faction comes from [factionChoice.ts](../../dashboard/factionChoice.ts).

**Settings:** Faction; **View** (also in the header, and its own picker card "Faction influence ranking"); the ranking table's options; the shared line-chart time options; Breakdown opens.

**Notable behaviour:**

- Shares are the game's recorded influence shares, never re-normalised. A member whose share is not known at an hour leaves that stretch a gap; members appear from the hours the rosters list them.
- Retained accounts of departed countries are not ranked as members.
- Each account is its own native holder: its band, swatch and drill are named and coloured as that holder at the last hour it was a member, so two countries holding one tag in turn never share a name or colour. A name still being read says so; one the recording lacks reads unavailable. Every account's look is read only for the bands over time; the ranking reads only an opened member's.
