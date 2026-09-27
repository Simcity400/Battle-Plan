# Charts

The in-house chart kit every widget draws with. It has no chart dependency: charts draw on Canvas 2D so dozens of animated widgets stay cheap, and one shared scheduler drives them all. Everything here except the DOM readers is pure and tested with `node --test`.

| Module | What it does |
|---|---|
| [scheduler.ts](scheduler.ts) | One `requestAnimationFrame` loop. A task draws when woken and keeps frames coming only while it is still moving; hidden tasks never draw; the loop sleeps when nothing moves. |
| [surface.ts](surface.ts) | Backing-store size from the canvas's real on-screen size (which includes the widget's Size zoom and display scaling), and pointer positions in layout pixels. |
| [tokens.ts](tokens.ts) | Theme values a chart draws with, read from CSS variables through a probe element, and `APPEARANCE_EVENT` for re-reading them. |
| [palette.ts](palette.ts) | The categorical palette for categories (not countries), solved per theme for 3:1 contrast, and `nudgeApart` for the fixed-colour setting and faction colours. |
| [timeAxis.ts](timeAxis.ts) | Ticks from the recording's own hours: decade, year, quarter, month or day, with labels centred on each period's visible part (quarters on their first tick). |
| [valueAxis.ts](valueAxis.ts) | Nice linear and log ticks, and exact formatting of recorded fixed-point integers. |
| [view.ts](view.ts) | Zoom state and its reducer (wheel, box, pan, dragging either axis, resets), playback domains (draw-in, cursor, window), how far past the playhead a chart shows, and axis settings. |
| [series.ts](series.ts) | Runs of recorded values, gap-aware segments, sampling, a monotone curve and stacking. |
| [easing.ts](easing.ts) | Shape easing for the app's motion setting and playback speed. |
| [labels.ts](labels.ts) | Non-overlapping end labels and label columns measured over the whole campaign. |
| [tooltipPlacement.ts](tooltipPlacement.ts) | Tooltips beside the pointer, flipped and clamped so they are never cut off. |
| [frame.ts](frame.ts) | The plot frame (hour/value ↔ pixel, linear or log), hit zones for the two axes and the plot, and thin bar rectangles. |
| [draw.ts](draw.ts) | Canvas drawing shared by every chart: axes and gridlines, gap-broken lines (solid or dashed) with a ringed end dot, stacked areas, gap shading, event ticks, playhead, crosshair, box selection and end labels. |
| [timeline/](timeline/) | Lane timelines: one lane per slot with bars for who held it and when, a left column naming each holder at the playhead, and the same zoom, hover and Jump to as the time chart. |
| [badges.ts](badges.ts) | Event badges: small glyphs along the top edge with a guide line down the plot. Hovering one snaps to its hour and names the event; a click picks it; right-click jumps there. |
| [TimeChart.tsx](TimeChart.tsx) | The time chart component: layers, a value axis fitted from zero or, with `fitValues`, to both bounds of what is drawn, zoom that holds during playback (wheel, box, Shift/middle-drag pan, axis drags, double-click and Escape resets, a Reset zoom chip), live hover and right-click. |
| [tooltip.ts](tooltip.ts), [TooltipLayer.tsx](TooltipLayer.tsx) | One tooltip for the whole dashboard, fed structured rows (never HTML), rendered in the top layer so no widget clips it and the native map stays visible. |
| [Legend.tsx](Legend.tsx) | A legend for two or more series, with each series' current value. |
| [jump.ts](jump.ts) | The "Jump to <date>" menu item, which asks the clock to seek. |
| [drill/](drill/) | The drill-down: its state (follow playback or a picked day), settings fields, Pin as widget, the host (a slide inside the widget or a panel beside it) and the parts list. |

Widgets show loading, empty, error and "Not in this recording" with [WidgetState](../components/WidgetState.tsx). The Appearance settings' **Charts** section holds the app-wide chart motion (snappy, smooth, cinematic) and country colours (raw or adjusted); charts read both from `ChartTokens`.

## Rules

- **Shapes ease; numbers stay exact.** Labels and tooltips show the recorded value at the resolved hour; only positions glide. Nothing eases across a gap, and a gap is never drawn as zero.
- **No fixed dates.** Axes, ticks and ranges come from the recording's start and end; settings store offsets from the branch start.
- **No fixed colours or sizes.** Draw with `ChartTokens`. Countries use their recorded map colours; categories use the palette in its fixed order, and a ninth category folds into "Other".
- **Sharp at any Size.** Size the backing store with `backingSize` and draw in layout pixels; never stretch a canvas.
- **Zoom holds during playback.** A zoomed axis keeps its range until the user resets it.
