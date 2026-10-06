# Trade flows

Who sells which resources to whom: a Sankey of sellers (left) to buyers (right), one ribbon per seller, buyer and resource, sized by amount (mock-up style-6).

- **Measure (setting):**
  - **Current** (default): each deal's amount per day at the moment shown, so ribbons appear, grow, shrink and vanish during playback.
  - **Total so far:** everything delivered from the start of the time shown up to the moment shown; ribbons only grow.
  - **Campaign average:** each flow's average amount per day over the hours it traded in that time.
  - With Live off, the moment shown is the end of the campaign (or of a date range), so current is the end state and the other two cover the whole time.
- **Ribbon colour (setting):** by resource (default; the resource legend is always shown, even for one resource) or by seller (the seller's map colour).
- **Countries:** all countries (default, the kit's `countries.all`), chosen countries or a smart group (human players, a faction, at war with a country; members at each hour, at the end or ever).
  - Chosen countries show their trade with anyone (default) or, as a setting, only trade between them.
  - A group counted at each hour counts a trade only while its country is a member, and nothing in hours its membership was not recorded.
  - At the moment shown, a group whose members are unknown or none says so (the kit's `worldFilter`).
- **Resources (setting):** any of the campaign's own resources; none chosen shows them all.
- **Countries per side (setting, 12 by default):** the countries that sell or buy the most at the moment shown are drawn. The rest are summed into one "N other sellers" or "N other buyers" node, so nothing is left out. One country past the limit is shown itself rather than as "1 other".
  - When fewer fit the widget's height (the flow kit's `flowCapacity`: a label row a node), only those are drawn, the rest are summed, and the caption says so.
- **Ribbon width (setting):**
  - **Same scale all campaign** (default): widths are amounts, so the diagram grows and shrinks with the trade.
  - **Fit the moment shown:** the trade shown fills the height.
- **Hover** (or the keyboard) on a ribbon shows:
  - the resource and its icon;
  - the amount (per day, delivered, or average per day);
  - the civilian factories paid and the convoys used, of those needed;
  - the number of deals when several are summed.
  - For totals and averages, the factories and convoys are averages over each trade's own days traded, summed as exact fractions before rounding once for display. Whether convoys were needed is decided on the exact sums, and an average above zero that rounds away shows as "<0.1".
  - A ribbon between summed nodes lists its trades, as many as fit the window (`fitTooltip`). A sole trade through a summed node names and flags its actual seller and buyer too.
- **Hover on a node** lists what the country sells or buys per resource and the total. A summed node lists its countries.
- Hovering dims every ribbon but the one pointed at, or a node's own ribbons.
- **Right-click** (Enter or the context-menu key on a keyboard stop) jumps to the moment shown: the read behind it for the current amount. A ribbon also offers when its trade was first and last recorded in the time shown.
- **Keyboard:** with the diagram focused, the arrow keys step through each seller and its ribbons, then each buyer. Home and End go to either end and Escape leaves. Each stop's tooltip shows and is read out in full.
- **Screen readers:** the canvas is described by its largest ribbons, in words.

## Data

**Product `trade-flows`** (`crates/dashboard/src/products/trade_flows.rs`) reads every recorded `ResourceExchanges` deal over the branch, or the deals of the chosen countries. Each flow's parallel run arrays cover the hours its summed terms held.

- **What the values mean** is the same evidence Resource balance uses (the module lists it):
  - `delivered` is the deal's daily amount, exact against the save;
  - `assigned_factories` is the civilian factories paid, exact against the save's `lended_cic`;
  - convoys are shown as recorded, since the save confirms only whether a deal needs any.
- **Freshness:**
  - Deals are read every hour, as Resource balance reads them. An hour without the deal or country tables is unknown, and runs end at a gap or a change in a named country's identity (slot, tag and id).
  - An empty diagram over a time recorded only in part (deals left out, days not recorded, or a group's membership not recorded) says so, for every measure, never "no trade".
  - `known` lists the hours whose deals are known. At any other hour (a gap, a missing table or an unreadable country catalogue), the widget says "Trade deals were not recorded at this time" instead of showing no trade; a total or average over a time with no known hour says "not recorded in this time".
  - `partial` lists known hours whose read also listed deals it could not attribute (a seller or buyer the links do not resolve, a resource the definitions do not name), with how many. They are never shown as the whole trade: the caption and tooltips say how many deals were left out at the moment, or how many days had some over a total or average.
  - Totals and averages count recorded hours only and say how many days were not recorded, including days a group's membership was not.
- **Tooltip names:** a tooltip names and flags its countries as recorded at the hour it is about (the read behind the amount for the current measure, else the moment shown), through the kit's `useHoverLabels`, asking only for the countries that tooltip names; while those reads load, the diagram's own look shows, and a part a finished read did not give is not known.
- **Tooltip updates:** the flow kit caches by the hovered item and memoized tooltip callback. The callback follows the diagram and every input it reads, so filters, measures, range bounds, colours, side limits and arriving names or flags refresh a still hover.
- **Diagram names:** the diagram's own labels are named, flagged and coloured with the kit's `useCountryLooks` (the looks rule of `dashboard/looks.ts`):
  - read as recorded near the moment shown, about once a month of playback (`lookHour`), so a long campaign never asks at every frame;
  - a look is held only while the next read loads, so nothing flashes; once a read finishes, what it did not give is not known: the name falls back to the recording's catalogue name, else the tag, the flag keeps its place outlined, and the colour is the "other" grey.
- **Resources:** the recorded `ResourceDefinitions`, named and pictured through the shared `products/resources.rs` (see the Resource balance README).
  - A mod's resources appear with their own names and icons.
  - A resource without an icon keeps a placeholder, and one without a name shows its token.
- **Colours:** each resource takes the categorical palette colour of its place in the campaign's full list (the game's order), whatever the filter shows; from the ninth on, resources share the "Other" colour. Countries use their recorded map colours ("fixed" country colours nudge them apart).
- **One scale all campaign** is never below what a side carries at any hour: the largest sum of daily amounts at once, the final total, or the sum of each flow's highest running average (a running average peaks at the end of a run, even between reads).
- **Tag order** follows the data service's (`compareTags`), so ties and the reply's sorted countries agree with Rust.
- **Market purchases (`PurchaseContracts`)** are the international market's equipment contracts, not resources. The recorded table has no equipment type or quantity to size a ribbon by, so they are not shown.

**Size:**
- The reply is bounded. A scan that would keep more than 500,000 runs, or a reply longer than the transport holds (many short flows cost more than their runs; measured with `transport::json_len` before sending), is refused with a message asking for fewer countries, never cut short.
- A 1,500-country test world fits one reply with every flow.
- On the earlier daily recording (15 days, 90 countries trading) the whole-world reply was 92 KB: 411 flows, 476 runs and 13 KB of resource icons. A current hourly recording has not been measured here.

**Built from the kit:**
- the flow diagram ([charts/flow](../../charts/flow/));
- `fitTooltip`, `formatAmount`, `rescale`, `rescaleSum` and `readAt`;
- `nudgeApart`, `jumpItem`, `useCountryLooks`, `useHoverLabels`, `worldFilter`, `compareTags` and the country groups.

This folder keeps only what trade means: the measures, the summing of the rest into one node and the wording.

**Speed:** the reply is decoded once; each flow keeps running sums, so any hour's total is a binary search. Exact numbers (BigInt) are worked out only for what a tooltip shows.

## Not yet

- **Equipment market contracts** are not shown (see above). Showing them would need their equipment and quantity recorded.
