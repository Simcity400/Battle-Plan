# Land battles

**What it shows:** Which land battles happened where and when, who attacked whom and who won, and what each side lost, as a log or a biggest-battles list; a click opens one battle's course and equipment losses.

**Products:** [`land-battles-manifest`, `land-battles`, `land-battles-ranking`, `land-battle-course`](../../../../crates/dashboard/src/products/land_battles.rs) and [`land-battle-terrain`](../../../../crates/dashboard/src/products/land_battles/terrain.rs); [`place-names`](../../../../crates/dashboard-base/src/products/place_names.rs) supplies locations. Uses [shared battles](../battles/README.md) for windows, ranking, filters, names and Jump.

**Settings:** All countries and the log by default; manual/smart-group filters; Log or Biggest (also a switch beside Live), Biggest by casualties, most divisions at once or length, user-selected top N, month headers, new-row animation, density and where the battle opens (in the widget or beside it); Campaign, Follow, Pin and Range.

**Notable behaviour:**

- One row serves both views: date (the latest event, as the log orders rows; Biggest shows the start), place with terrain and length, attackers, each side's manpower lost, defenders, [the measure Biggest ranked by: peak size where casualties are unavailable, whatever was chosen] and the result. One country shows its flag and name; a coalition shows flags and **+N**, every country in the hint. Narrow widgets fold names into flags. The winning side's name is full weight and the losing side's muted.
- The result shows the winning side's flags, with its name when it is one country, then **won**. A battle open at the shown hour says **Ongoing** (with the live dot when following). An unread ending says **Ending not recorded**; an ended battle without a verdict says **Result not recorded**, and the native verdicts **No winning side** and **Both sides awarded a win** stay as recorded.
- Manpower bars are each row's split: the larger loss fills its half and the numbers carry the size. Unknown losses use the hatch. Colours are the first two categorical palette colours (attackers, defenders).
- The tooltip is one summary: terrain, start, end, length, most divisions at once, each side's countries, most divisions, divisions at the last read, manpower and whole equipment lost, the result, and one dated note (**Final losses** or **Losses so far**, with the read).
- The opened battle shows the same facts per side, manpower lost so far and divisions at every recorded read as held steps to the shown hour (unread hours and rosters break the line), and equipment lost by design at the latest read at or before the shown hour: same-named designs sum exactly, zero designs are left out, six per side then the rest as one line. Exact amounts are in each design's hint.
- Hourly counters retain manpower and exact signed fixed5 equipment losses separately. Census rows carry each side's manpower and exact equipment total per read; replies from builds that listed each design are reduced to that total. Post-flush results supply final losses and native winner masks. Older absent fields stay unknown. Only manpower ranks casualties; unavailable totals fall back to peak size. Equipment shows whole numbers.
- `land-battle-course` sends one battle's every run and every read with each design's exact amount. Designs carry their content name (null when unnamed) and their handle separately, so same-named designs stay apart and no handle is shown as a name. A side lists every design on its first read or when its designs or order change, else only the changed amounts; the client rebuilds each full read. Window replies send each warning reason once, by index.
- `land-battle-terrain` titles each province's terrain as the land combat window does: the province's terrain name from the content map definitions, the first terrain database category with that name regardless of case, and that category's localisation. A missing key or category stays unavailable with its reason. Only drawn rows ask; terrains already read stay while the next rows load, for one recording revision and content revision: after either changes, every drawn row asks again.
- Counts include distinct reserve divisions at `readHour`; peak is the largest simultaneous roster, a minimum with incomplete bounds; rows also carry each side's most divisions at once, null from builds that did not send it. Duration uses observed span/native rounds and is a minimum (**at least**) only for a battle already under way when its recorded stretch began. Terminal-only rows retain results but have no known roster/peak; the manifest accepts a complete census or covered result.
- Locations try primary name, state, then **Unnamed province**, including empty primary names; a name still loading reads **Loading…**. State-less provinces never use region names.
- A result side that lists no countries keeps the countries of the battle's last hourly read, dated at that read; filters and ranking match each side at its own read. A side unknown in both stays unknown and survives every filter.
- Adjacent complete absence bounds episodes. Gaps split them and end known losses; results cannot join across gaps. Latest-present battles stay ongoing. Disappearance proves neither winner nor final losses.
- Prefetch cannot reveal future facts. Filter before top N; unknown participants survive. Range ranking uses the full observed peak/length.
- Parity preparation uses the shared battle reader to retain all selected real windows after visible screenshots. Complete ordered rows retain identities, supplying dates and exact numeric/loss operands without resolving offscreen country/location labels. Bounded rendered samples use the ordinary dated labels and description helpers. Failed or pending history windows refuse completion.

**Files:** [LandBattlesWidget.tsx](LandBattlesWidget.tsx) supplies the BattleViewAdapter and its view; [LandBattleList.tsx](LandBattleList.tsx) draws the rows, header switch and terrain lookup, and [LandBattleDrill.tsx](LandBattleDrill.tsx) the opened battle. [landBattlesData.ts](landBattlesData.ts) validates and projects rows and loss wording; [landBattleCourse.ts](landBattleCourse.ts) validates the course and terrain replies and shapes the drill, tooltip and wording; adjacent tests cover both. `definition.ts`, `index.ts` and `landBattles.css` own settings, registration and style.

Shared framework conventions: [widgets](../README.md).

Land battle queries read prepared block replies, ranking records and course pages.
The background census uses each accepted frame and persists its derived continuation
for resume. Plain compressed JSON pages hold exact block replies and compact ranking
records with prior peaks and supplying loss reads. Courses use stable numeric
keys from the shared catalogue.
Ranking reads only records for battles in the selected windows and uses the same
country filter, measure, tie order and reply builder as the recording product.
Delayed finalizer facts repair their affected block pages. Manifests read prepared
availability. The Land battles log and biggest list retain their parity row labels.
