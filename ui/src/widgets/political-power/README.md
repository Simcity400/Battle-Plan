# Political power

## What it shows

Country political-power balance lines with markers for one-off payments and a short day tooltip: balance, daily change (gain and daily costs) and that day's payments.

## Products it reads

[`country-measure`](../../../../crates/dashboard/src/products/country_measure.rs) (`politicalPower`) supplies balances; [`politics-ledger`](../../../../crates/dashboard/src/products/politics_ledger.rs) supplies daily accounting and matched payments. Shared looks/groups follow the [widget contract](../README.md#contract).

## Settings

Countries, campaign/follow/pin/range and shared [balance options](../country-measure/options.ts): playback/window, ticks, fill, end labels, decimals, axes and drills. Whole campaign is the default.

## Notable behaviour

- Uses [country-measure](../country-measure/README.md); common chart, comparison picks and Pin/Follow behaviour belong there.
- Runs break at gaps, missing values and replacement holders. A fallback drop needs consecutive reads of the same country and remains explicitly net.
- Amounts are whole unless Decimals asks for more; daily rates keep two decimals. Captured evaluated charges stay separate from realized balance changes, and gain the cap took is named. Later reads never fill earlier unknowns.
- Names, flags, end labels and membership markers use their own recorded hours.

## Files

| File | Responsibility |
|---|---|
| [definition.ts](definition.ts) | Metadata and shared settings. |
| [index.ts](index.ts) | Configures MeasureWidget for PP. |
