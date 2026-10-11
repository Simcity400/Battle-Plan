# Peace deals

**What it shows:** Newest-first conference cards with recorded winners, losers and actions, plus explicitly dated derived or unattributed outcomes.

**Product(s) it reads:** [`peace-deals`](../../../../crates/dashboard/src/products/peace_deals.rs) and its [exact finalizer layer](../../../../crates/dashboard/src/products/peace_deals/exact.rs); [`place-names`](../../../../crates/dashboard/src/products/place_names.rs) supplies dated state labels.

**Settings:** Whole world with no country picker; content-defined action filter and shared log density/animation/month/chip controls; Campaign, Follow, Pin and Range.

**Notable behaviour:**

- Actual outcome dates and supplying observations stay separate. Exact actions use captured finalizer dates; estimated cards say **by** and show one **Estimated between** status. Missing outcomes say **Outcomes not recorded**. Earlier fallback cards can remain before that read.
- Derived outcomes require the same native holders at both bounding reads and exclude cancelled subject relations.
- Unknown war/action/taker/target facts stay explicit; giver never substitutes for target. Captured ordered actions, automatic/reduced outcomes and subject relationships remain distinct. Grouped war-end outcomes do not invent a conference identity.
- Filters keep whole matching conferences, including all their actions. Dated gaps stay visible and never prove no conference. Range clips outcomes and gaps; backward seeks remove future cards immediately.
- Width-fitted participants/targets retain every remainder on hover/focus and in complete accessible tooltips. Only drawn cards request dated country/state facts; missing state names use **State N**, while flags retain a quiet unavailable state.
- LogList owns variable-height windowing, keyboard, motion and Jump. Live entry animation does not replay merely from scrolling. Jump targets the card's displayed exact/bounded date.

**Files:** [PeaceDealsWidget.tsx](PeaceDealsWidget.tsx) renders shared LogList/FittedList cards; [peaceData.ts](peaceData.ts) groups, filters and projects rows/tooltips, with adjacent tests. Shared [usePeaceDeals.ts](../../dashboard/usePeaceDeals.ts) loads/validates. `peaceDeals.css`, `definition.ts` and `index.ts` own styling, settings and registration.

Shared framework conventions: [widgets](../README.md).
