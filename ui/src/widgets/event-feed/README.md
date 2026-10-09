# Event feed

## What it shows

A world log of Wars & peace, Diplomacy, Politics and Milestones, newest first with dated country labels and optional game artwork.

## Products it reads

[`event-feed`](../../../../crates/dashboard/src/products/event_feed.rs) supplies window discovery and event pages composed from topic histories.

## Settings

All countries by default, or manual/groups; campaign/follow/pin/range, category toggles/chips, artwork and shared log options.

## Notable behaviour

- Rows become visible at their supplying read, with exact native dates kept separate from By bounds. Technology completions are excluded. Initial snapshots and lost territory do not invent events/causes.
- Country matches use each row’s identity/membership hour; departures use the last recorded member hour. Literal choices expand to native holders/aliases, never unrelated reuse.
- Native declarations, joins, civil-war transfers and annexations retain actor/target roles independently with exact event-hour ID/instance witnesses. Unknown roles say **a country not identified**; All countries keeps them. After a world reset or load, coverage restarts at the next hourly boundary; actions after the last recorded hour of a session are not shown.
- Outside-branch Live/Pin is unavailable and suspends feed/label reads. Ranges use overlapping recorded hours. Missing-capture reasons stay in Not recorded.
- Load only needed windows. Page counts/slices belong to client/request/filter; source/branch/content changes reset log state, while filtering retains it. Labels request only visible rows at their own dates.

## Files

| File | Responsibility |
|---|---|
| [EventFeedWidget.tsx](EventFeedWidget.tsx), [useEventFeed.ts](useEventFeed.ts) | Virtual log, window demand and source ownership. |
| [eventFeedData.ts](eventFeedData.ts) | Validation, filtering, read cutoffs and row wording; adjacent tests. |
| [definition.ts](definition.ts), [index.ts](index.ts), [eventFeed.css](eventFeed.css) | Categories, shared options, registration and layout. |
