# Air superiority by region

**What it shows:** Strategic regions at a playback moment, with exact comparative air-score shares and assigned-wing counts.

**Product(s) it reads:** [`air-superiority`](../../../../crates/dashboard/src/products/air_superiority.rs) supplies moment windows and lightweight choices. Native scoring and side assignment live in that product.

**Settings:** Follow or Pin; Per side (default) or Exact viewpoint; country/side viewpoint, side and recorded-theatre filters, content-region selection, and contested/activity/name ordering. Most contested is the default.

**Notable behaviour:**

- Coalition anchors use native slot/ID history and dated tags, excluding reuse. Faction colours need a strict majority of wartime participants; peaceful score-roster members do not vote. Otherwise use the established country colour; missing anchors stay unknown.
- Per-side comparison merges allied factions and assigns co-belligerents once, using one leader's independent friendly score rather than a world sum. Historical keys stabilise colours through joins/splits; labels name present members.
- Automatic exact viewpoint uses the dashboard country, falling back to the player. The tooltip names the supplying viewpoint and strict native thresholds.
- No scores means **No air activity**; filtered inactivity does not claim other sides are inactive. Unknown scores are hatched; absent strategic-air owners are excluded with counted explanations. Gaps say **Not recorded**.
- Missing region rows retain native snapshot defaults without filling gaps. Theatre exclusions still apply to regional orders.
- Wing counts include inactive mission-region assignments, not sorties. Names, theatre text and dated country looks follow recorded content and language/identity boundaries. Zero-score bars cannot reconstruct the game's neutral map colour.
- Windows project locally and prefetch the next block. Keyboard and Jump reach every region; readable row and legend windowing must preserve access to all entries.

**Files:** [AirSuperiorityRegionWidget.tsx](AirSuperiorityRegionWidget.tsx) renders rows; `airSuperiorityRegion.css` styles them. [Shared air settings](../../dashboard/airOptions.ts) and [projection](../../dashboard/airSuperiority.ts) serve both air-superiority widgets. `definition.ts` and `index.ts` declare settings and registration.

Shared framework conventions: [widgets](../README.md).
