# Trade flows

## What it shows

Resource trade as seller → buyer ribbons, with a complete contributor drill for nodes and links.

## Products it reads

[`trade-flows`](../../../../crates/dashboard/src/products/trade_flows.rs) supplies sparse native-holder flows, coverage and resource content. Shared country facts provide dated labels, flags and colours.

## Settings

All/manual/group countries, campaign/follow/pin/range; current/total/average, resource/seller colour, resource filter, touching/between selection, countries per side, scale and drill choices.

## Notable behaviour

- Current is per day; totals/averages count recorded hours and expose gaps/partial deals. Average divides by each flow’s active trading duration, not the full campaign. Equipment-market contracts are excluded.
- Native slot/id holders keep one history through renames; reused tags create separate nodes. Manual/group filtering resolves holders at membership dates. Missing identities never borrow future aliases.
- Fixed country colours use the full native census; resource colours use full content order, independent of filters/ranks. Resource names come from content, with Resource name unavailable for missing names and ordinals retained. Only drawn actors request art. Other nodes sum every hidden holder, including nonpositive contributors and internal flows.
- Signed totals/parts and missing reasons remain exact; geometry never substitutes for a full total. Keep sparse prefix histories and the shared compact stack, without a country × hour grid.
- Drills save holders/resource with campaign/branch scope. Picking holds; Follow uses the aligned clock. Pin keeps cohort, counting/history extent and branch-relative date; a country header follows the body's date/holder or stays unavailable. Source/numeric changes invalidate picks.
- Oversized replies refuse explicitly; no chosen country/contributor cap.

## Files

| File | Responsibility |
|---|---|
| [TradeFlowsWidget.tsx](TradeFlowsWidget.tsx), [useTradeFlows.ts](useTradeFlows.ts) | Diagram, products and filters. |
| [tradeData.ts](tradeData.ts), [tradeDrill.ts](tradeDrill.ts), [TradeDrillView.tsx](TradeDrillView.tsx) | Exact sparse measures, cohort pins and histories. |
| [tradeColours.ts](tradeColours.ts), [tradeTooltip.ts](tradeTooltip.ts) | Stable colours and fitted dated tooltips; adjacent tests. |
| [definition.ts](definition.ts), [index.ts](index.ts), [tradeFlows.css](tradeFlows.css) | Options, registration and layout. |
