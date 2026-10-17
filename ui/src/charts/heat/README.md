# Heat strips

[HeatChart.tsx](HeatChart.tsx) draws content-neutral categorical rows on the shared [TimeChart](../TimeChart.tsx) surface. [layout.ts](layout.ts) owns cells, clipping, run coalescing and read navigation. The caller owns comparison semantics, colours, labels and tooltips; follow the [chart contracts](../README.md).

## Data and appearance

Rows have opaque IDs, labels and one cell per ascending period. A period has inclusive first/last hours and the supplying `readHour`; null means a recording gap. Cells contain any number of `{id, label, color, share}` parts. Empty parts are blank inactivity, `unknown` is hatched, and `filtered` is outlined. Gaps use the separate recording shade, excluding any hours still being prepared. Future reads do not draw. `approximate` hours fade their cells and give them the dashed approximate edge (the hatch stays for unknown cells); `pending` hours draw a light band and no cells. The kit names both in one [PreparedKey](../PreparedKey.tsx) line under its legend.

Keep rows/parts immutable. Identical part arrays can coalesce adjacent drawing runs while tooltips retain each period's read. Drawing clips exact periods without rounding across gaps.

`look="margin"` fades the leader from theme grey by its margin over the runner-up: ties stay grey, a lone part has a zero runner-up, and the ramp is owned by `marginColor`. `look="split"` uses proportional horizontal stripes in stable part order.

## Interaction and scale

The shared surface owns label ellipsis, top time axis, zoom/pan, resets, playback cursor and tooltip ownership. `afterPlayhead` shows the whole campaign with the cursor. Left/right and Home/End step reads; up/down select rows. Pointer snapping and keys share `heatNavigation`. The tooltip receives row, period (null when absent) and hour; the context menu receives the dated target for [jumpItem](../jump.ts).

Rows scroll at readable heights. A logical spacer keeps the full inventory; the canvas, cell runs and labels cover only the viewport plus a small row margin. Dates stay at the top, and keyboard navigation scrolls every logical row into view. The optional HTML legend windows independently.

The gutter measures campaign labels in the drawing font and reserves plot space in narrow cards. Pass `labelCatalogue` when playback changes the row inventory; otherwise `rows` must include the full campaign label inventory. Font and card width changes invalidate measurement. Keep pure coverage tests beside layout and shared checks in [AGENTS.md](../../../../AGENTS.md).
