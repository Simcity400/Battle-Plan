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
| Table | [table/](table/README.md): typed columns, exact persisted sorting, cell bars, ranks, nested group rows and windowing. |
| Share | [StackedShareChart](StackedShareChart.tsx)/[share.ts](share.ts): held proportions and percent axes; [sunburst/share.ts](sunburst/share.ts) adapts flat world shares. |
| Butterfly | [ButterflyChart](butterfly/ButterflyChart.tsx): paired bars either side of one axis on one scale, exact end values and ratios, drawn as the race draws its rows (name column, bar shape, fonts, gridlines, value axis and row sizing for the top N). |
| Meter | [ProgressMeter](meter/ProgressMeter.tsx): known/unknown progress, live/done/idle and opt-in sweep. |
| Stacked periods | [stackedBars.ts](stackedBars.ts): a time-chart layer of one bar per period (such as a calendar month) stacked by part, signed parts either side of zero, unknown periods hatched; period lookup and keyboard stops. |

## Shared machinery

| Modules | Responsibility |
|---|---|
| [scheduler.ts](scheduler.ts), [surface.ts](surface.ts) | Wake-driven frames, visibility, backing size and layout-pixel coordinates. |
| [tokens.ts](tokens.ts), [useChartTokens.ts](useChartTokens.ts), [charts.css](charts.css) | Theme/layout tokens and appearance changes. |
| [series.ts](series.ts), [intervalIndex.ts](intervalIndex.ts), [stack.ts](stack.ts) | Validated coverage, indexed runs/stretches and compact signed stacks. |
| [draw.ts](draw.ts), [strip.ts](strip.ts), [labels.ts](labels.ts) | Gap-aware drawing, extrema-preserving thinning and fitted labels. |
| [chartSummary.ts](chartSummary.ts), [markSummary.ts](markSummary.ts) | Prepared width-bounded line geometry and stable event clusters; exact reached hours and event pages stay separate. |
| [timeAxis.ts](timeAxis.ts), [valueAxis.ts](valueAxis.ts), [numberFormat.ts](numberFormat.ts), [view.ts](view.ts), [frame.ts](frame.ts) | Dates, exact formatting, shared number formats, axes, playback and zoom geometry. |
| [palette.ts](palette.ts), [factionColours.ts](factionColours.ts), [easing.ts](easing.ts) | Category/line looks, faction defaults and motion. |
| [rowMotion.ts](rowMotion.ts), [useRowMotion.ts](useRowMotion.ts), [WindowedRows](WindowedRows.tsx), [Legend](Legend.tsx) | Committed row motion, viewport rows/legends and keyboard access. |
| [hover.ts](hover.ts), [pick.ts](pick.ts), [linePick.ts](linePick.ts), [jump.ts](jump.ts) | Input ownership, drawn picks, line candidates and shared-clock seeks. |
| [tooltip.ts](tooltip.ts), [TooltipLayer](TooltipLayer.tsx), [TooltipRows](TooltipRows.tsx) | Structured owner-scoped tooltips, fitted/scrollable rows and full spoken text. |
| [images.ts](images.ts), [countryFlag.ts](countryFlag.ts), [hatch.ts](hatch.ts) | Shared image decoding, dated flag states and unknown-value hatching. |
| [prepared.ts](prepared.ts), [PreparedKey.tsx](PreparedKey.tsx) | Approximate and still-being-prepared marking: span clipping, the faded dashed edge, the pending band, its time-chart layer, the one key line and the prepared-range status wording. |

Hourly summaries carry original supplying runs, so drawing, extents and picks use
the same legacy curves or held steps, including gaps and fractional view edges.
Dense summaries draw only screen-sized extrema geometry. Country race lines can
carry these same summaries and delegate exact picks, hover and keyboard reads to
their prepared query owner.

## Rules

Prepared time charts request summaries at their actual layout width, eased view and playback cutoff. Held summaries retain chronological extrema and the trailing value's change hour. A column containing gaps or native breaks shows disconnected extrema dots; it never connects unknown spans. Crowded marks show count/date-range clusters with stable exact event ordinals. Hover and keyboard request exact hours or reached event pages; spoken chart text names the bounded clusters. Late summaries cannot paint another viewport or playback cutoff.

Race charts also accept a real slice of a global ranking through `remoteRows`: total count, loaded offset, raw global value extrema, numeric owner and a visible-range callback. The existing scroll surface and Home, End and arrow controls reach every global row; only loaded rows draw or supply tooltip values. Keyboard targets awaiting a page acquire focus only under the same numeric owner, and pointer input, manual scrolling, blur or Escape cancels that pending focus. Spoken ranks retain their global positions. Whole-array races keep the same local interface and motion.

- Use thin grids, readable labels and restrained fills. `ChartTokens` carries Appearance and reduced motion. Country/content palettes retain faction grey/hue, resolving clashes only by lightness; user overrides win. Extra lines use named dashes ([LineSwatch](LineSwatch.tsx)); no data is dropped for lack of colour.
- Shapes may ease; numbers stay exact. Format recorded integers/fractions with `valueAxis.ts`, retaining signed corrections and known zero. Locale formatting uses the shared formats in `numberFormat.ts`: `toLocaleString` given a locale or options builds a new formatter on every call. Missing is not zero. Never ease across gaps, native breaks or identity changes. Unknown attribution uses the hatch; recording gaps use the separate time shade.
- Country race name columns reserve their unavailable-name label alongside the names drawn, so cached and pending name reads use the same initial bar origin. Longer names still grow the column during playback.
- Dates/ranges come from recordings; settings store branch-relative offsets. Separate displayed/picked hours from supplying reads and Jump dates. Zoom holds during playback. End labels use each line's last drawn hour, even when the view ends in a gap.
- Use `backingSize` and layout pixels for sharp Size zoom/display scaling. Screen positions use live element rectangles for pointer coordinates and keyboard tooltips, so moving a widget keeps picks and tooltips aligned. Shared scheduler tasks wake on changes, honour `active` and release on teardown. Hidden tasks never draw; settled tasks sleep.
- Window rows/legends; `WindowedRows.onVisibleRows` reports row/consumer changes, including departing content/hours and retained focused rows, for artwork requests. Tab and Shift+Tab traverse the complete row order, retaining nested control order and native input keys. Binary range bounds exclude offscreen history from drawing and fitting; continuity starts with a binary supplying-run lookup and walks the intervening runs. Time charts draw plots and interactions on one canvas. Thinning preserves extremes/gaps. Race rows that do not fit at their least height scroll under a fixed value axis, drawing only those in view; curved lines sample a fixed power-of-two grid of hours, so a line growing during playback keeps its shape and only its ends move; flow callers sum overflow into a last node.
- Picks retain drawn geometry, actors and callback. `pickOwner` covers client/source/branch, request/view, selection/membership and numeric lifetime, excluding playback/labels/art/redraws. A new owner cancels picks. A click picks at once, never waiting out a double-click: where a chart picks, its plot ignores double-clicks and the axes or Reset zoom reset instead. `DrillHost` starts the slide on the click and mounts the breakdown a frame later. `onViewChange` fires only for a new range or handler. `linePick` excludes unknown/gapped/clipped/future paths and returns overlapping actors for a chooser. Keyboard picks obey the same rule.
- Pointer and keys share values, tooltips and menus. Supply read/change `keyHours`, including unknown dates, and shared hit geometry for displaced columns. Dated menus use `jumpItem`; refresh cannot steal tooltip ownership.
- Country flags use the recorded top-bar shield artwork in its `panel` form (edge shading evened out) and fit its content-derived proportions inside their slots. Native countries supply `countryFlag: { flag, flagPending, flagReason }` even without a URL. HTML/Canvas adapters keep a grey placeholder labelled "Flag loading" or "Flag unavailable" for loading, missing or decode-failed flags; the reason stays in the metadata and is never shown, read out or added to a tooltip. Generic icons/flagless aggregates omit it. Never request a fresher flag read.

## Breakdown and tooltip semantics

Sunburst items may supply disjoint aggregate contributor counts. These add to the
union of counted contributor IDs through grouping, hidden levels and Other folding.

Sunburst distinguishes unmeasured parts from unknown totals using `unknownValue`, `finalUnknown` and level-specific uncertainty; `authoritativeTotal` preserves an independently known whole. Unknown wholes cannot supply Fill denominators/percentages. Signed corrections keep exact rows, draw only positive area and use Share without percentages. `weight` controls order/colour/folding, never amounts. Preserve exact denominators and attribution notes; missing names leave values exact. Other retains its parts; flat shares can disable folding.

Tooltips retain dated metadata/full `spoken` text. Tooltips carry no read-cadence notes ("Read …; … is read once a day"); notes explain something specific. `useHoverLabels` shows each country's sampled list look (same stretch and identity segment) while the hovered hour's own read is pending. `countryTooltipContent` shares Chord/Flow/Timeline headers; dated metadata overrides node looks. `fitTooltip` retains totals and says "N more" for omitted rows; Canvas fits before decoding flags. Scrollable lists retain all rows with stable `scrollId`, decoding only the viewport. `scrollTooltip` handles wheel/Page keys and same-owner scroll; spoken text needs no hidden artwork.

Logs separate identity/supplying `revealedHour` from displayed dates. Measured cards use layout pixels and `anchorRows` for stable scrolling. A log's viewport never takes its size from what it draws: rows clip rather than scroll sideways, and card logs keep the scrollbar's gutter because card heights follow the width. Hold only wanted pages/keyboard destinations; backward seeks remove future cards immediately, and late pages cannot reclaim dismissed tooltips.

Parity builds expose complete ordered log and eager/windowed legend view models after normal visible
captures. Preparation retains real history windows/pages and certifies their exact
boundaries; it does not resolve labels or artwork for every offscreen row. Full
models preserve row identities and operands independently of deferred labels.
Preparation identity follows a parent empty state as its log/table mounts.
Ordinary builds expose no preparation or registry.

Log row windows use the scroller's outer layout height, keeping overscan membership stable when a row adds a horizontal scrollbar. Scroll bounds and keyboard movement use the usable client height; one retained viewport observer reports size changes without depending on the height it publishes.

## Adding or changing a chart

Country race lines accept prepared width summaries with separate exact pick reads.
Their optional `pending` spans use the shared light time band and the same key as
approximate geometry.
Stacked shares accept synchronized count summaries for dense geometry, retaining
ordinary drawing when exact runs fit the view and preserving breaks in coarse
buckets. Summary samples never supply exact hover counts.

1. Reuse a kit above. Keep validation, content meaning and wording in widgets; share content-neutral helpers here.
2. Keep layout/hits/navigation pure beside the component. Drawing, picks and keyboard stops share geometry, tokens, scheduler, tooltip ownership and gap-aware runs.
3. Check exact/missing values, gaps, seeks, ownership, keys and scale with synthetic inputs. Follow [AGENTS.md](../../../AGENTS.md) and [frontend checks](../../README.md#development); inspect themes, narrow/tall sizes, Size zoom and reduced motion.

Prepared line summaries carry approximate spans separately from their display
values. The shared renderer clips these spans and draws faded dashed strokes;
one Approximate key names them. Exact spans retain the ordinary line geometry.
Timeline bars and heat cells inside approximate spans take the same fade and
dashed edge ([prepared.ts](prepared.ts)). A reply answered from its gate's
prepared prefix names the requested hours after it `pending`: a time axis draws
them as a light band with no geometry (`pendingLayer`), and [PreparedKey](PreparedKey.tsx)
names it "Still being prepared" beside Approximate in the same key line. Logs and
lists show one prepared-range status above or beside their rows, including an
empty list, until coverage is complete: "Prepared A to B; the rest is still
being prepared.", led by "Newest days exact from C." when a proven newest window
answers exactly (`PreparedLogCoverage`'s `exact`).

Prepared log rows can independently name a proven newest interval with
`preparedExactFrom`, and provisional prefix rows with `preparedApproximate`.
Event badges whose inventory can still change use `approximate`: the badge fades,
its edge is dashed, and its tooltip says Approximate. These marks are independent
of whether a numeric operand was read exactly.
