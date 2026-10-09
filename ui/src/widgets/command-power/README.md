# Command power

## What it shows

Country command-power balances over time, with payment ticks and an optional dashed maximum.

## Products it reads

[`country-measure`](../../../../crates/dashboard/src/products/country_measure.rs) (`commandPower`) supplies balances; [`politics-ledger`](../../../../crates/dashboard/src/products/politics_ledger.rs) supplies income, payments and reservations. [`country-stability`](../../../../crates/dashboard/src/products/country_stability.rs) supplies the native maximum. Shared country looks and groups follow the [widget contract](../README.md#contract).

## Settings

Countries, campaign/follow/pin/range, shared [balance options](../country-measure/options.ts), and Maximum. Whole campaign is the default.

## Notable behaviour

- Uses the [shared balance renderer](../country-measure/README.md); change common chart and drill behaviour there.
- The maximum loads independently; missing operands leave it unknown.
- Current CP holds are reservations, separate from spending. Allocations and releases never become payment ticks. Negative daily gain is Daily deficit.
- Missing ledger capture retains explicitly net balance-drop ticks. Named payments retain evaluated charges separately from realized changes.

## Files

| File | Responsibility |
|---|---|
| [definition.ts](definition.ts) | Metadata and maximum toggle. |
| [index.ts](index.ts) | Configures the shared MeasureWidget for CP. |
