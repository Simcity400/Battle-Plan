# Command power spending

## What it shows

Recorded CP payments and daily deficits as a dated log, with current reservations beside the totals.

## Products it reads

[`politics-ledger`](../../../../crates/dashboard/src/products/politics_ledger.rs) through the [shared spending consumer](../../dashboard/usePoliticsSpending.ts).

## Settings

Countries and campaign/follow/pin/range; shared [spending options](../../dashboard/politicsSpendingOptions.ts) control categories, order, artwork, totals and log presentation. Follow is the default.

## Notable behaviour

- Recurring upkeep groups by country and day, but contributions enter only at their supplying reads. Hourly group membership and ranges select the native payment hours; undated older aggregates cannot supply a filtered subtotal.
- Evaluated charges and realized balance changes stay distinct. Daily deficit is separate from real costs.
- CP held remains visible with running totals off. Allocations/releases never add to spending; missing reservation baselines or coverage keep holds unknown.
- Names and flags use the action hour; coverage-gap rows use their gap start. Later reads never repair an earlier unknown.

## Files

| File | Responsibility |
|---|---|
| [CommandPowerSpendingWidget.tsx](CommandPowerSpendingWidget.tsx) | CP adapter for shared PoliticsSpendingLog. |
| [definition.ts](definition.ts), [index.ts](index.ts) | Metadata, shared options and category choices. |
