# Land battles

**What it shows:** Observed land battles with native locations, division counts, losses, results and a biggest-battles leaderboard.

**Products:** [`land-battles-manifest`, `land-battles`, `land-battles-ranking`](../../../../crates/dashboard/src/products/land_battles.rs); [`place-names`](../../../../crates/dashboard/src/products/place_names.rs) supplies locations. Uses [shared battles](../battles/README.md).

**Settings:** All countries and Battle log by default; manual/smart-group filters, Biggest battles by Casualties/Peak size/Length, user-selected top N and shared log/table controls; Campaign, Follow, Pin and Range.

**Notable behaviour:**

- Hourly counters retain manpower and exact signed fixed5 equipment/variant losses separately. Post-flush results supply final losses and native winner masks; earlier losses say **so far**. Older absent fields stay unknown. Only manpower ranks casualties; unavailable totals fall back to peak size.
- Replies send each equipment label and window warning reason once, by index. Within a reply, a side's read lists every variant on its first read or when its labels or order change, else only the changed amounts. The validator rebuilds each full read (equipment total, values and exact raw amounts) and dated warning before rendering.
- Counts include distinct reserve divisions at `readHour`; peak is the largest simultaneous roster, a minimum with incomplete bounds. Duration uses observed span/native rounds. Terminal-only rows retain results but have no known roster/peak; the manifest accepts a complete census or covered result.
- Locations try primary name, state, then **Unnamed province**, including empty primary names. State-less provinces never use region names; IDs appear only in tooltips.
- Adjacent complete absence bounds episodes. Gaps split them and end known losses; results cannot join across gaps. Unread endings say **Ending not recorded**; latest-present battles stay ongoing. Disappearance proves neither winner nor final losses.
- Prefetch cannot reveal future facts. Filter before top N; unknown participants survive. Header sorting stays in that set; range ranking uses the full observed peak/length. Variant names keep quantities and counted reasons when missing.

**Files:** [LandBattlesWidget.tsx](LandBattlesWidget.tsx) supplies BattleViewAdapter; [landBattlesData.ts](landBattlesData.ts) validates/projects rows and loss wording, with adjacent tests. `definition.ts` and `index.ts` own settings/registration; shared components own rendering, windows and Jump.

Shared framework conventions: [widgets](../README.md).
