# Command power spending

## What it shows

A compact table of recorded command-power payments (Date, Item, Category, Amount, and Country when several countries are chosen), with category chips and a one-line summary of what was spent and gained in the shown period.

## Products it reads

[`politics-ledger`](../../../../crates/dashboard/src/products/politics_ledger.rs) (`balance` view, shared with the Command power chart) through the [shared spending consumer](../../dashboard/usePoliticsSpending.ts) and [spending table](../../dashboard/PoliticsSpendingTable.tsx).

## Settings

Countries, campaign/follow/pin/range and the shared [spending options](../../dashboard/politicsSpendingOptions.ts): game artwork, filter chips, categories, summary, decimals, cell bars, density and sort. Follow is the default.

## Notable behaviour

- Ability upkeep and daily deficits fold into one row per country, category and game month; every other payment is its own row. Amounts are whole unless Decimals asks for more.
- Payments the recording could not name, such as raids and preferred tactics, are named by their kind. Gained counts the daily gain only; a deficit is spending.
- Command power set aside for raids, assignments and abilities is not spending. The game's reserve counter can sit below zero, so its level is never shown as an amount held; the Command power chart shows what moved into and out of reserve each day, and its dashed maximum already reflects the reserve as the game displays it.
- Unrecorded stretches are one note; they never hide recorded payments. Names and flags use the action hour.

## Files

| File | Responsibility |
|---|---|
| [CommandPowerSpendingWidget.tsx](CommandPowerSpendingWidget.tsx) | CP adapter for the shared spending table. |
| [definition.ts](definition.ts), [index.ts](index.ts) | Metadata, shared options and category choices. |
