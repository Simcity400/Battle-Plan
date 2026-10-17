# Air superiority timeline

**What it shows:** One strategic region per row and calendar day per column, coloured by the leading side's margin or split into side shares.

**Product(s) it reads:** [`air-superiority`](../../../../crates/dashboard-air-superiority/src/products/air_superiority.rs) supplies selected-region daily timelines, the current-day moment and lightweight choices.

**Settings:** Per side by default or Exact viewpoint; side/theatre filters, chosen regions or most-active count (default ten), margin/split cells; Campaign, Follow, Pin and Range. Shared heat-chart zoom, pan and keyboard controls apply.

**Notable behaviour:**

- Comparison, co-belligerents, stable historical side keys, dated names and unknown owners follow [Air superiority by region](../air-superiority-region/README.md).
- Country colours use each cell's supplying hour; historical coalitions keep their native anchor's established colour read. A latest-hour tag lookup cannot colour earlier cells.
- Each cell describes the day's last reached recorded hour and dates it explicitly. Live ranks regions using completed days, so an unfinished day cannot reorder them. The server selects and ranks rows.
- Blank cells mean no activity, shading means not recorded and hatching means unknown. Gaps never supply scores; theatre membership has its own read date.
- Follow combines daily history with the current-day moment and missing-row snapshot defaults, preserving theatre exclusions/gaps. Held replies belong to client, recording, content and comparison; changes clear them. Campaign shows full extent with the clock cursor.
- The shared heat kit owns drawing, axes, scrolling, tooltip, navigation and Jump. Long row/legend lists stay reachable without enlarging the backing canvas by country count.
- The full choices product supplies campaign region labels before the first chart draw, keeping the gutter stable as ranked rows change during playback.

**Files:** [AirSuperiorityTimelineWidget.tsx](AirSuperiorityTimelineWidget.tsx) configures the [heat kit](../../charts/heat/README.md); `airSuperiorityTimeline.css` styles the wrapper. Shared [airOptions.ts](../../dashboard/airOptions.ts) and [airSuperiority.ts](../../dashboard/airSuperiority.ts) own comparison/projection. `definition.ts` and `index.ts` declare settings and registration.

Shared framework conventions: [widgets](../README.md).

The widget reads prepared daily values with a cached keyframe sample for each
later block. Approximate cells have the shared faded dashed edge and Approximate
key. Most-active regions and historical side identities are marked
approximate too. `sampleHours` dates the actual supplying frame in tooltips;
recorded daily columns keep their own bounds. Cells crossing the preparation
boundary are wholly approximate, while unread keyframes stay pending. The heat
kit shows the pending band and Still being prepared key. Pending columns use
that wording in tooltips and stay separate from the recording-gap shade. Exact prepared replies
replace these markers as coverage completes. The parity model is "Air superiority
daily values".
