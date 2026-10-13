# Land battles

**What it shows:** Observed land battles with native locations, division counts, losses, results and a biggest-battles leaderboard.

**Products:** [`land-battles-manifest`, `land-battles`, `land-battles-ranking`, `land-battle-course`](../../../../crates/dashboard/src/products/land_battles.rs) and [`land-battle-terrain`](../../../../crates/dashboard/src/products/land_battles/terrain.rs); [`place-names`](../../../../crates/dashboard/src/products/place_names.rs) supplies locations. Uses [shared battles](../battles/README.md).

**Settings:** All countries and Battle log by default; manual/smart-group filters, Biggest battles by Casualties/Peak size/Length, user-selected top N and shared log/table controls; Campaign, Follow, Pin and Range.

**Notable behaviour:**

- Hourly counters retain manpower and exact signed fixed5 equipment losses separately. Census rows carry each side's manpower and exact equipment total per read; replies from builds that listed each design are reduced to that total. Post-flush results supply final losses and native winner masks; earlier losses say **so far**. Older absent fields stay unknown. Only manpower ranks casualties; unavailable totals fall back to peak size. Equipment shows whole numbers.
- `land-battle-course` sends one battle's every run and every read with each design's exact amount. Designs carry their content name (null when unnamed) and their handle separately, so same-named designs stay apart and no handle is shown as a name. A side lists every design on its first read or when its designs or order change, else only the changed amounts; the client rebuilds each full read. Window replies send each warning reason once, by index.
- `land-battle-terrain` titles each province's terrain as the land combat window does: the province's terrain name from the content map definitions, the first terrain database category with that name regardless of case, and that category's localisation. A missing key or category stays unavailable with its reason.
- Counts include distinct reserve divisions at `readHour`; peak is the largest simultaneous roster, a minimum with incomplete bounds; rows also carry each side's most divisions at once, null from builds that did not send it. Duration uses observed span/native rounds and is a minimum (**at least**) only for a battle already under way when its recorded stretch began. Terminal-only rows retain results but have no known roster/peak; the manifest accepts a complete census or covered result.
- Locations try primary name, state, then **Unnamed province**, including empty primary names. State-less provinces never use region names; tooltips show names and battle facts.
- A result side that lists no countries keeps the countries of the battle's last hourly read, dated at that read; filters and ranking match each side at its own read. A side unknown in both stays unknown and survives every filter.
- Adjacent complete absence bounds episodes. Gaps split them and end known losses; results cannot join across gaps. Unread endings say **Ending not recorded**; latest-present battles stay ongoing. Disappearance proves neither winner nor final losses.
- Prefetch cannot reveal future facts. Filter before top N; unknown participants survive. Header sorting stays in that set; range ranking uses the full observed peak/length. Variant names keep quantities and counted reasons when missing.

**Files:** [LandBattlesWidget.tsx](LandBattlesWidget.tsx) supplies BattleViewAdapter; [landBattlesData.ts](landBattlesData.ts) validates/projects rows and loss wording, with adjacent tests. `definition.ts` and `index.ts` own settings/registration; shared components own rendering, windows and Jump.

Shared framework conventions: [widgets](../README.md).
