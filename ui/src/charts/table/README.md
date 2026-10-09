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

Every table widget declares `TABLE_OPTIONS` and `tableSortOptions(columns, defaultSort)`. Read `tableSortOf` and pass `setOption`: column/direction save atomically. Content-dependent columns use `tableSortOptions(..., true)`, `tableSortChoices` through `useOptionChoices`, and `sortOptionValue={tableSortToken}`. Cell bars and rank markers default on; density defaults comfortable. `rank` adds the rank column. A bar maximum uses the operand's units; `visible` fits the filtered table. `CellBar` permits a custom bar operand; null hides it and zero is an empty bar.

## Rows and interaction

Heights and width measurements use layout pixels under Size zoom. `rowHeight` specifies comfortable density and scales for compact density; CSS and windowing share it. Use `minmax(0, 1fr)` for truncating names and `max-content` for natural numeric widths. Keep full names available in titles/tooltips. `muted` changes appearance without hiding, disabling or reordering rows.

`onRowClick` handles click/Enter/Space. `menu` handles right-click, the menu key or Shift+F10; add [jumpItem](../jump.ts) only when a recorded hour exists. Arrow keys and Home/End reveal rows below the sticky header, retaining focus through reorders.

Tooltip builders run lazily and refresh the current stable row without reclaiming another owner's tooltip. `CountryCell` takes the dated name, flag, `flagPending` and `flagReason`, retaining the grey missing/decode-failed placeholder. Use `onVisibleRows` to request mounted flags only, `useTableHour` for playback and `usePlaybackSpeed` for speed-sensitive motion. Row motion follows committed renders; large tables retain only viewport and moving rows. Reduced motion removes departed rows immediately.

Keep synthetic tests beside the pure helpers, including large inventories; shared development checks are in [AGENTS.md](../../../../AGENTS.md).
