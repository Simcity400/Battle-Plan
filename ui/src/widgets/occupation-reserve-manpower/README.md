# Occupation reserve manpower

**What it shows:** How much reserve manpower each occupier holds from each occupied country: a ranked race, one bar per holder split by source country in each source's colour, and, from a bar, that holder's sources over time.

**Product(s) it reads:** [`occupation-reserve-manpower`](../../../../crates/dashboard-occupation/README.md), a keyed family prepared from the daily reserve entries. The race request holds every holder's entries over the branch and looks the hour up locally; a holder's history is read when its breakdown opens.

**Settings:** Show (Holders or Holder breakdown, also a picker card), the race's time and axis settings, date in the corner, and where the breakdown opens. Countries: all holders by default; chosen countries, groups or the top N.

**Notable behaviour:**

- Bars are the sum of a holder's positive entries; entries from the same source share its segment, and the tooltip lists every entry. Negative entries, whose meaning the recording does not establish, are listed apart as Negative entries and never drawn or netted.
- A holder with no entries at a recorded read holds none and is absent; hours not recorded say so. Sources that do not resolve keep their amounts under Source country unavailable.
- Holders and sources are recorded country identities (tag and slot), so a reused tag never merges two countries.
- Below 420 px the race's own narrow rules apply: rows scroll under a fixed axis and names ellipsise.

**Files:** [OccupationReserveWidget.tsx](OccupationReserveWidget.tsx) draws the race and breakdown; [occupationReserveData.ts](occupationReserveData.ts) validates replies and builds entries and stacked bands; [useOccupationReserve.ts](useOccupationReserve.ts) requests the race.

Shared framework conventions: [widgets](../README.md).
