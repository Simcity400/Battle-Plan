# Peace deals

**What it shows:** Newest-first conference cards with recorded winners, losers and actions, plus explicitly dated derived or unattributed outcomes.

**Product(s) it reads:** The prepared peace-deals query gathers branch events and uses the existing product reply builder.  [`peace-deals`](../../../../crates/dashboard-peace-deals/src/products/peace_deals.rs) and its [exact finalizer layer](../../../../crates/dashboard-peace-deals/src/products/peace_deals/exact.rs); [`place-names`](../../../../crates/dashboard-base/src/products/place_names.rs) supplies dated state labels.

**Settings:** Whole world with no country picker; content-defined action filter and shared log density/animation/month/chip controls; Campaign, Follow, Pin and Range.

Unprepared outcomes show the shared "This range is still being prepared." status.
Conference closure, derived transfers and delayed finalizers require earlier
observations; an unprepared span remains pending rather than an empty conference list.

**Notable behaviour:**

- Actual outcome dates and supplying observations stay separate. Exact actions use captured finalizer dates; estimated cards say **by** and show one **Estimated between** status. Missing outcomes say **Outcomes not recorded**. Earlier fallback cards can remain before that read.
- Derived outcomes require the same native holders at both bounding reads and exclude cancelled subject relations.
- Unknown war/action/taker/target facts stay explicit; giver never substitutes for target. Captured ordered actions, automatic/reduced outcomes and subject relationships remain distinct. Grouped war-end outcomes do not invent a conference identity.
- Filters keep whole matching conferences, including all their actions. Range clips outcomes; backward seeks remove future cards immediately.
- The log lists conference cards only. Hours the hourly finalizer capture or ownership/war reads missed, including hours missing from the recording, could hide a conference: when any fall within the shown time, one muted **Some conferences may be missing.** line sits above the log and its hint gives their span, never past the shown hour. The conference roster is read once a day; the hours between its reads are its cadence, not gaps, because a conference that opens and closes between them still reaches the finalizer capture or the ownership comparison.
- Width-fitted participants/targets retain every remainder on hover/focus and in complete accessible tooltips. Only drawn cards request dated country/state facts; missing state names use **State N**, while flags retain a quiet unavailable state.
- Unavailable flags retain the shared placeholder width and border radius in action chips and participant lists, independent of stylesheet load order.
- LogList owns variable-height windowing, keyboard, motion and Jump. Live entry animation does not replay merely from scrolling. Jump targets the card's displayed exact/bounded date.

**Files:** [PeaceDealsWidget.tsx](PeaceDealsWidget.tsx) renders shared LogList/FittedList cards; [peaceData.ts](peaceData.ts) groups, filters and projects rows/tooltips, with adjacent tests. Shared [usePeaceDeals.ts](../../dashboard/usePeaceDeals.ts) loads/validates. `peaceDeals.css`, `definition.ts` and `index.ts` own styling, settings and registration.

Shared framework conventions: [widgets](../README.md).
