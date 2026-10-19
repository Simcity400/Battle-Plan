# Trade flows

## What it shows

The View setting switches between resource flows and Market & lend-lease. The Market & lend-lease catalogue card adds this same Trade flows widget with that view selected; every copy can switch views in settings.

Resource trade as seller â†’ buyer ribbons, with a complete contributor drill for nodes and links, and a view of one country's whole trade from its flag or name.

## Products it reads

[`trade-flows`](../../../../crates/dashboard-resource-balance/src/products/trade_flows.rs) supplies sparse native-holder flows, coverage and resource content. Preparation observes every country through the same trade step and allocates seller/buyer/resource keys only for observed flows. Its native sampler feeds both the observer and progressive current histories. Unprepared histories draw approximate keyframe spans through the shared stack and clipping renderer; zooms of at most thirteen unprepared blocks decode exactly. Exact reads supply written amounts and holder aliases. Total/average measures, cumulative coverage counts and first/last trade dates depend on earlier frames and stay pending until preparation is complete. Fact events and signed cumulative endpoints answer those quantities without decoding history. Dense histories use the shared D-12 stack and per-flow envelope scale; exact windows retain the existing geometry and scale sweep. A country view outside the chosen countries reads the same product for that country's own tags. Shared country facts provide dated labels, flags and colours.

The [`market-lendlease`](../../../../crates/dashboard-resource-balance/src/products/market_lendlease.rs) product supplies market offers, inventory, asking prices, subsidies, automation, request/contract equipment and price terms, and lend-lease schedules, dates and delivery/loss occurrences. This moment view uses the same campaign, native country selection, time settings and Follow/Pin drill host. Equipment deliveries/losses are changes in the game's retained totals for the recorded hour, including final increments copied before agreement reset/removal. Fuel and CIC keep their explicit native operands. Missing tables remain unknown.

## Settings

View, all/manual/group countries, campaign/follow/pin/range; resource flows additionally offers current/total/average, resource/seller colour, resource filter, touching/between selection, countries per side, scale and drill choices.

## Notable behaviour

- Current is per day; totals/averages count recorded hours and expose gaps/partial deals. Average divides by each flowâ€™s active trading duration, not the full campaign. Equipment-market contracts are excluded.
- Native slot/id holders keep one history through renames; reused tags create separate nodes. Manual/group filtering resolves holders at membership dates. Missing identities never borrow future aliases.
- Fixed country colours use the full native census; resource colours use full content order, independent of filters/ranks. Resource names come from content, with Resource name unavailable for missing names and ordinals retained. Only drawn actors request art. Other nodes sum every hidden holder, including nonpositive contributors and internal flows.
- Signed totals/parts and missing reasons remain exact; geometry never substitutes for a full total. Keep sparse prefix histories and the shared compact stack, without a country Ã— hour grid.
- Drills save holders/resource with campaign/branch scope. Picking holds; Follow uses the aligned clock. Pin keeps cohort, counting/history extent and branch-relative date; a country header follows the body's date/holder or stays unavailable. Source/numeric changes invalidate picks.
- A country's flag or name (or its node's menu, from the keyboard) opens its imports and exports: totals, then by resource and by partner, for the widget's measure at the drill day, with what was delivered over the time shown; and the same over time, a band per resource, exports above the axis and imports below. Its resource filter applies; its country choice does not. Recorded zero deals stay listed; unrecorded days stay unknown. Pinning it pins that country.
- Oversized replies refuse explicitly; no chosen country/contributor cap.

## Files

| File | Responsibility |
|---|---|
| [TradeFlowsWidget.tsx](TradeFlowsWidget.tsx), [useTradeFlows.ts](useTradeFlows.ts) | Diagram, products and filters. |
| [tradeData.ts](tradeData.ts), [tradeDrill.ts](tradeDrill.ts), [TradeDrillView.tsx](TradeDrillView.tsx) | Exact sparse measures, cohort pins and histories. |
| [tradeCountry.ts](tradeCountry.ts), [TradeCountryView.tsx](TradeCountryView.tsx) | One country's imports and exports, by resource and partner, and over time. |
| [marketData.ts](marketData.ts), [TradeMarketView.tsx](TradeMarketView.tsx) | Exact market requests, validation, fixed-point formatting and presentation. |
| [tradeColours.ts](tradeColours.ts), [tradeTooltip.ts](tradeTooltip.ts) | Stable colours and fitted dated tooltips; adjacent tests. |
| [definition.ts](definition.ts), [index.ts](index.ts), [tradeFlows.css](tradeFlows.css) | Options, registration and layout. |

The default view is Resource flows. Market & lend-lease reads only its own moment product; resource-only settings are hidden in that view. Selecting an offer, contract or agreement opens its details through the shared inline/pop-out drill and its Follow/Pin controls. Equipment, fuel and CIC keep their own units. Saved contract metadata, variable stores and route accounting remain available through the replay row API. Local action drafts are not recorded.

A closed market drill uses the widget's Pin, Campaign, Range endpoint or Follow
time. Only an open drill overrides that moment. Pin stores its selected offer,
request/contract or agreement in the shared campaign/branch-scoped trade selection
and restores that breakdown after reload, with its branch-relative time and Follow
choice intact.

The complete delivery-event ledger includes explicit holder-level amount refusals. Missing delivery
amounts appear as named unavailable amounts for the affected agreement or
contract, rather than as zero or `No deliveries observed`.
