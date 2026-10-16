# Command power

## What it shows

Country command-power balances over time, with markers for one-off payments, an optional dashed maximum and a short day tooltip.

## Products it reads

[`country-measure`](../../../../crates/dashboard-base/src/products/country_measure.rs) (`commandPower`) supplies balances; [`politics-ledger`](../../../../crates/dashboard-politics-ledger/src/products/politics_ledger.rs) supplies income, payments and reservations. [`country-stability`](../../../../crates/dashboard/src/products/country_stability.rs) supplies the native maximum. Shared country looks and groups follow the [widget contract](../README.md#contract).

## Settings

Countries, campaign/follow/pin/range, shared [balance options](../country-measure/options.ts) (including Decimals), and Maximum. Whole campaign is the default.

## Notable behaviour

- Uses the [shared balance renderer](../country-measure/README.md); change common chart and drill behaviour there.
- The maximum loads independently; missing operands leave it unknown.
- Command power set aside for raids, assignments and abilities is not spending and gets no marker. The game's reserve counter can sit below zero, so the tooltip shows only what moved into or out of reserve that day; the maximum already reflects the reserve as the game displays it. Negative daily gain is Daily deficit.
- Missing ledger capture retains explicitly net balance-drop ticks. Named payments retain evaluated charges separately from realized changes.

## Files

| File | Responsibility |
|---|---|
| [definition.ts](definition.ts) | Metadata and maximum toggle. |
| [index.ts](index.ts) | Configures the shared MeasureWidget for CP. |
