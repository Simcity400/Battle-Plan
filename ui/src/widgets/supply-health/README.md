# Supply health

**What it shows:** Each country's share of divisions in supply, strained or in the native very-low supply state, with a complete classification drill.

**Product(s) it reads:** [`supply-health`](../../../../crates/dashboard/src/products/supply_health.rs) supplies exact count runs and native identity spans. Its module notes describe the division-icon classification and content thresholds.

**Settings:** One country by default or manual/group comparisons; Show Supply classification, inline/pop-out drill, playback/window/Show after the playhead; Campaign, Follow, Pin and Range.

**Notable behaviour:**

- Native icon frames: hidden is **In supply**, frames 1/2 **Strained**, frame 3 **No supply**. Grace can cover low received supply; frame 3 need not mean zero. Recorded content defines every threshold; province supply is no substitute.
- Every division counts for its owner, including expeditionary sender, matching Army size. Countries have separate denominators. Any missing owned input, incomplete roster/link or unresolved owner breaks the whole country's stack. Complete zero divisions means total zero and undefined shares.
- Exact counts round to shares once. Held areas never interpolate or join gaps/identity breaks; tiled rendering cannot leak future columns. Only viewport countries request looks. The byte budget applies after identity filtering; oversized replies refuse without truncation.
- Prepared clients retain one viewport and exact reached moments. Unprepared views draw approximate keyframe shares immediately; zooms touching at most thirteen unprepared blocks read exact frames. The native classifier resolves the recording's Defines once per query. Dense views use synchronized width summaries; exact short views keep the existing drawing path. Rough display counts stay separate from exact hover, drill and keyboard values.
- Native holders follow aliases; dated groups clip areas/values. Capitulation badges do not end stacks. Flags preserve dated pending/refusal reasons; an unknown holder supplies no substitute look.
- Drill keeps the holder's full history beyond groups. Three buckets exhaust the denominator, without invented residuals/causes. Picking holds; Follow uses the aligned clock; Pin saves native origin/branch-relative day. Headers use body dates or stay neutral. Pinned breakdowns omit the body country heading when the frame names the country.
- Invalid origins refuse the body without selecting a current actor. Parent states/actor choosers keep neutral dated headers.

**Files:** [SupplyHealthWidget.tsx](SupplyHealthWidget.tsx) renders windowed country charts; `SupplyBreakdown.tsx`/`SupplyCountryChip.tsx` own details/header; [supplyHealthData.ts](supplyHealthData.ts) validates/projects exact counts. Adjacent tests cover these rules. `supplyHealth.css`, `definition.ts` and `index.ts` own styling, settings and registration; [StackedShareChart](../../charts/StackedShareChart.tsx) owns drawing.

Shared framework conventions: [widgets](../README.md).
