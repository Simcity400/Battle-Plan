# Political power spending breakdown

## What it shows

The recorded political-power payments of the shown period as a sunburst, category → specific item, with “Spent” and the total in the centre and a note comparing it with what was gained.

## Products it reads

[`politics-ledger`](../../../../crates/dashboard-politics-ledger/src/products/politics_ledger.rs) (`spending` view) through [usePoliticsSpending](../../dashboard/usePoliticsSpending.ts); [politicsSpending.ts](../../dashboard/politicsSpending.ts) owns exact grouping and [powerBalance.ts](../../dashboard/powerBalance.ts) the gain.

Prepared clients request per-item totals and latest matched facts for the selected native-date window. Current and final totals stay separate for Share/Fill, and aggregate payment counts need no list of contributing day IDs.

## Settings

Countries, campaign/follow/pin/range, category/item level toggles, Decimals and the shared sunburst options. Whole campaign is the default.

## Notable behaviour

- Everything recorded draws. Amounts are the evaluated charges, written in whole numbers unless Decimals asks for more; totals, shares and slices use the exact amounts. A slice whose cost was partly not recorded is marked partial.
- Unrecorded time is named in the note (how long and on which dates) and is never a slice or legend row. Shares are of the recorded spending.
- The note also gives the game's daily gain over the same period and any of it lost at the cap. With nothing spent, the widget says so and gives the same note.
- A payment the recording could not name sits in its category ring without an item slice; a category with no recorded cause is “Other spending”.
- Law, advisor and idea items keep game-style effects and targets at their most recent matched action, whose date the tooltip names.

## Files

| File | Responsibility |
|---|---|
| [PoliticalPowerSpendingBreakdownWidget.tsx](PoliticalPowerSpendingBreakdownWidget.tsx) | Shared spending-to-Sunburst adapter; adjacent renderer test. |
| [definition.ts](definition.ts), [index.ts](index.ts) | Levels, decimals, common options and registration. |
