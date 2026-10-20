# Governments in exile

## What it shows

Every government in exile at the playback date with its host, legitimacy, exile manpower against its cap, deployed air crews and recruited army leaders, plus an "N in exile" count in the header. All countries by default; chosen countries or groups keep the exiles that are, or are hosted by, one of them.

## Products it reads

- [`politics-state-history`](../../../../crates/dashboard-politics/src/products/politics_state/history.rs) in `exiles` mode, one week of hours per request: each exile holder's tags, host holder, legitimacy, manpower, cap, leader resources and supplying read hour as runs, and every host holder's tags. Following or pinning only looks the runs up locally ([politicsHistory.ts](../../dashboard/politicsHistory.ts)).
- [`politics-state`](../../../../crates/dashboard-politics/src/products/politics_state.rs) in `exiles` view, one uncached request for the shown hour while exiles are shown: deployed air crews are an hour detail, joined by exile tag.

## Settings

Follow/pin, countries/groups and the shared table options: Sort by (Manpower descending by default), Cell bars, Rank change markers and Density. There is no Live switch.

## Notable behaviour

- A country is in exile at an hour a recorded host covers; it appears and leaves with that status. No exile start date is recorded, so there is no "since" date or Jump action.
- Fields and wording follow the exile cards in [country people](../country-people/README.md): Host, Legitimacy, Exile manpower, Exile manpower cap, Deployed air crews, Army leaders recruited. Values are exact and in the shared number format. A value that is not recorded shows an em dash, sorts last and reads "Not recorded" on hover; zero is a recorded value. A host without a tag at the hour reads "Not recorded" in its cell, sorts last and is counted in diagnostics.
- Air crews show an em dash while the hour detail loads; hover says "Not recorded" only when the detail says so.
- Hover adds "Exile resources last read <date>" when the supplying read is older than the shown hour. Legitimacy has no threshold colour.
- An hour past the prepared history, or whose group membership is still being prepared, shows that it is still being prepared, never an empty table; an hour the exiles were not read says so.

## Files

| File | Responsibility |
|---|---|
| [GovernmentsInExileWidget.tsx](GovernmentsInExileWidget.tsx) | Shared table, meters, hover, header count, hour detail and states. |
| [governmentsInExileData.ts](governmentsInExileData.ts) | Validation, local lookup at an hour, narrowing, rows and sort operands; adjacent tests. |
| [definition.ts](definition.ts), [index.ts](index.ts), [governmentsInExile.css](governmentsInExile.css) | Options, registration and layout. |
