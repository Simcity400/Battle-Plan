# Country balance widgets

## What it shows

Shared Political power/Command power balances and income/spending drills; this folder has no registered kind.

## Products it reads

[`country-measure`](../../../../crates/dashboard/src/products/country_measure.rs) supplies balances; [`politics-ledger`](../../../../crates/dashboard/src/products/politics_ledger.rs) supplies accounting/payments; [`country-stability`](../../../../crates/dashboard/src/products/country_stability.rs) supplies CP maximum. Looks/groups use the [shared kit](../README.md#reuse-first).

## Settings

[options.ts](options.ts) owns shared country, time, ticks, fill, end-label, decimals, axis and drill choices. Each registered kind supplies a MeasureConfig and its definition.

## Notable behaviour

- Balances, maxima, payments and end labels are whole numbers unless Decimals asks for more; daily rates keep the two decimals the game shows. Only the writing rounds.
- A day's tooltip: the balance, the CP maximum, the daily change (PP: gain and daily costs beneath it; CP: daily gain or deficit), gain lost at the cap, each one-off payment by name or kind, and, for CP, what moved into or out of reserve. No hour stamps, zero rows or repeated changes. The reserve counter can sit below zero, so only its changes are shown. Beyond four countries it lists balances only.
- Markers show one-off payments, including ones hidden by income or clamping; costs charged every day and daily deficits get none. Incomplete capture falls back to an explicit “Balance fell”. Exact signed operands keep independent supplying dates; later reads never backfill a tooltip, and detail failure leaves balances usable.
- The drill's income tab shows the daily gain, daily costs and native wrapping (Adjustment) with one total and no shares; its spending tab lists the day's one-off payments and one row per daily-cost category. Below them it names gain lost at the cap and CP reserve moves. Missing totals have no percentages; gaps and holder changes break accounting/history.
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
