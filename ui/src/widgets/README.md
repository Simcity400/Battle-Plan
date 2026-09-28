# Widgets

Each folder is one widget: everything it owns lives there, and nothing elsewhere switches on its kind.

```
widgets/<name>/
  definition.ts   pure metadata: kind, version, title, description, icon, time modes, history, sizes
  index.ts        the React renderer (and header controls if any)
  <Name>Widget.tsx
  <name>Data.ts   payload validation and local playback lookups, with tests
  <name>.css
  README.md       what it shows, which product it reads, what remains
```

`definitions.ts` (this folder) lists every widget's metadata and registers it, for node tests and the layout model. `index.ts` pairs each kind with its renderer and registers the full widgets; TypeScript rejects a kind missing from either list.

## Contract

- The dashboard gives a widget `WidgetProps`: its spec, the shared runtime (tracks, clock, client, maps), whether it is on screen, the map options, a header slot and a diagnostics callback. See `dashboard/definitions.ts`.
- Data comes from one product per request through `useProduct` in `dashboard/products.ts`. A product reply covers the whole branch, so following playback is a local lookup with no further requests.
- Playback: subscribe with `usePlaybackHour` only when the widget follows the clock, and resolve the hour with `resolveWindow`. Every widget has its settings behind the gear in its header's top-right corner. A widget with `history: true` also gets a Live switch beside it: off shows the whole campaign as it stood at the end, on follows the timeline. Default to `campaign`. A line chart with whole-campaign playback offers `AFTER_PLAYHEAD_OPTION` (`dashboard/options.ts`): off by default, so nothing after the playhead shows; on, pass `afterPlayhead` to `TimeChart` and draw to `shownUntil(...)`. A widget that only makes sense at the playback moment, such as the map, sets `history: false` and always follows playback.
- **Settings.** The gear opens the same panel for every widget, with sections in a fixed order: Campaign, Countries, Time, Size, Options, Axes, Interaction, Reset (`dashboard/settingsSections.ts`). A section appears only when it has something to set. A widget adds its own choices by declaring `options` in its definition: toggles, choices, numbers, content choices and content sets (`dashboard/options.ts`; a content choice lists things only the recording's content knows, such as its idea categories, through the renderer's `useOptionChoices` hook; a content set picks several of them as chips, up to its `max` and never more than 16, each token at most 160 characters; a chip that cannot be chosen says why), each placed in the time, options, axes or interaction section. A control inside the widget, such as chips above a chart, saves an option through the optional `setOption` prop exactly as the panel does. Read them with `resolveOptions(definition.options, widget.options)`; a missing or illegal stored value falls back to the field's default, so the widget never sees a bad value. `countries: { max, ranked?, aggregate? }` lets the widget show several countries (one by default). Such widgets also offer smart groups (`dashboard/groups.ts`, from the `country-groups` product): human players, any recorded faction and everyone at war with a country, with top N when the widget ranks countries. Each group counts members at each hour (marking joins and leaves), at the end, or anyone ever, and can be summed into one series when the widget allows it. Resolve a group with `groupRuns` and `groupMembers`; never assume which factions or sides exist. Never store a date: store an offset from the branch start.
- **Saving.** Every settings-panel change saves to the active layout at once. Adding, moving, resizing and removing widgets are edit-mode actions and wait for Save. Stored options stay with the layout even when the widget type is not installed. When a definition's `version` rises, give it a `migrate(from, widget)`; without one, older settings are kept and the widget reports them as unsupported.
- **Header.** The frame draws the icon, title, the campaign name when several are open, the country chip (flag and name, "+N" for more) for data widgets, the Live switch and the gear. A widget never draws its own header or panel.
- **Faction colours.** Colour factions with `useFactionColours` (campaign settings, keyed by the recorded campaign): the user's choice, else a default that is readable, never grey and apart from the others. Give faction legend items `onColor` so their swatch edits the campaign colour. Faction leaders are not recorded yet, so defaults come from the palette.
- **Jump to.** Any dated row or chart point offers a right-click menu (`useContextMenu`) with `jumpItem(runtime, trackId, hour)`. It calls `runtime.requestSeek`, which moves the shared clock only when the hour is inside that track's branch; the clock still owns time.
- **Drill-down.** A widget whose value has parts wraps its view in `DrillHost` and lists the parts with `DrillParts` (`charts/drill`). It declares `DRILL_OPTION` (in the widget or beside it) and `breakdownOption(...)` among its options. The breakdown follows playback until a day is picked (`reduceDrill`), and **Pin as widget** adds `pinnedBreakdown(...)` below the widget with `runtime.addWidgetBelow`, as a layout change. Part labels come only from the game or the recording; whatever the recording cannot attribute is its own row.
- A widget never opens files, starts a clock or reads another widget's state. Header controls, icon and country choice come from its definition.
- Validate the product payload shape in `<name>Data.ts`; the transport already checks source identity.
- Register the Rust product in `crates/dashboard/src/products/`; regenerate `generated/dashboard.ts` when its types change.

## Reuse first

Anything a second widget would need belongs in the shared kit, not in a widget folder: fix it once there and every widget, present and future, has the fix. A widget folder keeps only what is about its own content (what its values mean, its rows and wording). Before writing a helper, look in the kit; when a widget grows one that another could use, move it to the kit with its tests. The kit so far:

- **Time charts** ([charts/TimeChart.tsx](../charts/TimeChart.tsx)): layers, zoom, hover and right-click, a value axis sized to its widest label, and keyboard hover (the arrow keys step through `keyHours`, the tooltip is read out, and a key takes the hover from the pointer). Give a chart a `tooltip` and keyboard users get it too; pass every read as `keyHours`, and a range starting between reads still reaches the value carried into it.
- **Value labels and amounts** ([charts/valueAxis.ts](../charts/valueAxis.ts)): `formatAxisValue` (the chart's default: exact, with the precision its ticks need), `formatFixed`, `formatAmount`, `formatSigned`, `fixedNumber`, and `rescale` for amounts in a binary denominator (the engine's 32768ths).
- **Tooltips** ([charts/tooltip.ts](../charts/tooltip.ts)): `fitTooltip` (the fullest form that fits the window, else a compact one with its totals kept and the rest counted; the full content stays readable by screen readers through `spoken`), `tooltipText`, `showTooltip`/`hideTooltip`; `useViewportHeight` ([charts/useChartTokens.ts](../charts/useChartTokens.ts)).
- **Runs** ([charts/series.ts](../charts/series.ts)): `runAt`, `sameStretch` (glide only within an unbroken stretch), `clipToSpans` (a date range or a group membership), `extentOf`/`extentOfAll`, `lineEndHour` (end labels where a line is last drawn), segments and curves; `drawLine`, `drawGapFill`, `placeEndLabels` and the rest in [charts/draw.ts](../charts/draw.ts).
- **Overlapping spans** ([charts/intervalIndex.ts](../charts/intervalIndex.ts)): `IntervalIndex` and `intervalIndexOf`, for lookups by hour without a scan.
- **Line looks** ([charts/palette.ts](../charts/palette.ts)): `lineStyles`, `lineColours`, `lineDashes`, `SECONDARY_DASH` (a ceiling or other secondary line); the `chart-line-key` paragraph naming solid and dashed lines; `useChartTokens` for the theme.
- **Bar strips** ([charts/strip.ts](../charts/strip.ts)): up and down bars under a time chart (`TimeChart`'s `strip`), `barStrip` draws them and resolves a hover to the bar drawn under the pointer, so hovering a bar is about the read behind it while the chart's own hover stays on its lines.
- **Reads** ([dashboard/reads.ts](../dashboard/reads.ts)): `readAt` (the read behind a value) and `readNote` ("Read …; … are read once a day.").
- **Countries at an hour** ([dashboard/countryFlagsAt.ts](../dashboard/countryFlagsAt.ts), [dashboard/countryNamesAt.ts](../dashboard/countryNamesAt.ts)): `useCountryLabelsAt` (name and flag as recorded then), `useCountryNamesAt`, `useCountryFlagsAt`.
- **Choosing content** ([dashboard/options.ts](../dashboard/options.ts), [components/ChipSet.tsx](../components/ChipSet.tsx)): content-set options and chips that say why they cannot act.
- **Lane timelines** ([charts/timeline/](../charts/timeline/)): lanes of spans with keyboard stepping and Jump to; a span's `band` draws a thickness that follows a value (factories each day), and a `muted` lane fades.
- **Moving table rows** ([charts/useRowMotion.ts](../charts/useRowMotion.ts)): rows that slide to a new order, flash in and fade out.
- **Groups, drill-down, jump and states**: [dashboard/groups.ts](../dashboard/groups.ts), [charts/drill/](../charts/drill/), [charts/jump.ts](../charts/jump.ts), [WidgetState](../components/WidgetState.tsx).

## Adding a widget

1. Create the folder above and implement `Product` in `crates/dashboard/src/products/<name>.rs`.
2. Add its metadata to `widgets/definitions.ts` and its renderer to `widgets/index.ts`.
3. Run `npm run check:ui`, `npm run test:ui` and the checks in `AGENTS.md`.
