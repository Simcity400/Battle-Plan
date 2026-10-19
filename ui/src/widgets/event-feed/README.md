# Event feed

## What it shows

A world log of Wars & peace, Diplomacy, Politics and Milestones, newest first with dated country labels and optional game artwork. Kinds the game gives no picture (wars, peace, peace conferences, releases, state changes, factions, guarantees, ruling parties) show their own icon on a tile in the category's colour (`EVENT_ICONS` in [definition.ts](definition.ts)), as do pictured kinds whose artwork is missing.

## Products it reads

[`event-feed`](../../../../crates/dashboard/src/products/event_feed.rs) supplies backward cursor pages composed directly from prepared topic events. A test-only recording scan supplies the parity oracle. [`place-names`](../../../../crates/dashboard-base/src/products/place_names.rs) names the states of state changes and nuclear strikes at their event hour.
A Live bound inside accepted coverage reads that prefix. A Live bound past it
reads back from the prefix's last hour, under the shared "Prepared A to B; the
rest is still being prepared." status, and every continuation keeps that hour;
a grown prefix starts the pages again. A page of obsolete project changes can return no rows while
its cursor advances to older facts. Repeated continuations stop page discovery
and show the existing error/retry state.

## Settings

All countries by default, or manual/groups; campaign/follow/pin/range, category toggles/chips, artwork and shared log options.

## Notable behaviour

- Every recorded event is listed with its day: the game's own date, or the day of the read that first shows it when the reads either side are consecutive. A break in the recording, or the time before it starts, lists nothing; a focus completed inside a break is counted, not listed. Rows become visible at their supplying read. Technology completions are excluded. Initial snapshots and lost territory do not invent events/causes.
- Country matches use each row’s identity/membership hour; departures use the last recorded member hour. Literal choices expand to native holders/aliases, never unrelated reuse.
- Native declarations, joins, civil-war transfers, annexations, ended wars, releases, state owner changes and peace conference starts retain actor/target roles independently with exact event-hour ID/instance witnesses. A recorded cause says how each happened (a join also names the ally whose war it joined, a state change its state) and the tooltip adds the operation it ran inside, the hostility reason, the peace conference and the operands that change its meaning; rows of recordings without causes say so. A pair at war whose ending the native history recorded shows that exact row only: the derived "no longer at war" row, first seen at the read after the pair's last war read, drops each counterparty with a native end of the same pair dated at either of those two reads, and remains for every other pair (older recordings, refused native intervals). Unknown roles say **a country not identified**; All countries keeps them. After a world reset or load, coverage restarts at the next hourly boundary; actions after the last recorded hour of a session are not shown.
- Nuclear strikes name the bomb (from its native type, whether or not the stockpile was recorded), launcher and owner of the struck state by their exact identities at the impact hour. Their tooltip names the state at that hour and totals building levels, war support, units, manpower (with air crews) and equipment the impact applied before its `on_nuke_drop` effects; a part the recording did not fully capture says so instead of a total.
- Outside-branch Live/Pin is unavailable and suspends feed/label reads. Ranges use overlapping recorded hours. Missing-capture reasons are reported with the widget's diagnostics, not drawn.
- Load only demanded cursor pages. The cursor retains the Live read bound while native dates move backward; delayed facts remain reachable after their supplying read has been passed. Page payloads release outside demand, while cursor/count descriptors retain the discovered history. Source, branch, content, filter and Live-bound changes own fresh page demand. Labels request only visible rows at their own dates.
- Parity builds prepare complete ordered event operands after visible captures. The cursor reader follows every continuation with the same category, country, range and supplying-read cutoffs. Every selected page must finish. Preparation requests no offscreen names or artwork; visible text and screenshots cover mounted descriptions. Ordinary builds expose no preparation or extra page demand.
- Semantic row operands retain event content and order plus the item's token, name, description and idea effects. Internal row IDs and page-local item indexes are excluded. Whole page replies and decorative icon bytes are excluded from per-row serialization; descriptions still use the original full view model.

## Files

| File | Responsibility |
|---|---|
| [EventFeedWidget.tsx](EventFeedWidget.tsx), [useEventFeed.ts](useEventFeed.ts) | Virtual log, cursor demand and source ownership. |
| [eventFeedData.ts](eventFeedData.ts) | Validation, filtering, read cutoffs and row wording; adjacent tests. |
| [definition.ts](definition.ts), [index.ts](index.ts), [eventFeed.css](eventFeed.css) | Categories, shared options, registration and layout. |

Scripted event fires, news, selected options, decisions and missions have distinct Politics rows. Repeated native instances keep distinct occurrence IDs and exact hours. Tooltips retain chosen ordinals, captured bound scope operands and available lifecycle results. Static titles, bodies, pictures and authored option labels resolve through recorded content; unavailable dynamic inputs appear in diagnostics.

Effect execution, actual terminal transitions and duration changes have distinct feed kinds and wording. Authored effect descriptions and their named/countable missing inputs share the existing tooltip.

Retained script-state rows use read time and carry neither occurrence certainty nor actor/target roles. Their details show pending due hours, fired baseline membership and timer/scope state without inventing historical fires or choices.
