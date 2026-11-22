# 0311. Every widget region takes its time from one resolver and reads through one gate

A widget shows more than one time at once. Its main view reads the widget's own time, follow, campaign, a range or a pin. A drill can follow playback while the widget is pinned, or stay on an hour the user picked. A pop-out, a tooltip or a hover reads the hour under the pointer. Each of these is a **region**: a widget spec plus the user's action on it (`spec`, `follow`, or `at` a picked hour).

`dashboard/regionTime.ts` is the only place a region's hour is worked out. Axis hours (`AxisHour`, the playback clock's position) and track hours (`TrackHour`, an hour of one recorded branch) are distinct branded types at every public boundary, so one cannot be passed for the other. A track hour exists only when the branch recorded it: an hour outside the branch, or inside one of the branch's declared gaps (`BranchMetadata.gaps`, with the recording's own reason), is no hour. `coverageAt` says which, and the region's state carries the message ("outside the recording" or "not recorded") and the gap's reason. A pick keeps the hour picked, so a pick inside a gap says "not recorded" with the gap's reason. Nothing is substituted for a missing hour: no branch end, no cached hour, no parent's pin, no hour the selection was made at. A missing pin is no hour; the region says so and adds "Choose another time in widget settings." when its own pin or range put it there.

**The gate.** `dashboard/regionGate.ts` (`useRegionGate`, `useGate`) is the one production guard. It runs the stages in order, each from the recording:

1. **Coverage.** No track hour: outside, or not recorded with the gap's reason. Country groups are not even fetched.
2. **Country identity.** The selected key's holder at the region's hour, and a selection's aliases at that hour. A slot key and a tag name one country through the identity index, never by comparing strings. A failed identity read is a failed stage with a retry.
3. **Group membership**, only when the spec chooses a group or factions; Top N and every country rank instead. Membership changes and identity runs are boundaries of the region's hour, so a member joining at 1005 is present after a 1003 anchor.
4. **The read.**

The first stage that does not hold decides the region's state (`GateState` renders it): outside, not recorded, choose a country, choose a holder, absent at this date, no members, failed, loading. Displayed facts are dated by the reply's own hour; frame work uses the one page hour.

**Bypassing is a type error.** Every request's params are `GatedParams`: `hour`, every `…Hour` field, `from`, `to`, `reads[]` and `hours[]` accept only a `GatedHour`, which only the gate mints and only while its stages hold. An open gate gives `hour`, `window`, `hourAt(axis)` for a following read, `recorded(hour)` for a hover or a reply's own hour, `within(hour)` and `span(first, last)` for range bounds inside the branch. A follow subscription is a `FollowQuery`, minted by `gate.follow` while the region follows inside its track: a paused seek out of coverage closes it and coming back reopens it. Persisted times are `GatedTime` from `gate.time()` (the region's own hour, or follow), and `addWidgetBelow` accepts no other; a dated country choice is written only as `boundCountryToken`, minted by the gate's country stage or `gate.bind(tag)` (the Stability and Equipment losses drills still save their own holder record, offered only while the drill's gate is open); Pin to now uses `pinNow`, which pins only recorded hours. Two named exceptions carry hours they do not choose: the follow transport's clock and registration messages, and the label facts (country names, flags and styles) dated by rows already shown.

**Drills.** Every drill uses one reducer and action set (`charts/drill/drillState.ts`) through `useDrill`, which gives the drill its own gate; every `DrillHost` takes that control and shows the drill's gate state itself. The reducer is the only place Follow, Pin and pick become a drill's time: Follow always follows, whatever the parent's mode; a saved breakdown is an open drill reading its widget's time until the user picks or follows. A drill opens following playback only under a following widget, when playing or opened near the playhead. The drill's hour, reads, day and pin come from its own gate, so a drill's data, date, state and mount never depend on its parent's hour, availability or loading, and a parent leaving the track keeps the drill mounted.

Playback costs nothing per frame. Region hours re-render React only when the clock is anchored (seek, step, play, pause) or a region crosses a boundary: its track's ends, a declared gap, a country's identity runs, or a group's membership changes. The shared boundary watch checks every region once per clock hour.

[Resolver](../../ui/src/dashboard/regionTime.ts)
[Gate](../../ui/src/dashboard/regionGate.ts)
[Region scopes and hours](../../ui/src/dashboard/widgetTime.ts)
[Drill state](../../ui/src/charts/drill/drillState.ts)
[Drill control](../../ui/src/charts/drill/useDrill.ts)
