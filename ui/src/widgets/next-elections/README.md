# Next elections

## What it shows

Every country whose recorded game predicate says it holds elections at the playback date: how far through its term it is, the days left and the date it votes next. All countries by default; chosen countries or groups narrow the table.

## Products it reads

[`politics-state-history`](../../../../crates/dashboard-politics/src/products/politics_state/history.rs) in `elections` mode, once per branch: each country holder's tags, election eligibility, signed interval, next date and term start as coalesced runs. Following or pinning only looks the runs up locally ([politicsHistory.ts](../../dashboard/politicsHistory.ts)). Dated country names and flags load for mounted rows; a Country sort asks for every row's name.

## Settings

Follow/pin, countries/groups and the shared table options: Sort by (Days left ascending by default), Cell bars, Rank change markers and Density. There is no Live switch.

## Notable behaviour

- Countries known not to hold elections are hidden. A country read at the hour whose eligibility was not is neither listed nor shown as not holding them; it is counted in diagnostics.
- Days left is the next election's hour less the shown hour, counting a part day as a day. The Term meter is the share of the term passed, drawn only when the interval, next date and term start are all recorded at the hour; otherwise the meter is empty. A next date that is not recorded shows an em dash, sorts last and reads "Not recorded" on hover.
- The election interval is on hover. The schedule is read every hour, so its values are current at the playback date and the hover carries no separate read date.
- The menu offers Jump to the next election when that date lies inside the recording. There is no row click.
- An hour past the prepared history, or whose group membership is still being prepared, shows that it is still being prepared, never an empty table.

## Files

| File | Responsibility |
|---|---|
| [NextElectionsWidget.tsx](NextElectionsWidget.tsx) | Shared table, meters, hover and states. |
| [nextElectionsData.ts](nextElectionsData.ts) | Validation, local lookup at an hour, rows and sort operands; adjacent tests. |
| [definition.ts](definition.ts), [index.ts](index.ts), [nextElections.css](nextElections.css) | Options, registration and layout. |
