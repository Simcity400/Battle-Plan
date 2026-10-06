# Shared tables

`Table.tsx` renders the style-5 Scoreboard as a selectable HTML table with a
sticky 26px header, themed scrolling and windowed rows. It supports the Military
Generals leaderboard, Aces and Supply hubs as well as Scoreboard, Agency
comparison, Operatives roster and Occupied states. Use one column model for all
of them; custom renderers can compose portraits, flags, subtitles, stars,
mini-bars and nonwrapping trait icons. See the synthetic, Node-tested
[Military story](military.fixture.ts).

## Final API

Import `Table`, `CountryCell`, `CellBar`, `TableColumn` and `TableProps` from `./Table`.
The column type also lives in [columns.ts](columns.ts), where missing-cell
handling can be tested without a browser. `ReactNode`, `MenuItem`, `WidgetProps`
and `TooltipContent` below are the existing shared types.

```ts
type SortValue = string | number | bigint | null | undefined;
type SortDirection = 'ascending' | 'descending';
interface Sort { column: string; direction: SortDirection }

interface TableColumn<T> {
  key: string;
  header: ReactNode;
  label: string;
  value: (row: T) => SortValue;
  render?: (row: T) => ReactNode;
  renderMissing?: boolean;
  align?: 'start' | 'end';
  numeric?: boolean;
  sortable?: boolean;
  bar?: 'visible' | number | bigint;
  width?: string;
}

interface TableProps<T> {
  label: string;
  rows: readonly T[];
  columns: readonly TableColumn<T>[];
  rowKey: (row: T) => string;
  sort: Sort;
  setOption?: WidgetProps['setOption'];
  cellBars?: boolean;
  rank?: boolean;
  rankMarkers?: boolean;
  density?: 'comfortable' | 'compact';
  rowHeight?: number;
  onRowClick?: (row: T) => void;
  muted?: (row: T) => boolean;
  menu?: (row: T) => MenuItem[];
  tooltip?: (row: T) => TooltipContent;
  onVisibleRows?: (rows: readonly T[]) => void;
  hoursPerSecond?: number;
}
```

`header` may be empty; `label` is always required and names the header for
assistive technology and the gear menu's Sort by choices. Sortable headers are
real buttons with `aria-sort` on the column header. `sortable` defaults to true;
false removes the button and the column's Sort by choices. Alignment defaults
to start, or end when `numeric` is true; an explicit `align` wins. `numeric`
supplies tabular figures, without restricting custom content or bar support.

`value` is the exact sort and bar value, independent of displayed content.
String values use a shared locale-aware numeric `Intl.Collator`: Ägypten sorts
before Zambia and State 2 before State 10. Row-key tie-breaks remain deterministic.
Null, undefined and nonfinite numbers display **—**, even with `render`, unless
`renderMissing: true`. That opt-in is useful for a portrait/name or an explicit
"Unknown" fallback. Missing values stay last in both directions. Equal values,
including missing ones, tie-break by `rowKey` using UTF-8/code-point order
(not locale or input order), ascending in both sort directions. Keys must be
unique and stable across updates. There is no human/AI visibility gate.

`rowHeight` is a positive comfortable-density height in layout pixels. Omit it
to measure the CSS default once: 34px comfortable, 25px compact. Explicit
heights scale by the same 25/34 ratio in compact density. CSS and windowing use
the same measured height; the header stays 26px. Custom-height cells center
their content without reserving the default vertical padding. The
`--table-row-height` variable is available to renderers that need to fit art
in both densities. Column `width` accepts a CSS grid track (e.g. `92px` or
`minmax(190px, 2fr)`), including the cell's 12px padding on each side.
Use `minmax(0, 1fr)` for names that should truncate and `max-content` for
numbers that keep their natural width. Content-sized columns measure the header
and mounted cells, share the widest measurement across rows, and grow as new
virtual rows appear. Density, font or column-label changes remeasure them. `CountryCell`
exposes the full country name through its title; custom name cells should do
the same. Column minimums, rather than full name text, determine overflow.

`onRowClick` runs on a row click, or Enter/Space on a focused row. Clicks do
not open the menu. `menu` builds items on right-click, the context-menu key or
Shift+F10. Include the existing helper from [jump.ts](../jump.ts):

```ts
jumpItem(
  runtime: Pick<WidgetRuntime, 'requestSeek' | 'tracks'>,
  trackId: string,
  rawHour: number,
): MenuItem
```

Return no Jump item when the row has no recorded hour. Add widget actions
beside it in the same array. `muted` adds grayscale and
`--table-muted-opacity` (default .55); it does not remove, disable or reorder
the row. Arrow keys and Home/End navigate rows and scroll only enough to show
the target below the sticky header. A focused row stays mounted across moves.

Tooltips use the shared layer. Their callback runs only when shown by pointer
or focus, and content is cached for that row until its data or builder changes.
The shown row's stable key survives playback reorders; its current content is
refreshed without hiding the tooltip, and removal clears it. Refresh never
reclaims the shared layer after another widget takes it. Focus builds a
single accessible description for the focused row; other rows
do not carry full hidden tooltips. Use `CountryCell` with `useCountryLabelsAt`
for flags and names at the shown hour. `onVisibleRows` lets a widget request
mounted countries' flags. `useTableHour` follows playback at most ten times
per second; seeks and pauses publish immediately.

## Minimal widget usage

```tsx
import Table from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { TABLE_OPTIONS, tableSortOf, tableSortOptions } from '../../charts/table/options';
import { jumpItem } from '../../charts/jump';
import { resolveOptions } from '../../dashboard/options';
import type { WidgetProps } from '../../dashboard/definitions';

type Row = { id: string; name: string; score: bigint | null; hour: number; ended: boolean };
const columns: TableColumn<Row>[] = [
  { key: 'name', header: 'Name', label: 'Name', value: row => row.name },
  { key: 'score', header: 'Score', label: 'Score', numeric: true,
    value: row => row.score, bar: 'visible' },
];
const defaultSort = { column: 'score', direction: 'descending' } as const;
export const OPTIONS = [...TABLE_OPTIONS, ...tableSortOptions(columns, defaultSort)];

function Roster({ rows, openCareer, widget, runtime, setOption }:
  WidgetProps & { rows: Row[]; openCareer: (row: Row) => void }) {
  const options = resolveOptions(OPTIONS, widget.options);
  return <Table label="Roster" rows={rows} columns={columns} rowKey={row => row.id}
    sort={tableSortOf(options.tableSort, defaultSort)} setOption={setOption}
    cellBars={options.cellBars === true} rank rankMarkers={options.rankMarkers === true}
    density={options.density === 'compact' ? 'compact' : 'comfortable'} rowHeight={48}
    muted={row => row.ended} onRowClick={openCareer}
    menu={row => [jumpItem(runtime, widget.trackIds[0], row.hour),
      { label: 'Open career', run: () => openCareer(row) }]} />;
}
```

Every table widget declares `TABLE_OPTIONS` and `tableSortOptions` in its gear
menu. Cell bars and rank markers default on; comfortable density is the
default. The single atomic `tableSort` option persists column and direction
through `setOption`. `rank` adds the optional # column. Bar scales use the
value's units; `'visible'` fits the filtered table's largest value, while a
fixed maximum can express 0–100% regardless of the rows shown.

## Motion and scale

Rank history resets whenever the sort column or direction changes; markers
describe value changes under the same ordering. The rank track is 44px and
country flags are 18×12px, as in style-5.

For a composed cell whose text sorts by a different value than its bar, use:

```ts
function CellBar(props: {
  value: number | bigint | null;
  maximum: number | bigint;
  children: ReactNode;
}): React.JSX.Element
```

Null hides the bar; zero renders an empty bar. Pass null when the widget's shared
Cell bars option is off. This also supports several mini-bars in a custom cell.
Use `usePlaybackSpeed(clock)` alongside `useTableHour` for motion that responds
to rate changes while paused.

`useRowMotion` owns slides, entry flashes and exit fades. Large tables mount
only a viewport window and retain previously visible rows through their slide
even if they move outside it. Reduced motion removes leaving rows immediately.
[logic.ts](logic.ts) keeps sort, bars, rank deltas and windowing pure.
`visibleRange(offsets, top, height, overscan)`,
`shownMotion(rows, start, end, animate, keep)` and
`scrollToShow(offsets, index, top, height, stickyTop)` use the same contracts as
the log kit on `origin/widgets/w3-construction-log`; neither imports the other
branch. Offsets include the sticky header; ranges have exclusive ends.

Node tests check that a 5,000-row redraw mounts only the rows in view and those
sliding through it. They do not time it; React reconciliation and browser
layout/paint require the lead's dev-browser check.
