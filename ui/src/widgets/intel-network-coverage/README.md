# Intel network coverage

**What it shows:** Who has eyes inside whom at the hour shown: network owners as rows, targets as columns, each cell the chosen recorded measure of the owner's intel network in the target.

**Product(s) it reads:** [`intel-network-coverage`](../../../../crates/dashboard-intelligence/README.md), a keyed family prepared from the daily network summaries. The matrix request ranks the countries shown and holds their pairs over the branch; playback looks the hour up locally. A pair's history is read when its drill opens.

**Settings:** Show (Matrix or Pair history, also a picker card); Measure (also the switch above the matrix): States covered (default), Strength or Strongest state; Basis (Controlled or Core states); Countries shown (the 12 most active by default, or the countries chosen in the header); where the breakdown opens.

**Notable behaviour:**

- Cells are the game's own retained summaries, never rebuilt from state rows. A network with a recorded summary of zeros is an empty network; a pair without a summary at a complete read has no network (blank); hours not recorded say so. No coverage percentage: the recording keeps covered counts, not a whole-network share.
- Colours run from theme grey to the accent over the shown cells' own range; exact values are in the tooltip, which names the direction.
- Countries shown rank by networks owned plus targeted; flags load only for the rows and columns drawn.
- Clicking a cell opens that pair's measure over time, following playback or pinned; Pin as widget keeps both countries.

**Files:** [IntelCoverageWidget.tsx](IntelCoverageWidget.tsx) draws the matrix and drill; [PairHistory.tsx](PairHistory.tsx) one pair over time; [intelCoverageData.ts](intelCoverageData.ts) validates the reply and looks pairs up; [useIntelCoverage.ts](useIntelCoverage.ts) requests it. The matrix is the heat kit's [HeatMatrix](../../charts/heat/HeatMatrix.tsx).

Shared framework conventions: [widgets](../README.md).
