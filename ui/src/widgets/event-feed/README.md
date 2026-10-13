# Event feed

## What it shows

A world log of Wars & peace, Diplomacy, Politics and Milestones, newest first with dated country labels and optional game artwork. Kinds the game gives no picture (wars, peace, peace conferences, releases, state changes, factions, guarantees, ruling parties) show their own icon on a tile in the category's colour (`EVENT_ICONS` in [definition.ts](definition.ts)), as do pictured kinds whose artwork is missing.

## Products it reads

[`event-feed`](../../../../crates/dashboard/src/products/event_feed.rs) supplies window discovery and event pages composed from topic histories. [`place-names`](../../../../crates/dashboard/src/products/place_names.rs) names the states of state changes and nuclear strikes at their event hour.

## Settings

All countries by default, or manual/groups; campaign/follow/pin/range, category toggles/chips, artwork and shared log options.

## Notable behaviour

- Every recorded event is listed with its day: the game's own date, or the day of the read that first shows it when the reads either side are consecutive. A break in the recording, or the time before it starts, lists nothing; a focus completed inside a break is counted, not listed. Rows become visible at their supplying read. Technology completions are excluded. Initial snapshots and lost territory do not invent events/causes.
- Country matches use each row’s identity/membership hour; departures use the last recorded member hour. Literal choices expand to native holders/aliases, never unrelated reuse.
- Native declarations, joins, civil-war transfers, annexations, ended wars, releases, state owner changes and peace conference starts retain actor/target roles independently with exact event-hour ID/instance witnesses. A recorded cause says how each happened (a join also names the ally whose war it joined, a state change its state) and the tooltip adds the operation it ran inside, the hostility reason, the peace conference and the operands that change its meaning; rows of recordings without causes say so. A pair at war whose ending the native history recorded shows that exact row only: the derived "no longer at war" row, first seen at the read after the pair's last war read, drops each counterparty with a native end of the same pair dated at either of those two reads, and remains for every other pair (older recordings, refused native intervals). Unknown roles say **a country not identified**; All countries keeps them. After a world reset or load, coverage restarts at the next hourly boundary; actions after the last recorded hour of a session are not shown.
- Nuclear strikes name the bomb (from its native type, whether or not the stockpile was recorded), launcher and owner of the struck state by their exact identities at the impact hour. Their tooltip names the state at that hour and totals building levels, war support, units, manpower (with air crews) and equipment the impact applied before its `on_nuke_drop` effects; a part the recording did not fully capture says so instead of a total.
- Outside-branch Live/Pin is unavailable and suspends feed/label reads. Ranges use overlapping recorded hours. Missing-capture reasons are reported with the widget's diagnostics, not drawn.
- Load only needed windows. Page counts/slices belong to client/request/filter; source/branch/content changes reset log state, while filtering retains it. Labels request only visible rows at their own dates.

## Files

| File | Responsibility |
|---|---|
| [EventFeedWidget.tsx](EventFeedWidget.tsx), [useEventFeed.ts](useEventFeed.ts) | Virtual log, window demand and source ownership. |
| [eventFeedData.ts](eventFeedData.ts) | Validation, filtering, read cutoffs and row wording; adjacent tests. |
| [definition.ts](definition.ts), [index.ts](index.ts), [eventFeed.css](eventFeed.css) | Categories, shared options, registration and layout. |
