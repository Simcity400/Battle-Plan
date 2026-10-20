# Shared tables

[Table.tsx](Table.tsx) renders selectable HTML tables with accessible sticky headers, exact sorting, cell bars, ranks, row motion and viewport windowing. Widgets own row meaning, labels and dated facts. Use the [chart contracts](../README.md) and the composed-cell example in [military.fixture.ts](military.fixture.ts).

## Files

| File | Responsibility |
|---|---|
| [Table.tsx](Table.tsx) | `TableProps`, rendering, focus, tooltips, menus, `CountryCell` and `CellBar`. |
| [columns.ts](columns.ts) | `TableColumn`, missing-cell rules, grid tracks and content-width measurement. |
| [logic.ts](logic.ts) | Exact sorting/grouping, bars, rank deltas and viewport calculations. |
| [options.ts](options.ts) | Shared gear options and persisted sort encoding. |
| [useTableHour.ts](useTableHour.ts) | Bounded playback refreshes; seeks and pauses publish immediately. |
| [table.css](table.css) | Shared table geometry and theme styles. |

## Columns and ordering

Supply typed `columns`, unique stable `rowKey`s and `sort`. A column's `value` is its exact sort/bar operand, independent of `render`. `label` is required for accessible headers and settings even when `header` is empty. `numeric` uses tabular figures/end alignment; explicit alignment wins. `sortable: false` removes sorting controls; `direction` can fix an approved direction.

Null, undefined and nonfinite values display an em dash and sort last in both directions. Custom missing content requires `renderMissing: true`. Strings use locale-aware numeric collation; equal operands tie deterministically by key in ascending code-point order regardless of direction. `tieKey` can use the current tag while `rowKey` preserves recorded country identity for motion. There is no human/AI visibility gate.

Optional `groupBy` keeps one windowed table. Supply consistent dated descriptors keyed by recorded identity; equal labels remain separate groups. Group order precedes the selected within-group sort without changing row keys or values.

Narrow tables keep their most important columns. Give every column that can go a `priority`: while the content is wider than the table, the column with the largest priority hides (the later one of equal priorities first), one per pass before paint, and comes back once the table regains the width it needed. Hidden values move into the row tooltip after the widget's own rows, as `label` and the cell's text (`text` when the cell is not plain text, else the rendered string or the formatted operand, a dash when missing); `inTooltip` marks a value the widget's tooltip already shows, and a table without a tooltip titles the row by its first shown column. The first column (the row's identity) and the column the table is sorted by never hide, and a sort change recomputes which may. Columns without a priority always show; when they alone are too wide, the table scrolls sideways. The widths each column needed hold for the same tracks and typography only: a column-width, density, font or wrapping change recomputes the fit. When the hidden values make a tooltip taller than the window, it takes the shared scrolling layer, and the table forwards the wheel and Page Up/Page Down to it as the chart kits do, so every row stays reachable. The fit also recomputes when the rank column is turned on or off, and the combined tooltip is rebuilt when the window height or the shown columns change. `narrow="scroll"` (the `narrowTable` option in `TABLE_OPTIONS`, read with `narrowTableOf`) keeps every column and scrolls sideways instead. Sorting, parity rows and natural widths still cover hidden columns.

Every table widget declares `TABLE_OPTIONS` and `tableSortOptions(columns, defaultSort)`. Read `tableSortOf` and pass `setOption`: column/direction save atomically. Content-dependent columns use `tableSortOptions(..., true)`, `tableSortChoices` through `useOptionChoices`, and `sortOptionValue={tableSortToken}`. Cell bars and rank markers default on; density defaults comfortable. `rank` adds the rank column. A bar maximum uses the operand's units; `visible` fits the filtered table. `CellBar` permits a custom bar operand; null hides it and zero is an empty bar.

## Rows and interaction

Heights and width measurements use layout pixels under Size zoom. `rowHeight` specifies comfortable density and scales for compact density; CSS and windowing share it. Text uses the Tables font (the interface font unless one is chosen) at the interface's small size. Cells use the bar charts' weights: 500 for names and text, 600 for numeric cells; ranks stay muted at 400. Word wrapping measures each mounted row without a line limit; prefix offsets keep windowing, motion and keyboard scrolling aligned. Unmounted rows use their last measured height or the density minimum, and resizing or font changes invalidate measurements. Turning wrapping off restores fixed rows and ellipsis. Use `minmax(var(--table-name-min), 1fr)` for names, so a narrow table scrolls before names collapse, and `max-content` for natural numeric widths. Keep full names available in titles/tooltips. `muted` changes appearance without hiding, disabling or reordering rows.

Resize observers collect natural column widths and wrapped row heights, then commit their measurements together in an animation frame before paint. Layout writes run outside observer delivery, so growing a numeric track or reaching new rows cannot feed back into that delivery cycle. Changing the measurement scope or unmounting cancels pending writes; disconnected rows supply no measurements.

`onRowClick` handles click/Enter/Space. `menu` handles right-click, the menu key or Shift+F10; add [jumpItem](../jump.ts) only when a recorded hour exists. Arrow keys and Home/End reveal rows below the sticky header, retaining focus through reorders.

Tooltip builders run lazily and refresh the current stable row without reclaiming another owner's tooltip. `CountryCell` takes the dated name, flag, `flagPending` and `flagReason`, retaining the grey missing/decode-failed placeholder. Use `onVisibleRows` to request mounted flags only; it reports only when the mounted rows change, so columns rebuilt from the reported rows settle. Use `useTableHour` for playback and `usePlaybackSpeed` for speed-sensitive motion. Row motion follows committed renders; large tables retain only viewport and moving rows. Reduced motion removes departed rows immediately.

`onVisibleRows` reports changes to the mounted row objects and their order, or a changed callback. Recreating a column accessor or the sorted array with the same row objects does not report the same rows again, so artwork state updates settle without causing a parent/table render cycle.

Parity builds expose every ordered row identity, exact column operand and sort.
An optional item projection retains referenced inputs without repeating whole
replies. History preparation uses the original recording reader and certifies
its exact boundary. Preparation identity survives an empty view gaining a table;
readiness waits without serializing the history. Visible screenshots cover
mounted cell text and artwork. Ordinary builds expose no row hook.

Keep synthetic tests beside the pure helpers, including large inventories; shared development checks are in [AGENTS.md](../../../../AGENTS.md).
