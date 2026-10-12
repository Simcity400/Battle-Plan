# Charts

Shared Canvas charts and windowed HTML logs/tables. Widgets supply exact values, dated labels and meaning; the kit owns drawing/interaction. See the [widget contract](../widgets/README.md).

## Chart kit map

| Kit | Entry point and purpose |
|---|---|
| Time | [TimeChart.tsx](TimeChart.tsx): layers, axes, zoom/pan, playback, hover, keyboard stops, badges and strips. |
| Race | [race/](race/): ranked bars, lines and mixes. [CountryRaceView](race/CountryRaceView.tsx), parts/faction views and drills share presentation; [options.ts](race/options.ts) owns settings. |
| Log | [LogList.tsx](log/LogList.tsx): dated rows or measured cards, month dividers, motion, keyboard access and demand-loaded pages; [logOptions.ts](log/logOptions.ts) owns settings. |
| Sunburst | [Sunburst.tsx](sunburst/Sunburst.tsx): grouped parts, folding, zoom and Share/Fill; [layout.ts](sunburst/layout.ts)/[sunburstOptions.ts](sunburst/sunburstOptions.ts) own types/settings. |
| Timeline | [timeline/](timeline/README.md): holder lanes, bands, career steps, marks and expandable trees. |
| Chord | [ChordChart.tsx](chord/ChordChart.tsx): directed ribbons/arcs; [frame.ts](chord/frame.ts) binds data, motion and picks; [fold.ts](chord/fold.ts) preserves remainders. |
| Flow | [flow/](flow/README.md): source/target ribbons, stable ordering and caller-summed remainders. |
| Heat | [heat/](heat/README.md): categorical rows, dated shares, inactivity, gaps and unknown cells. |
| Drill | [DrillHost](drill/DrillHost.tsx), [DrillParts](drill/DrillParts.tsx), [drillState.ts](drill/drillState.ts): inline/pop-out breakdowns, Follow, picked moments and Pin as widget. |
| Table | [table/](table/README.md): typed columns, exact persisted sorting, cell bars, ranks and windowing. |
| Share | [StackedShareChart](StackedShareChart.tsx)/[share.ts](share.ts): held proportions and percent axes; [sunburst/share.ts](sunburst/share.ts) adapts flat world shares. |
| Butterfly | [ButterflyChart](butterfly/ButterflyChart.tsx): paired bars on one scale, exact end values and ratios. |
| Meter | [ProgressMeter](meter/ProgressMeter.tsx): known/unknown progress, live/done/idle and opt-in sweep. |

## Shared machinery

| Modules | Responsibility |
|---|---|
| [scheduler.ts](scheduler.ts), [surface.ts](surface.ts) | Wake-driven frames, visibility, backing size and layout-pixel coordinates. |
| [tokens.ts](tokens.ts), [useChartTokens.ts](useChartTokens.ts), [charts.css](charts.css) | Theme/layout tokens and appearance changes. |
| [series.ts](series.ts), [intervalIndex.ts](intervalIndex.ts), [stack.ts](stack.ts) | Validated coverage, indexed runs/stretches and compact signed stacks. |
| [draw.ts](draw.ts), [strip.ts](strip.ts), [labels.ts](labels.ts) | Gap-aware drawing, extrema-preserving thinning and fitted labels. |
| [timeAxis.ts](timeAxis.ts), [valueAxis.ts](valueAxis.ts), [numberFormat.ts](numberFormat.ts), [view.ts](view.ts), [frame.ts](frame.ts) | Dates, exact formatting, shared number formats, axes, playback and zoom geometry. |
| [palette.ts](palette.ts), [factionColours.ts](factionColours.ts), [easing.ts](easing.ts) | Category/line looks, faction defaults and motion. |
| [rowMotion.ts](rowMotion.ts), [useRowMotion.ts](useRowMotion.ts), [WindowedRows](WindowedRows.tsx), [Legend](Legend.tsx) | Committed row motion, viewport rows/legends and keyboard access. |
| [hover.ts](hover.ts), [pick.ts](pick.ts), [linePick.ts](linePick.ts), [jump.ts](jump.ts) | Input ownership, drawn picks, line candidates and shared-clock seeks. |
| [tooltip.ts](tooltip.ts), [TooltipLayer](TooltipLayer.tsx), [TooltipRows](TooltipRows.tsx) | Structured owner-scoped tooltips, fitted/scrollable rows and full spoken text. |
| [images.ts](images.ts), [countryFlag.ts](countryFlag.ts), [hatch.ts](hatch.ts) | Shared image decoding, dated flag states and unknown-value hatching. |

## Rules

- Use thin grids, readable labels and restrained fills. `ChartTokens` carries Appearance and reduced motion. Country/content palettes retain faction grey/hue, resolving clashes only by lightness; user overrides win. Extra lines use named dashes ([LineSwatch](LineSwatch.tsx)); no data is dropped for lack of colour.
- Shapes may ease; numbers stay exact. Format recorded integers/fractions with `valueAxis.ts`, retaining signed corrections and known zero. Locale formatting uses the shared formats in `numberFormat.ts`: `toLocaleString` given a locale or options builds a new formatter on every call. Missing is not zero. Never ease across gaps, native breaks or identity changes. Unknown attribution uses the hatch; recording gaps use the separate time shade.
- Dates/ranges come from recordings; settings store branch-relative offsets. Separate displayed/picked hours from supplying reads and Jump dates. Zoom holds during playback. End labels use each line's last drawn hour, even when the view ends in a gap.
- Use `backingSize` and layout pixels for sharp Size zoom/display scaling. Time and race charts read canvas sizes through `watchCanvasSize`, only when the canvas or its parent resizes: read in every frame, a chart drawn after another has resized forces a layout. Shared scheduler tasks wake on changes, honour `active` and release on teardown. Hidden tasks never draw; settled tasks sleep.
- Window rows/legends; `WindowedRows.onVisibleRows` reports row/consumer changes, including departing content/hours, for artwork requests. Index compact replies once; thinning preserves extremes/gaps. Race rows that do not fit at their least height scroll under a fixed value axis, drawing only those in view; curved lines sample a fixed power-of-two grid of hours, so a line growing during playback keeps its shape and only its ends move; flow callers sum overflow into a last node.
- Picks retain drawn geometry, actors and callback. `pickOwner` covers client/source/branch, request/view, selection/membership and numeric lifetime, excluding playback/labels/art/redraws. A new owner cancels picks. A click picks at once, never waiting out a double-click: where a chart picks, its plot ignores double-clicks and the axes or Reset zoom reset instead. `DrillHost` starts the slide on the click and mounts the breakdown a frame later. `onViewChange` fires only for a new range or handler. `linePick` excludes unknown/gapped/clipped/future paths and returns overlapping actors for a chooser. Keyboard picks obey the same rule.
- Pointer and keys share values, tooltips and menus. Supply read/change `keyHours`, including unknown dates, and shared hit geometry for displaced columns. Dated menus use `jumpItem`; refresh cannot steal tooltip ownership.
- Country flags use the recorded top-bar shield artwork in its `panel` form (edge shading evened out) and fit its content-derived proportions inside their slots. Native countries supply `countryFlag: { flag, flagPending, flagReason }` even without a URL. HTML/Canvas adapters keep a grey placeholder labelled "Flag loading" or "Flag unavailable" for loading, missing or decode-failed flags; the reason stays in the metadata and is never shown, read out or added to a tooltip. Generic icons/flagless aggregates omit it. Never request a fresher flag read.

## Breakdown and tooltip semantics

Sunburst distinguishes unmeasured parts from unknown totals using `unknownValue`, `finalUnknown` and level-specific uncertainty; `authoritativeTotal` preserves an independently known whole. Unknown wholes cannot supply Fill denominators/percentages. Signed corrections keep exact rows, draw only positive area and use Share without percentages. `weight` controls order/colour/folding, never amounts. Preserve exact denominators and attribution notes; missing names leave values exact. Other retains its parts; flat shares can disable folding.

Tooltips retain dated metadata/full `spoken` text. Tooltips carry no read-cadence notes ("Read …; … is read once a day"); notes explain something specific. `useHoverLabels` shows each country's sampled list look (same stretch and identity segment) while the hovered hour's own read is pending. `countryTooltipContent` shares Chord/Flow/Timeline headers; dated metadata overrides node looks. `fitTooltip` retains totals and says "N more" for omitted rows; Canvas fits before decoding flags. Scrollable lists retain all rows with stable `scrollId`, decoding only the viewport. `scrollTooltip` handles wheel/Page keys and same-owner scroll; spoken text needs no hidden artwork.

Logs separate identity/supplying `revealedHour` from displayed dates. Measured cards use layout pixels and `anchorRows` for stable scrolling. Hold only wanted pages/keyboard destinations; backward seeks remove future cards immediately, and late pages cannot reclaim dismissed tooltips.

Log row windows use the scroller's outer layout height, keeping overscan membership stable when a row adds a horizontal scrollbar. Scroll bounds and keyboard movement use the usable client height; one retained viewport observer reports size changes without depending on the height it publishes.

## Adding or changing a chart

1. Reuse a kit above. Keep validation, content meaning and wording in widgets; share content-neutral helpers here.
2. Keep layout/hits/navigation pure beside the component. Drawing, picks and keyboard stops share geometry, tokens, scheduler, tooltip ownership and gap-aware runs.
3. Check exact/missing values, gaps, seeks, ownership, keys and scale with synthetic inputs. Follow [AGENTS.md](../../../AGENTS.md) and [frontend checks](../../README.md#development); inspect themes, narrow/tall sizes, Size zoom and reduced motion.
