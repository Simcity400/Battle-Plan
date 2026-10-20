# Refits

**What it shows:** One country's refit work at the hour shown: each live refit line, named by its target design, with its progress meter and the ships on it in the game's order, under one totals line (ships refitting, refit lines, dockyards).

**Product(s) it reads:** [`naval-refits`](../../../../crates/dashboard-naval-yards/README.md), a keyed family prepared from the hourly refit progress and the independently dated daily line costs, targets and allocations. One request per country holds the branch; playback looks the hour up locally.

**Settings:** One country (the dashboard's by default); the shared table settings (density, When narrow). Follow (default), Pin or the campaign's end.

**Notable behaviour:**

- A line is live only where the hourly progress census says so; daily metadata never makes a line live. No queued ships and no completion dates: the recording has neither.
- Lines are grouped rows that open and close (the shared table's group rows); ships are shown by their recorded name where the recording holds one, else by their design's name, with the full reference on hover.
- Hull and dockyards are the low-priority columns: they hide first as the widget narrows.

**Files:** [RefitsWidget.tsx](RefitsWidget.tsx) draws the grouped table; [useYards.ts](useYards.ts) requests a country's history for both naval yard widgets and [shipNames.ts](shipNames.ts) names ships; lookups are in [navalYards.ts](../../dashboard/navalYards.ts).

Shared framework conventions: [widgets](../README.md).
