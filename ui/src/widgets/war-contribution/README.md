# War contribution

**What it shows:** Who carried each side of one war: every participation term the war screen counts (equipment damage, occupation, air damage, strategic bombing, ship and convoy damage, casualties, lend-lease sent and received) as a two-sided bar per term, the viewpoint's side left and its opponents right, each bar split into its members in their country colours. The last row, Total contribution, is the game's own participation share. Unknown casualty contributions are hatched parts.

**Data it reads:** the [`wars`](../../../../crates/dashboard-wars/src/products/wars.rs) product, once for the viewpoint's war inventory and names (the same request Wars makes) and once with a requested-hour read for the participation groups, which the product reads from that hour's frame alone. The group shown is the one whose native sides are the chosen war's at that hour; nothing is summed across overlapping native groups. Scores stay exact fixed-point words: bar lengths are geometry only and the tips and tooltips print the recorded values.

**Settings:** the widget's country is the viewpoint (the dashboard's unless one is picked). **War**: the viewpoint's main war (the one with the most participants going on at the shown hour, else the largest fought so far) or any war the viewpoint fought, by in-game name. **Show** (also the caption switch): Share of side, where each side's bar is full and split by member, or Totals, where each row scales to its own larger side because the terms have different units, so no value axis is drawn. Campaign, Follow, Pin and Range: an ended war reads its last observed hour.

**Notable behaviour:**

- A term with nothing yet reads "None yet"; a participant the group did not score is left out of its side rather than drawn as zero, and a group the product refused says so instead of drawing.
- Hovering a part names the member, the term, its exact value and its share of the side; hovering elsewhere in a row gives both sides' totals. The menu jumps to the war's start and to members' joining dates.
- Below 420 px the exact values move from the bar tips into the tooltip; long term names shorten with the full name in the tooltip.

**Files:** [WarContributionWidget.tsx](WarContributionWidget.tsx) loads and draws through the shared [ButterflyChart](../../charts/butterfly/ButterflyChart.tsx) with per-side parts; [warContributionData.ts](warContributionData.ts) builds the rows and chooses the war; [useWarChoices.ts](useWarChoices.ts) lists the wars for settings.
- Members' names and colours are read at the war's read hour itself, never a sampled or held look, so a tag that passed to another country shows its holder then.
