# Country balance widgets

## What it shows

Shared Political power/Command power balances and income/spending drills; this folder has no registered kind.

## Products it reads

[`country-measure`](../../../../crates/dashboard/src/products/country_measure.rs) supplies balances; [`politics-ledger`](../../../../crates/dashboard/src/products/politics_ledger.rs) supplies accounting/payments; [`country-stability`](../../../../crates/dashboard/src/products/country_stability.rs) supplies CP maximum. Looks/groups use the [shared kit](../README.md#reuse-first).

## Settings

[options.ts](options.ts) owns shared country, time, ticks, fill, end-label, axis and drill choices. Each registered kind supplies a MeasureConfig and its definition.

## Notable behaviour

- Exact signed daily operands, evaluated adjustments and realized balance changes keep independent supplying dates. Later reads never backfill a tooltip; detail failure leaves balances usable.
- Captured payments can be hidden by income or clamping. Incomplete capture falls back to explicitly net drops. CP reservations and daily deficits are outside payment ticks.
- Income sources not captured remain Other; native wrapping appears as Adjustment. Missing totals have no percentages. Each drill shows one total with a concise status and missing values in their part rows; adjustment action details and CP reservations remain below it. Gaps and holder changes break accounting/history.
- Drawn curves, including visible maximum, supply picks. Overlap/keyboard choices hold the date. Scope/numeric changes invalidate gestures; label refresh retains ownership.
- Picking holds; Follow uses the aligned clock from any time mode. Pins save native slot/ID and branch-relative origin, following only that holder's aliases. Headers use the body date; invalid holders refuse the body and older missing-holder pins stay neutral.
- Names and flags are dated; only mounted legend rows and the displayed drill country request artwork.

## Files

| File | Responsibility |
|---|---|
| [MeasureWidget.tsx](MeasureWidget.tsx), [measureData.ts](measureData.ts) | Chart, validation and local lookups. |
| [PowerDrill.tsx](PowerDrill.tsx), [powerDrillData.ts](powerDrillData.ts) | Signed parts and history. |
| [powerActorPick.ts](powerActorPick.ts), [powerPickModel.ts](powerPickModel.ts) | Actor picks and ownership. |
| [drillCountry.ts](drillCountry.ts), [powerCountryData.ts](powerCountryData.ts) | Shared native bindings and alias-ledger joins. |
| [options.ts](options.ts), [measure.css](measure.css) | Options/layout; adjacent helper/render tests. |
