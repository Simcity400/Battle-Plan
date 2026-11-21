# 1204. Preserve trade coverage and country identity in drill-downs

Trade history is read in bounded windows and joined after all windows arrive. The read extent belongs to the branch or the saved widget range, so moving the playhead does not request the history again. Narrow country choices request every recorded tag of their native holders. Hourly groups further restrict those flows to the hours with recorded membership. Resource choices use the recorded catalogue, with one entry per token.

A drill holds the native countries selected in the diagram. The Other group includes all holders outside the displayed cohort, including idle holders and signed corrections, while a single-country drill reads that holder's purchases and sales. A pinned drill retains the history and accumulation origin as branch-relative offsets. Opening it starts at the displayed hour. Selecting Follow resumes the dashboard clock. The header resolves the holder from the selection and its recorded identity history, so tag reuse cannot relabel a pinned drill.

Within recorded coverage, a missing deal is a known zero. Missing deal, catalogue or membership coverage breaks the history. Cumulative totals retain earlier recorded trade across the gap. Bar and hover values use the original signed amounts, while filled geometry displays positive values. A zero crossing is a sweep boundary, which lets the display clamp the curve without changing selected values.

The geometry uses native run prefixes and a scalar stack guide. Totals are linear between run boundaries and zero crossings. For averages, grouping denominators lets the sweep combine changing terms and carry constant terms without evaluating each ratio at each boundary. An envelope of per-flow peaks bounds interior rises, including the first positive interval before the origin hour. The chart uses interval bounds to choose visible segments and expands all edges only at the endpoints emitted by pixel thinning. Exact values at a selected hour come from the original prefixes.

[Trade queries](../../ui/src/widgets/trade-flows/useTradeFlows.ts) · [Drill data](../../ui/src/widgets/trade-flows/tradeDrill.ts)
