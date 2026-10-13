# Political power spending

## What it shows

A compact table of recorded political-power payments: Date, Item (game artwork and name), Category (swatch and name) and Amount, with a Country column when several countries are chosen. Category chips and a one-line summary (spent and gained in the shown period, and any gain lost at the cap) sit above it.

## Products it reads

[`politics-ledger`](../../../../crates/dashboard/src/products/politics_ledger.rs) (`balance` view, shared with the Political power chart) through [usePoliticsSpending](../../dashboard/usePoliticsSpending.ts). [PoliticsSpendingTable.tsx](../../dashboard/PoliticsSpendingTable.tsx) renders both spending tables with the shared [table kit](../../charts/table/README.md); [politicsSpending.ts](../../dashboard/politicsSpending.ts) owns rows, folding and wording, and [powerBalance.ts](../../dashboard/powerBalance.ts) the daily gain.

## Settings

Countries, campaign/follow/pin/range and the shared [spending options](../../dashboard/politicsSpendingOptions.ts): game artwork, filter chips, categories, summary, decimals, cell bars, density and sort. Follow is the default; the default order is newest first. A layout saved with the earlier log keeps its oldest-first or largest-first order.

## Notable behaviour

- One row per payment, dated at the native action hour and shown from its supplying read. Costs the game charges every day (daily cost modifiers, organisation upkeep) fold into one row per country, category and game month, with the days counted and the items listed in its tooltip.
- Amounts are the exact evaluated charges, written in whole numbers unless Decimals asks for more. A payment whose cost wasn't recorded has no amount and is counted in a note, never as zero; a partly recorded amount is marked partial.
- Unrecorded stretches are one plain note (how long, and on which dates), not rows; they never hide what was recorded. Spent counts every category; a category filter adds the shown amount.
- Gained is the game's daily gain before daily costs, over the shown period. Gain the cap took is shown separately.
- A payment the recording could not name is named by its kind (its category), never “Not identified”; a category with no recorded cause is “Other spending”.
- Tooltips give the cost, target, payment count and the game-style effects of laws, advisors and ideas at the action hour. Flags and names are requested only for mounted rows, each at its own date, however many dates they span; the menu jumps to the row's date.

## Files

| File | Responsibility |
|---|---|
| [PoliticalPowerSpendingWidget.tsx](PoliticalPowerSpendingWidget.tsx) | PP adapter for the shared spending table. |
| [definition.ts](definition.ts), [index.ts](index.ts) | Metadata, shared settings and category choices. |
