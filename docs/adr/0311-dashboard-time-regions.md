# 0311. Every widget region takes its time from one resolver and reads through one gate

A widget shows more than one time at once. Its main view reads the widget's own time, follow, campaign, a range or a pin. A drill can follow playback while the widget is pinned, or stay on an hour the user picked. A pop-out, a tooltip or a hover reads the hour under the pointer. Each of these is a **region**: a widget spec plus the user's action on it (`spec`, `follow`, or `at` a picked hour).

`dashboard/regionTime.ts` is the only place a region's hour is worked out. Axis hours (`AxisHour`, the playback clock's position) and track hours (`TrackHour`, an hour of one recorded branch) are distinct branded types at every public boundary, so one cannot be passed for the other. A track hour exists only when the branch recorded it: an hour outside the branch, or inside one of the branch's declared gaps (`BranchMetadata.gaps`, with the recording's own reason), is no hour. `coverageAt` says which, and the region's state carries the message ("outside the recording" or "not recorded") and the gap's reason. Nothing is substituted for a missing hour: no branch end, no cached hour, no parent's pin, no hour the selection was made at. A missing pin is no hour; the region says so and adds "Choose another time in widget settings." when its own pin or range put it there.

**The gate.** `dashboard/regionGate.ts` (`useRegionGate`, `useGate`) is the one production guard. Every read takes a gate, not a widget: `useHeadProduct(runtime, gate, …)` asks nothing unless the gate is open. The gate runs the stages in order, each from the recording:

1. **Coverage.** No track hour: outside, or not recorded with the gap's reason. Country groups are not even fetched.
2. **Country identity.** The selected key's holder at the region's hour. An automatic choice is dated and saved only at a recorded hour.
3. **Group membership.** Members at the region's hour. Membership changes and identity runs are boundaries of the region's hour, so a member joining at 1005 is present after a 1003 anchor.
4. **The read.** Only now is the region's data asked, at the region's hour.

The first stage that does not hold decides the region's state (`GateState` renders it): outside, not recorded, choose a country, choose a holder, absent at this date, no members, failed, loading. Displayed facts are dated by the reply's own hour; frame work uses the one page hour.

**Drills.** Every drill uses one reducer and action set (`charts/drill/drillState.ts`) through `useDrill`, and every `DrillHost` takes that control. A drill's region is its own: a saved breakdown reads its widget's time until the user picks or follows; Follow, Pause and pick act on the same state whether the breakdown is saved or opened. A drill opens following playback only under a following widget, when playing or opened near the playhead. The drill's hour, coverage, day and pin come from its own region, and `DrillHost` shows the drill's outside state itself, so a drill's data, date, state and mount never depend on its parent's hour, availability or loading. A parent leaving the track keeps the drill mounted.

Playback costs nothing per frame. Region hours re-render React only when the clock is anchored (seek, step, play, pause) or a region crosses a boundary: its track's ends, a declared gap, a country's identity runs, or a group's membership changes. The shared boundary watch checks every region once per clock hour.

[Resolver](../../ui/src/dashboard/regionTime.ts)
[Gate](../../ui/src/dashboard/regionGate.ts)
[Region scopes and hours](../../ui/src/dashboard/widgetTime.ts)
[Drill state](../../ui/src/charts/drill/drillState.ts)
[Drill control](../../ui/src/charts/drill/useDrill.ts)
