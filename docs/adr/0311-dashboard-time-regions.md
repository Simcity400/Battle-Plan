# 0311. Every widget region takes its time from one resolver

A widget shows more than one time at once. Its main view reads the widget's own time, follow, campaign, a range or a pin. A drill can follow playback while the widget is pinned, or stay on an hour the user picked. A pop-out, a tooltip or a hover reads the hour under the pointer. Each of these is a **region**: a widget spec plus the user's action on it (`spec`, `follow`, or `at` a picked hour).

`dashboard/regionTime.ts` is the only place a region's hour is worked out. `resolveRegion` takes the region, its track and the clock's axis hour and returns the region's track hour, its window, whether it follows playback, and the detail to show when it has no hour. Axis hours (`AxisHour`, the playback clock's position) and track hours (`TrackHour`, an hour of one recorded branch) are distinct branded types, so one cannot be passed for the other. A track hour exists only when the branch recorded it: `recordedHour` returns `null` for anything outside the branch, and an unresolved hour is `null` all the way down. Nothing is substituted for it: no branch end, no cached hour, no parent's pin, no hour the selection was made at.

Each region then runs the same stages, in this order:

1. **Coverage.** No track hour: the region says "This time is outside the recording." It adds "Choose another time in widget settings." only when the widget's own pin or range put it there.
2. **Country identity.** The selected country key's holder is resolved at the region's hour (`useReachedCountry`). A span of country facts is not evidence that the holder stayed the same. An automatic choice is dated at the first recorded hour the region resolves, and is never made or saved for an unresolved hour.
3. **Group membership.** Members are taken at the region's hour (`useCountriesSelectionAt`), never at the branch end or the hour data arrived for.
4. **The read.** Only now is the region's data asked, at the region's hour, and its loading state describes that read.

The first stage that does not hold decides the region's state: outside the recording, country absent at this date, no members at this hour, loading, or the data. Each stage's state comes from the recording, never from another stage's guess.

A region's state is its own. The widget host gives the main region a scope (`RegionContext`), and every `DrillHost` declares its drill's region, so a loading state inside a drill describes the drill's read, not the parent's. A parent leaving the track puts the outside state in the main face and keeps the drill mounted; a drill's state never replaces its parent.

Playback costs nothing per frame. Region hours re-render React only when the clock is anchored (seek, step, play, pause) or a region crosses a boundary: its track's ends, a country's identity runs, or a group's membership changes. The shared boundary watch checks every region once per clock hour.

[Resolver](../../ui/src/dashboard/regionTime.ts)
[Region scopes and hours](../../ui/src/dashboard/widgetTime.ts)
[Membership at a region's hour](../../ui/src/dashboard/useCountriesSelectionAt.ts)
[Country identity at a region's hour](../../ui/src/dashboard/reachedProduct.ts)
