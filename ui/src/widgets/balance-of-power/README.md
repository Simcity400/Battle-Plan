# Balance of power

## What it shows

One country (the dashboard's by default, or one picked): the balance the game shows it, named in a header line ("<range> · −0.34 · toward <side>"), its value over its lifetime on gridlines at the authored range bounds, and one lane per balance the country has had. All countries, a group or several countries: a table of every balance active at the playback date. Page and catalogue group: Politics.

## Products it reads

[`politics-state-history`](../../../../crates/dashboard-politics/src/products/politics_state/history.rs), prepared, once per branch: `country` mode for one country (every holder of its tag; the view follows the one holding it at the date, through renames, and names it, finds its members and drills its weekly change by the tag it held at each hour), `balances` mode for the table. Hours the balances were not read are unknown, like gaps. Playback looks the runs up locally ([balanceOfPowerData.ts](balanceOfPowerData.ts), [politicsHistory.ts](../../dashboard/politicsHistory.ts)). [`politics-state`](../../../../crates/dashboard-politics/src/products/politics_state.rs) `balances` is a transient hour detail: the drilled day's weekly change, or, once playback rests, the members of a shared balance.

## Settings

Show (Balance / Weekly change, also a catalogue card), Balance (a content choice; automatic is the one the game shows), the playback options and Breakdown opens for one country; Sort by (Value, strongest lean first), Cell bars, Rank change markers and Density for the table. Campaign, follow, pin and range; Live.

## Notable behaviour

- Every name, range, bound and side comes from the reply; unresolved names read "Balance name unavailable", "Range name unavailable" or "Side name unavailable" and are counted in diagnostics. Keys show only on hover.
- Gaps stay gaps: the line breaks, the lanes use the timeline kit's Held before / Ended by / Last recorded wording, and hours still being prepared draw the pending band. Gaps, hours not read and hours still being prepared are merged first, so a gap inside a longer unread stretch never ends a balance early. At an hour the balances were not read, the table and the one-country view say "The balance of power was not recorded at this date." and the header count is hidden, never "none active"; with hours not read before it, a country's absence reads "has no balance of power recorded".
- Live cuts lanes and balances at the playback date ("<Country> has no balance of power yet."). Outside the shown balance's lifetime the header reads "Starts <date>" or "Ended <date> in <range>".
- A click on the line opens the weekly change at that day: the derived sources as signed exact parts with their total, and each member's recorded weekly and daily contribution with theirs; the two totals are independent and never reconciled. Sources that cannot be derived read unavailable. An hour the balances were not read reads "not recorded"; until the history covers the date the country's holder is chosen at and the drilled day it is named at, the weekly change reads "still being prepared", never "none active". Follow, Pin as widget.
- A lane's menu offers Show this balance, which saves Balance. Holders the same stored record lists at the date share a row; equal values never join records; a row click opens the matching member's one-country view in the widget, with Back.

## Files

| File | Responsibility |
|---|---|
| [BalanceOfPowerWidget.tsx](BalanceOfPowerWidget.tsx) | One-country view with its breakdown, or the table. |
| [BalanceCountryView.tsx](BalanceCountryView.tsx) | Header line, chart and lanes for one country's balance. |
| [BalanceDrill.tsx](BalanceDrill.tsx), [useBalanceDetails.ts](useBalanceDetails.ts) | Weekly change parts and the transient hour detail. |
| [BalanceTable.tsx](BalanceTable.tsx) | All countries table and the row view. |
| [balanceOfPowerData.ts](balanceOfPowerData.ts) | Validation, local lookups, lanes, rows and parts; adjacent tests. |
| [definition.ts](definition.ts), [index.ts](index.ts), [useBalanceChoices.ts](useBalanceChoices.ts), [balanceOfPower.css](balanceOfPower.css) | Settings, registration, Balance choices and layout. |
