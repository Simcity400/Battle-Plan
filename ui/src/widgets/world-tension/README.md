# World tension & threat

**What it shows:** World tension with country contribution layers and dated threat-source events, plus a full contributor breakdown.

**Prepared data it reads:** Generic series queries for [`world-tension`](../../../../crates/dashboard/src/products/world_tension.rs) supply exact reached reads, uncapped contributions, source dates/text, prepared identity spans and width summaries. Native scales, cap and scan refusals are owned there. Overview and breakdown wait for accepted coverage. Remembered country names and links, historical contributor zeroes and unnamed attribution require preceding frames, so an unprepared tail stays pending.

**Settings:** All countries or manual/smart groups; top-five/eight + Other, share/recorded contributions, markers, values list, Show Breakdown and inline/pop-out drill; Campaign, Follow, Pin and Range plus playback/window/Show after the playhead controls.

**Notable behaviour:**

- Share mode scales contributions to independent world tension; recorded mode retains signed contributions beyond its cap. Negative parts stack below zero. The independent tension line remains exact when sums differ; missing reads break line/layers instead of carrying across gaps.
- Ranked layers use the shown moment. Hourly group members move contributions into/out of Other without losing world totals. Manual roots match native holders/aliases; separate slots sharing a tag remain separate contributors. Missing holder spans keep amounts with unknown looks.
- Events retain native dates and recorded-language reasons. Final amounts keep their supplying dates: they cannot describe current per-source decay or backfill earlier reads. Pre-recording events have no marker but retain contributions; the second-country operand is not interpreted.
- Full drill ignores overview country/top-layer filters, keeps every signed contributor and exact unidentified remainder, and uses the uncapped sum for signed shares. Zero/missing denominators give no share; virtual rows/sparse history preserve all contributors.
- Picking history holds; Follow uses the clock; Pin saves aggregate breakdown/day, not transient cause identity. Overlapping causes require choice; source revision clears it. Pending overview groups do not block the global drill. Looks stay dated and holder-bounded.
- Approved D-12 behaviour: dense signed stacks place each series' chronological extrema on a common bounded grid and use the shared signed-stack renderer. Baselines, selected-country sums and Other inside a dense bucket are summary geometry; they do not promise the extrema of every possible aggregate. Each series retains its own extrema and unknown spans remain disconnected. Views with at most one hour per bucket use the original drawing paths; hover, keyboard, contributor rows and navigation use exact recorded values.

**Files:** [WorldTensionWidget.tsx](WorldTensionWidget.tsx) renders overview/drill; `tensionData.ts` validates/ranks/projects; `tensionDrill.ts` shapes exact parts/history/pins; `TensionDrillView.tsx` renders details. Adjacent tests cover these rules. `worldTension.css`, `definition.ts` and `index.ts` own styling, settings and registration.

Shared framework conventions: [widgets](../README.md).
