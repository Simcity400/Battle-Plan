# Country balance widgets

## What it shows

Shared Political power/Command power balances and income/spending drills; this folder has no registered kind.

## Products it reads

[`country-measure`](../../../../crates/dashboard-base/src/products/country_measure.rs) supplies balances; [`politics-ledger`](../../../../crates/dashboard-politics-ledger/src/products/politics_ledger.rs) supplies accounting/payments; [`country-stability`](../../../../crates/dashboard/src/products/country_stability.rs) supplies CP maximum. Looks/groups use the [shared kit](../README.md#reuse-first).

## Settings

Prepared-query clients supply selected-country balance charts with width-bounded summaries and exact reached hours for the playhead, hover and visible line ends. Campaign, Follow, Pin and range time modes share these queries. Chart drawing and actor picks share summary geometry; picks require an exact known balance. Balance changes open a windowed exact event list with stable decimal ordinals, duplicate dates and playback cutoff filtering. Group selection and income/spending-only breakdowns use the history products. The command-power ceiling shares prepared stability series and exact reached reads. Accounting and maximum details load when hover or drill reaches them.

Pending preparation is separate from recorded missing values. Balances merge
committed prepared runs with transient samples for the unprepared tail. A wide
view uses each unprepared block's keyframe; faded dashed spans and one Approximate
key identify that geometry. A viewport touching at most thirteen unprepared
blocks reads every frame for exact values. State requests select their distinct
blocks and retain only reached hours. The stateless balance sampler also supplies
the preparation observer; none of these direct reads are stored.

Mounted requests publish their first reply immediately and replace it only when
committed branch coverage changes. Decode counters and phase changes do not
re-read the same prefix. An unchanged or regressing partial reply stops that
request while retaining its published data; polling always yields through a real
pause, even with an immediately resolving test wait. Effect keys use the viewport
and normalized reached hours rather than reply-object identities.
The timeline marks the committed exact
extent with recorded gaps excluded. Balance changes show only prepared events
and one dated note explaining that the rest is still being prepared. Cancellation
stops polling; failures retain the current partial chart/list with the actual
error, and Retry can restart preparation.

[options.ts](options.ts) owns shared country, time, ticks, fill, end-label, decimals, axis and drill choices. Each registered kind supplies a MeasureConfig and its definition.

## Notable behaviour

- Balances, maxima, payments and end labels are whole numbers unless Decimals asks for more; daily rates keep the two decimals the game shows. Only the writing rounds.
- A day's tooltip: the balance, the CP maximum, the daily change (PP: gain and daily costs beneath it; CP: daily gain or deficit), gain lost at the cap, each one-off payment by name or kind, and, for CP, what moved into or out of reserve. No hour stamps, zero rows or repeated changes. The reserve counter can sit below zero, so only its changes are shown. Beyond four countries it lists balances only.
- Markers show one-off payments, including ones hidden by income or clamping; costs charged every day and daily deficits get none. Incomplete capture falls back to an explicit “Balance fell”. Exact signed operands keep independent supplying dates; later reads never backfill a tooltip, and detail failure leaves balances usable.
- The drill's income tab shows the daily gain, daily costs and native wrapping (Adjustment) with one total and no shares; its spending tab lists the day's one-off payments and one row per daily-cost category. Below them it names gain lost at the cap and CP reserve moves. Missing totals have no percentages; gaps and holder changes break accounting/history.
- Prepared income and cause history uses width summaries; exact operands are requested only for the selected and hovered days. Capture instances retain separate daily tuples, while a recorded country follows its aliases. Multiple supplied income identities in one native day refuse that day's income history.
- Payment drawing, pointer snapping and keyboard stops admit event dates at their qualifying supplying read.
- Drawn curves, including visible maximum, supply picks. Overlap/keyboard choices hold the date. Scope/numeric changes invalidate gestures; label refresh retains ownership.
- Picking holds; Follow uses the aligned clock from any time mode. Pins save native slot/ID and branch-relative origin, following only that holder's aliases. Headers use the body date; invalid holders refuse the body and older missing-holder pins stay neutral.
- Names and flags are dated; only mounted legend rows and the displayed drill country request artwork. Closed breakdown regions keep their neutral title; an open breakdown names its country at the displayed date.

- Monthly line-style reads batch the complete country selection through the shared product request size; all replies and their diagnostics merge before replacing the held styles.

## Files

| File | Responsibility |
|---|---|
| [MeasureWidget.tsx](MeasureWidget.tsx), [measureData.ts](measureData.ts) | Chart, validation and local lookups. |
| [../../dashboard/preparedSeries.ts](../../dashboard/preparedSeries.ts), [preparedMeasure.ts](preparedMeasure.ts), [preparedMeasureData.ts](preparedMeasureData.ts), [PreparedDropList.tsx](PreparedDropList.tsx) | Owned bounded summaries, exact reached intervals and paged balance changes. |
| [PowerDrill.tsx](PowerDrill.tsx), [powerDrillData.ts](powerDrillData.ts) | Signed parts and history. |
| [preparedPowerParts.ts](preparedPowerParts.ts) | Width summaries, reached daily operands, alias bindings and shared navigation. |
| [powerActorPick.ts](powerActorPick.ts), [powerPickModel.ts](powerPickModel.ts) | Actor picks and ownership. |
| [drillCountry.ts](drillCountry.ts), [powerCountryData.ts](powerCountryData.ts) | Shared native bindings and alias-ledger joins. |
| [options.ts](options.ts), [measure.css](measure.css) | Options/layout; adjacent helper/render tests. |
