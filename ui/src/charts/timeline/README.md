# Lane timelines

[Timeline.tsx](Timeline.tsx) draws dated holder lanes, production bands, career steps, event marks and expandable trees. Widgets supply facts and wording; the kit supplies zoom/pan, playback cutoff, tooltips, keyboard navigation and context menus. Follow the [chart contracts](../README.md).

## Files

| File | Responsibility |
|---|---|
| [layout.ts](layout.ts) | Lane/span types, holders, cached row/band geometry and pointer/keyboard targets. |
| [viewport.ts](viewport.ts) | Visible lanes, overscan, scrolling and Size-zoom coordinates. |
| [runState.ts](runState.ts) | Honest holding/ending descriptions, coverage counts and gap bridging. |
| [laneRequests.ts](laneRequests.ts) | All names needed for sorting, flags only for visible lanes and margin. |
| [extent.ts](extent.ts), [labelCatalogue.ts](labelCatalogue.ts), [marks.ts](marks.ts) | Growing extents, stable label widths, disclosure and event-mark actions. |

## Spans and gaps

Holder lanes show the current icon/name and cross-fade holder changes. `depth`/`heading` form trees; lane heights, subrows and `stackedSpans` share cached prefix geometry for drawing, scrolling and hits. Only visible/overscanned lanes draw, and pointer movement does not rebuild layout.

A span's `band` uses the shared `bandMax`. Pixel-column placement preserves peaks without rounding over gaps; hit testing and Jump describe the span/read actually drawn, including a displaced narrow band. `runState`/`describeRun` retain uncertainty such as Held before, Ended by and Last recorded. `bridgeGaps` joins only where no recorded evidence contradicts continuation and the caller agrees; it never proves activity during missing hours.

A bar's name leaves room for its end mark, such as a completion tick. Compact lanes show just the lane label. `stepMax` with observed `value` draws steps; `joinedStep` connects consecutive hours only. `barMax` scales period counts, with positive/negative outcomes supplied only when known. Missing counts draw nothing; exact zero stays zero.

Markers/icons keep their actual dated hit target. Marks outside the view or at/after an exclusive playhead have no carried keyboard stop; date-only marks never act as held spans. Positioned glyphs retain independent stops.

## Labels and interaction

Use `labelCatalogue` for stable widths and `labelHour` for the actual read when extent includes an exclusive final edge. Growing extents opt into `preserveZoomOnExtentChange`. Supply `onLanesInView` for artwork demand and explicit `countryFlag` on native-country spans/identities; missing flags retain grey placeholders and dated reasons. Generic portraits/blueprints use their artwork path. Blueprint tiles without artwork use the shared flask fallback, and missing event-mark artwork uses a sparkle; empty lanes show a muted label.

Pointer and keyboard use the same cached geometry. `onHover` reports the resolved hour or null outside the tooltip builder; menus use that dated target. Label-column menus require `labelMenu`. Escape dismisses keyboard tooltips; `dismissPointerTooltipOnEscape` also permits pointer dismissal. Full keyboard descriptions use raw metadata; only displayed tooltip rows decode flags.

Prepared lanes receive their eased viewport and visible lanes through
`onSummaryView`. `onNavigateHour` loads exact change stops without retaining hourly
histories; queued arrow presses keep their order, and departed lanes, views, blur
and Escape discard pending navigation. Tooltips and Jump use the returned hour.
