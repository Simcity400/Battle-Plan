# World tension & threat

**What it shows:** World tension with country contribution layers and dated threat-source events, plus a full contributor breakdown.

**Product(s) it reads:** [`world-tension`](../../../../crates/dashboard/src/products/world_tension.rs) supplies exact tension, uncapped contributions, source dates/text and identity spans. Native scales, cap and scan refusals are owned there.

**Settings:** All countries or manual/smart groups; top-five/eight + Other, share/recorded contributions, markers, values list, Show Breakdown and inline/pop-out drill; Campaign, Follow, Pin and Range plus playback/window/Show after the playhead controls.

**Notable behaviour:**

- Share mode scales contributions to independent world tension; recorded mode retains signed contributions beyond its cap. Negative parts stack below zero. The independent tension line remains exact when sums differ; missing reads break line/layers instead of carrying across gaps.
- Ranked layers use the shown moment. Hourly group members move contributions into/out of Other without losing world totals. Manual roots match native holders/aliases; separate slots sharing a tag remain separate contributors. Missing holder spans keep amounts with unknown looks.
- Events retain native dates and recorded-language reasons. Final amounts keep their supplying dates: they cannot describe current per-source decay or backfill earlier reads. Pre-recording events have no marker but retain contributions; the second-country operand is not interpreted.
- Full drill ignores overview country/top-layer filters, keeps every signed contributor and exact unidentified remainder, and uses the uncapped sum for signed shares. Zero/missing denominators give no share; virtual rows/sparse history preserve all contributors.
- Picking history holds; Follow uses the clock; Pin saves aggregate breakdown/day, not transient cause identity. Overlapping causes require choice; source revision clears it. Pending overview groups do not block the global drill. Looks stay dated and holder-bounded.

**Files:** [WorldTensionWidget.tsx](WorldTensionWidget.tsx) renders overview/drill; `tensionData.ts` validates/ranks/projects; `tensionDrill.ts` shapes exact parts/history/pins; `TensionDrillView.tsx` renders details. Adjacent tests cover these rules. `worldTension.css`, `definition.ts` and `index.ts` own styling, settings and registration.

Shared framework conventions: [widgets](../README.md).
