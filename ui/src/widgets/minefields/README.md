# Minefields

**What it shows:** Who has mined which seas at the hour shown: per sea region, each country's current mines as segments on one shared scale, with the region's total at the right; a region opens its mines by owner over time and its standing reports.

**Product(s) it reads:** [`minefields`](../../../../crates/dashboard-naval-regions/README.md), a keyed family prepared from the hourly regional mine vectors. One request holds every owner's amounts and witnessed extents over the branch (or the chosen countries'); playback looks the hour up locally. A region's reports are a demand-loaded request at the drill's hour.

**Settings:** Countries (every owner by default), Regions, Sort regions by (Most mines or Name), where the breakdown opens.

**Notable behaviour:**

- Amounts are mines currently in the region, not mines laid or swept. An owner's omitted region below its witnessed extent is a proven zero; beyond it, or without a witness, the owner's amount is unknown and the row shows a hatched part.
- Bars share one scale of mine counts across rows, so bigger fields have longer bars; segments use the owners' country colours.
- Reports are the region's standing reports, with no strike dates: the recording keeps when a report expires, not when it struck. "Seen from" appears only where the first observation is proven.
- Below 420 px the total moves into the tooltip and the legend folds under the rows.

**Files:** [MinefieldsWidget.tsx](MinefieldsWidget.tsx) draws rows and the drill; [minefieldsData.ts](minefieldsData.ts) validates replies and proves amounts; [useMinefields.ts](useMinefields.ts) requests the history.

Shared framework conventions: [widgets](../README.md).
