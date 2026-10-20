# Civil wars

**What it shows:** Every civil war on a lane timeline: a heading per civil war ("Civil war in <country as named at the outbreak>", the side that existed before it) with its sides beneath, each breakaway indented under the side it broke from, and a winner only where the recording proves one. A header pill counts civil wars and ongoing ones.

**Prepared data it reads:** [`civil-wars`](../../../../crates/dashboard-politics/src/products/politics_state/civil_wars.rs), once per branch through the prepared path: each civil war's sides with their tag history, listed hours, parent side, how a side left (a recorded annexation), the hours the game's civil-war predicate held for a side and the proven outcome with its read hour, and the hours the lists were read. [`country-groups`](../../../../crates/dashboard-base/src/products/country_groups.rs) supplies native holders for narrowing and identity bounds for looks.

**Settings:** All countries by default, or chosen countries and groups (civil wars in which one of their native holders was a side); Ended civil wars Shown/Hidden; Grow with playback; Campaign, Follow (Live), Pin and Range.

**Notable behaviour:**

- Expanded by default; clicking a heading (or Enter/Space) collapses it. Ongoing civil wars come first, then the newest outbreak.
- A civil war is ongoing at the shown hour while the game's own civil-war predicate holds for a side, resolved from the reply's dated hours at playback: a leftover listing naming a country without states does not keep it going, and its end is when the predicate stopped holding. A listing the predicate never held for is no ongoing war and names no end.
- The heading's second line reads "Ongoing · N sides", "Winner: <side>" or "Outcome unknown". A winner comes only from the reply's outcome (a civil-war annexation of every other side); capitulation, disappearance, an outside annexation or a gap decide nothing. Once proven, the winner's lane carries the Winner pill in the theme's positive colour and the other sides are muted; under an unknown outcome nothing is muted.
- Sides are named and coloured at the shown hour while listed, else as they were at their last listed hour; the heading names its country at the outbreak and the winner at the outcome. Tags resolve per hour (dynamic and reused tags), never by a side's final tag; codes appear only in tooltips. A side with no recorded tag reads "Name not known", counted in diagnostics.
- Live reveals a civil war at its outbreak and a side when it is first listed; bars grow to the playback date. How a side left and the outcome appear only once playback reaches the read recording them; seeking back removes them.
- Hours the lists were not read are unknown, like gaps: a listing ending there reads "Last recorded", never ended. Hours after the prepared prefix are still being prepared: banded, keyed with Approximate through the shared key, and never an ending. Only lanes in view read names, flags and colours; tooltips read the countries they name at their own hours.
- The menu offers Jump to the hour clicked, the outbreak, a split and the end (the last hour it was ongoing).

**Files:** [CivilWarsWidget.tsx](CivilWarsWidget.tsx) renders and loads; [civilWarsData.ts](civilWarsData.ts) validates the reply and shapes the live cut, tree, lanes and wording, with adjacent tests. `civilWars.css`, `definition.ts` and `index.ts` own styling, settings and registration.

Shared framework conventions: [widgets](../README.md).
