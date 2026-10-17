# Faction strength

**What it shows:** Every recorded faction's combined factories or territory as a race, lines or full counted-member breakdown.

**Product(s) it reads:** [`faction-totals`](../../../../crates/dashboard-base/src/products/faction_totals.rs), using [`country-factories`](../../../../crates/dashboard-base/src/products/country_factories.rs) or [`territory`](../../../../crates/dashboard-base/src/products/territory.rs) measure tracks and [`country-groups`](../../../../crates/dashboard-base/src/products/country_groups.rs) rosters.

**Settings:** Factories/Available and all factions by default; factory count or Territory measure/rule, ranked factions or summed faction group, members at hour/end/ever, Show Members and inline/pop-out drill; Campaign, Follow, Pin and Range plus shared race/line controls. Army is not offered.

**Notable behaviour:**

- Prepared member values and memberships supply exact reached hours and window summaries. Dense lines and member stacks share the summary kit; aliases join by native holder before summation. Hover, keyboard and end labels retain exact sums, and cancelling member changes do not add keyboard stops. Parity models are **Faction totals** and **Faction member values**.
- Dense Top lines keep every faction whose column maximum reaches the N-th largest column minimum. This preserves leaders that peak inside a column and can show extra lines. Exact windows, reached values and picks retain exact ranking.

- Every counted member needs a known value or proven absent-country zero; incomplete rosters/identities leave gaps, not partial sums. End membership uses a dissolved faction's last complete roster only when dissolution is proved; Ever needs the full membership history.
- Unprepared spans wait for prepared membership and identity proofs. End/Ever counted holders, aliases and dissolution cannot be established from an isolated member-value frame. A shown hour with members at the hour (factories, no member breakdown) answers at once from that hour's rosters, identities and factories, exactly as preparation will; lines, End/Ever and Show Members wait for preparation.
- Names and colours use the shared dated faction helpers. A dissolved End set is labelled with its last-member date. Counted-member tooltips describe the selected roster rule, which may differ from the live roster.
- Member detail retains exact signed values, unknowns and any unassigned part of an independently known total. Native holders prevent End/Ever aliases from admitting replacement countries; look requests use proven dated identities.
- Clicking bars/lines opens the full selected faction independently of top N. Overlapping picks require a faction choice; Show Members also reaches zero/unknown totals. A held pick stays held; Follow uses the clock; Pin stores measure, membership, faction key and origin scoped to campaign/branch.
- Sparse member histories and virtual parts preserve access to every member without a dense member-by-hour matrix. The shared view suspends an inline drill's hidden main chart.

**Files:** [FactionStrengthWidget.tsx](FactionStrengthWidget.tsx) configures [FactionRaceView](../../charts/race/FactionRaceView.tsx); [factionStrengthData.ts](factionStrengthData.ts) shapes measure requests/labels, with adjacent tests. `definition.ts` and `index.ts` declare settings and registration. Territory meaning is shared with [Territory](../territory/README.md).

Shared framework conventions: [widgets](../README.md).
