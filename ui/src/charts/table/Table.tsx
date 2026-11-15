import { useCallback, useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { CSSProperties, ReactNode } from 'react';
import { inFrame } from '../../dashboard/frameUpdates.ts';
import { useContextMenu } from '../../components/ContextMenu';
import { CountryFlagUrl } from '../../components/CountryFlag';
import type { MenuItem } from '../../components/ContextMenu';
import type { WidgetProps } from '../../dashboard/definitions';
import { hideTooltip, refreshTooltip, scrollTooltip, showTooltip, tooltipText } from '../tooltip';
import { liveKey } from '../live';
import { atNextSlot, structureOpen } from '../../dashboard/structurePace';
import { useWhyCommit } from '../../dashboard/widgetPerf.ts';
import type { Live, LiveValues } from '../live';
import { scheduler } from '../scheduler';
import type { FrameTask } from '../scheduler';
import { BarCell, barScale, textCell } from '../../dashboard/follow/painters.ts';
import type { TextCell } from '../../dashboard/follow/painters';
import type { TooltipContent } from '../tooltip';
import { movingRowClass, useRowMotion } from '../useRowMotion';
import { useChartTokens, useViewportHeight } from '../useChartTokens';
import { easeTau } from '../easing';
import { useMotionNow } from '../liveMotion.ts';
import { sameRevision, useRevision, useMemoRows } from '../memoRows.tsx';
import type { RowEvents, RowRevision } from '../memoRows.tsx';
import { useTableAppearance } from './useTableAppearance.ts';
import { FULL_FIT, barFraction, barMaximum, densityRowHeight, fitColumns, groupedRows, isMissing, withRowHeights, nextSort, rankChanges, measuredRowOffsets, rowOffsets, scrollToShow, shownMotion, sortRows, treeRows, visibleRange } from './logic.ts';
import type { ColumnFit, RankChange, RowMeasurements, Sort, SortValue, TableGroup, TableTree, TreeRow } from './logic';
import { cellContent, columnAlign, growColumnWidths, hideOrder, tableGrid, withHidden } from './columns.ts';
import type { TableColumn } from './columns';
import { sortKey } from './options.ts';
import type { NarrowTable } from './options';

import { shownSlot } from '../rowNavigation.ts';
import type { RevealClock } from '../reveal';
import '../charts.css';
import './table.css';
import { engine } from '../../engine/frame.ts';

export type { TableColumn } from './columns';
export type { TableTree } from './logic';
const WINDOWING_THRESHOLD = 120;
const NO_KEYS: ReadonlySet<string> = new Set();
const NO_RANKS: ReadonlyMap<string, RankChange> = new Map();
/** How long a rank marker shows once its row moves. */
const RANK_HOLD_MS = 2500;
interface NaturalCell { key: string; row: string | undefined; revision: RowRevision; padding?: number }
interface NaturalMeasurement { revision: RowRevision; width: number; padding: number }
const NO_COLUMNS: readonly TableColumn<never>[] = [];
/** Unchanged markers keep their state, so new row objects in the same order do not re-render the table. */
const sameRanks = (before: ReadonlyMap<string, RankChange>, next: ReadonlyMap<string, RankChange>) => before.size === next.size
  && [...next].every(([key, change]) => { const held = before.get(key); return held?.rank === change.rank && held.delta === change.delta && held.until === change.until; });
export interface TableProps<T> {
  label: string; rows: readonly T[]; columns: readonly TableColumn<T>[]; rowKey: (row: T) => string;
  /** Data operands that determine every displayed cell, including failed reads and content. */
  rowRevision?: (row: T) => RowRevision;
  /** Current recorded tag order may differ from a country's stable motion key. */
  tieKey?: (row: T) => string;
  /** Optional grouping keeps the same row identities, geometry and exact values. */
  groupBy?: (row: T) => TableGroup;
  /** Group header rows with a disclosure and their rows nested under them, windowed together. */
  tree?: TableTree<T>;
  /** Rows keep the order they are given, such as the game's own order; headers offer no sorting. */
  unsorted?: boolean;
  sort: Sort; setOption?: WidgetProps['setOption'];
  /** Content-dependent column choices encode their persisted sort token. */
  sortOptionValue?: (sort: Sort) => string;
  cellBars?: boolean; rank?: boolean; rankMarkers?: boolean; density?: 'comfortable' | 'compact';
  /** Comfortable-density height in layout pixels; compact scales by the CSS ratio. */
  rowHeight?: number;
  onRowClick?: (row: T) => void;
  muted?: (row: T) => boolean;
  menu?: (row: T) => MenuItem[];
  tooltip?: (row: T) => TooltipContent;
  /** The rows actually mounted; a widget asks for flags only for these countries. */
  onVisibleRows?: (rows: readonly T[]) => void;
  hoursPerSecond?: number;
  /** Narrow tables hide prioritised columns into the row tooltip, or scroll sideways. */
  narrow?: NarrowTable;
  /** The playhead's values of the columns that `paint`, keyed `liveKey(rowKey(row), column.key)`: read as each frame
   * paints, so they move with no render. Their texts, bars and a sort by them come from here, not from `rows`. */
  live?: Live<LiveValues<SortValue>>;
  /** Following, the clock rows are revealed by and each row's reveal hour: rows past the clock's hour are drawn ahead,
   * hidden and taking no height, and each frame reveals those it reaches with the entry motion while the rows after them
   * glide down, so rows arriving cost no render. Not with `rank`, whose numbers count every row given. */
  reveal?: { clock: RevealClock; at: (row: T) => number };
}

/** Each slot's reveal hour, the same hours ascending, and the slot of each key: what a frame reads to reveal rows. */
interface Held { reveal: Float64Array; sorted: Float64Array; slots: ReadonlyMap<string, number> }

/** How many of `sorted` are at or before `hour`. */
function revealedThrough(sorted: Float64Array, hour: number): number {
  let low = 0, high = sorted.length;
  while (low < high) { const middle = (low + high) >> 1; if (sorted[middle] <= hour) low = middle + 1; else high = middle; }
  return low;
}

/** `[start, end)` widened over the rows next to it held ahead of `hour`: they take no height where the row they will
 * push down starts, and are drawn with it so revealing them needs no render. Rows merely unmeasured never widen it. */
function widenOverAhead(held: Held | null, hour: number | null, start: number, end: number, count: number): [number, number] {
  if (!held || hour === null) return [start, end];
  while (start > 0 && held.reveal[start - 1] > hour) start--;
  while (end < count && held.reveal[end] > hour) end++;
  return [start, end];
}
/** `offsets` with the slots not revealed at `hour` taking no height; `offsets` itself when every slot is revealed. */
function revealedOffsets(offsets: readonly number[], held: Held, hour: number): readonly number[] {
  if (revealedThrough(held.sorted, hour) === held.sorted.length) return offsets;
  const shown = [offsets[0]];
  for (let slot = 0; slot < offsets.length - 1; slot++) shown.push(shown[slot] + (held.reveal[slot] > hour ? 0 : offsets[slot + 1] - offsets[slot]));
  return shown;
}

/** A painted cell: its text, its bar if it draws one, and the key and column it shows. */
/** A painted bar and tone from another live cell of the row (`barCell`), and the tone last set. */
interface FromCell { key: string; bar: BarCell | null; element: HTMLElement; tone: string | undefined }
interface LiveCell<T> { key: string; column: TableColumn<T>; text: TextCell; bar: BarCell | null; from: FromCell | null }

/** style-5 Scoreboard: real table semantics, only the viewport's rows mounted. */
export default function Table<T>({ label, rows, columns, rowKey, rowRevision, tieKey = rowKey, groupBy, tree, unsorted = false, sort: storedSort, setOption, sortOptionValue = sortKey, cellBars = true,
  rank = false, rankMarkers = true, density = 'comfortable', rowHeight, onRowClick, muted, menu: rowMenu,
  tooltip, onVisibleRows, hoursPerSecond, narrow = 'hide', live, reveal: givenReveal }: TableProps<T>) {
  const revealBy = rank ? undefined : givenReveal;
  const owner = useId();
  const tokens = useChartTokens();
  const viewportHeight = useViewportHeight();
  const keyboardMenu = useRef(false);
  const contextMenu = useContextMenu(`${label} actions`);
  const scroll = useRef<HTMLDivElement>(null);
  const tableElement = useRef<HTMLTableElement>(null);
  const [view, setView] = useState({ top: 0, height: 0, width: 0, rowHeight: 0, headerHeight: 0 });
  const currentView = useRef(view);
  useLayoutEffect(() => { currentView.current = view; }, [view]);
  const [focusedKey, setFocusedKey] = useState<string | null>(null);
  const [description, setDescription] = useState<{ key: string; text: string } | null>(null);
  const tipCache = useRef<{ row: T; build: TableProps<T>['tooltip']; hidden: readonly TableColumn<T>[]; shown: readonly TableColumn<T>[];
    key: string; viewport: number; values: LiveValues<SortValue> | null; content: TooltipContent | null } | null>(null);
  const activeTip = useRef<{ key: string; origin: 'pointer' | 'keyboard'; x: number; y: number } | null>(null);
  const [localSort, setLocalSort] = useState<Sort | null>(null);
  const { tableSize, headerSize, tableFont, wrapping, comfortable, compact, headerMinimum, fonts } = useTableAppearance(scroll);
  const columnScope = JSON.stringify(columns.map(column => [column.key, column.label, column.width]));
  const widthScope = JSON.stringify([density, tableFont, tableSize, headerSize, wrapping, comfortable, compact, headerMinimum, columnScope]);
  const [natural, setNatural] = useState<{ scope: string; widths: ReadonlyMap<string, number> }>({ scope: widthScope, widths: new Map() });
  const currentNatural = useRef(natural);
  useLayoutEffect(() => { currentNatural.current = natural; }, [natural]);
  const heightScope = `${widthScope}/${view.width}/${rowHeight ?? ''}`;
  const bottomAnchor = useRef(false);
  const focusAnchor = useRef(false);
  const [rowMeasurements, setRowMeasurements] = useState<RowMeasurements>({ scope: heightScope, heights: new Map(), floor: 0 });
  const requestedSort = setOption ? storedSort : localSort ?? storedSort;
  const sortedColumn = columns.find(column => column.key === requestedSort.column && column.sortable !== false)
    ?? columns.find(column => column.sortable !== false);
  const sort = { column: sortedColumn?.key ?? '', direction: sortedColumn?.direction ?? requestedSort.direction };
  // The row's identity (the first column) and the column it is sorted by never hide;
  // rows kept in their given order have no sort column.
  const order = useRevision(narrow === 'hide' ? hideOrder(columns).filter(key => (unsorted || key !== sort.column) && key !== columns[0]?.key) : []);
  const [fit, setFit] = useState<{ scope: string; fit: ColumnFit }>({ scope: '', fit: FULL_FIT });
  // Widths each column needed hold only for the same tracks and typography.
  // The rank track takes width from the other columns too.
  const fitScope = JSON.stringify([order, widthScope, rank]);
  const columnFit = fit.scope === fitScope ? fit.fit : FULL_FIT;
  const shownColumns = useMemo(() => {
    if (!columnFit.hidden.length) return columns;
    const hidden = new Set(columnFit.hidden);
    return columns.filter(column => !hidden.has(column.key));
  }, [columns, columnFit]);
  const hiddenColumns = useMemo(() => columnFit.hidden.length
    ? columns.filter(column => columnFit.hidden.includes(column.key) && !column.inTooltip) : NO_COLUMNS as readonly TableColumn<T>[], [columns, columnFit]);
  // A sort by a painted column orders by the live values last committed: read again at most four times a second.
  const liveSort = live && !unsorted && sortedColumn && (sortedColumn.paint || sortedColumn.sortsLive) ? sortedColumn.key : null;
  const [sortedFrom, setSortedFrom] = useState<{ live: typeof live; values: LiveValues<SortValue> } | null>(null);
  const sortValues = liveSort === null ? null : sortedFrom && sortedFrom.live === live ? sortedFrom.values : live!.read();
  // Rebuilt column objects with the same accessor (such as new labels for another column) keep the order.
  const columnValue = sortedColumn?.value;
  const sortValue = useMemo(() => sortValues && liveSort !== null ? (row: T) => sortValues.get(liveKey(rowKey(row), liveSort)) : columnValue,
    [sortValues, liveSort, rowKey, columnValue]);
  // Groups the table holds itself: each toggled row's key and whether it is now open.
  const [toggled, setToggled] = useState<ReadonlyMap<string, boolean>>(new Map());
  const treeChildren = tree?.children, treeOpen = tree?.open, treeInitial = tree?.initiallyOpen, onToggle = tree?.onToggle;
  const isOpen = useCallback((row: T) => treeOpen ? treeOpen(row) : toggled.get(rowKey(row)) ?? treeInitial?.(row) ?? true,
    [treeOpen, treeInitial, toggled, rowKey]);
  const arrangeBy = useCallback((by: ((row: T) => SortValue) | undefined) => (list: readonly T[], level: number) => {
    if (unsorted) return list.slice();
    const value = by ?? (() => null);
    return groupBy && level === 1 ? groupedRows(list, groupBy, value, sort.direction, rowKey, tieKey)
      : sortRows(list, value, sort.direction, rowKey, tieKey);
  }, [unsorted, sort.direction, rowKey, tieKey, groupBy]);
  const arrange = useMemo(() => arrangeBy(sortValue), [arrangeBy, sortValue]);
  const { ordered, nesting } = useMemo((): { ordered: T[]; nesting: ReadonlyMap<string, TreeRow> | null } => {
    if (!treeChildren) return { ordered: arrange(rows, 1), nesting: null };
    const { rows: flat, nesting } = treeRows(rows, treeChildren, isOpen, arrange, rowKey);
    return { ordered: flat, nesting };
  }, [rows, arrange, treeChildren, isOpen, rowKey]);
  const toggle = (row: T, open: boolean) => {
    if (!treeOpen) setToggled(before => new Map(before).set(rowKey(row), open));
    onToggle?.(row, open);
  };
  const items = useMemo(() => ordered.map((item, slot) => ({ key: rowKey(item),
    item: { value: item, rank: nesting?.get(rowKey(item))?.position ?? slot + 1 } })), [ordered, rowKey, nesting]);
  const moving = useRowMotion(items, true);
  // The rows in view at the last commit, and those in view before its change, kept mounted while
  // they glide out until the next change. Read in render and written after a commit, so recording
  // them renders nothing again.
  const committedWindow = useRef<{ items: typeof items; keys: ReadonlySet<string>; departing: ReadonlySet<string> } | null>(null);
  const motionNow = useMotionNow();
  const glideMs = easeTau(tokens.motion, motionNow(hoursPerSecond), tokens.reducedMotion) * 2.5;
  const ordering = `${groupBy ? 'grouped/' : ''}${tree ? 'tree/' : ''}${unsorted ? 'given' : sortKey(sort)}`;
  // Rank markers against the ranks last committed, so a new order shows them in the frame it is drawn.
  const committedRanks = useRef<{ ordering: string; ranks: ReadonlyMap<string, RankChange> }>({ ordering: '', ranks: NO_RANKS });
  const drawnRanks = useMemo(() => {
    const at = performance.now();
    if (!rank || !rankMarkers) return { ranks: NO_RANKS, at };
    const held = committedRanks.current;
    const next = rankChanges(held.ranks, items.map(item => item.key), at, RANK_HOLD_MS, held.ordering !== ordering);
    return { ranks: sameRanks(held.ranks, next) ? held.ranks : next, at };
  }, [items, rank, rankMarkers, ordering]); // eslint-disable-line react-hooks/exhaustive-deps
  const ranks = drawnRanks.ranks;
  useLayoutEffect(() => {
    // A marker drawn here holds from when it is shown, however late its render commits.
    const shift = performance.now() - drawnRanks.at;
    let shown: Map<string, RankChange> | null = null;
    for (const [key, item] of ranks) {
      if (!item.delta || item.until !== drawnRanks.at + RANK_HOLD_MS) continue;
      shown ??= new Map(ranks);
      shown.set(key, { ...item, until: item.until + shift });
    }
    committedRanks.current = { ordering, ranks: shown ?? ranks };
    // A marker whose hold ends is hidden where it is drawn, never by a commit (run 13: a commit a marker while playing);
    // the next render leaves it out.
    let timer = 0;
    const expire = () => {
      const now = performance.now();
      let due = Infinity;
      for (const [key, item] of committedRanks.current.ranks) {
        if (!item.delta) continue;
        if (item.until > now) { due = Math.min(due, item.until); continue; }
        const mark = scroll.current?.querySelector<HTMLElement>(`tbody tr[data-table-row="${CSS.escape(key)}"] .table-delta`);
        if (mark) mark.hidden = true;
      }
      if (Number.isFinite(due)) timer = window.setTimeout(expire, Math.ceil(due - now) + 16);
    };
    expire();
    return () => window.clearTimeout(timer);
  }, [drawnRanks, ordering]); // eslint-disable-line react-hooks/exhaustive-deps
  useLayoutEffect(() => {
    const element = scroll.current;
    if (!element) return;
    if (!comfortable) return;
    const base = rowHeight != null && Number.isFinite(rowHeight) && rowHeight > 0 ? rowHeight : comfortable;
    // The stylesheet's row heights are for 12px text, the default interface's --type-sm.
    const measuredHeight = densityRowHeight(base, density, compact / comfortable) * tableSize / 12;
    const headerHeight = Math.max(headerMinimum, tableSize * 1.5 + 6);
    const measure = () => {
      const top = element.scrollTop, height = element.clientHeight, width = element.clientWidth;
      const next = { top, height, width, rowHeight: measuredHeight, headerHeight };
      if (!sameRevision(Object.values(currentView.current), Object.values(next))) { currentView.current = next; setView(next); }
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, [density, rowHeight, tableSize, comfortable, compact, headerMinimum]);
  const count = moving.reduce((count, row) => tokens.reducedMotion && row.phase === 'leaving' ? count : Math.max(count, row.slot + 1), ordered.length);
  const leavingSlots = JSON.stringify(moving.filter(row => row.phase === 'leaving' && row.slot >= items.length).map(row => [row.slot, row.key]));
  const slotKeys = useRevision(useMemo(() => {
    const keys = Array.from({ length: count }, (_, slot) => items[slot]?.key ?? '');
    for (const [slot, key] of JSON.parse(leavingSlots) as [number, string][]) keys[slot] = key;
    return keys;
  }, [count, items, leavingSlots]));
  const fullOffsets = useMemo(() => wrapping
    // Heights measured at another width stand until their rows are measured again: rows never collapse onto each other meanwhile.
    ? measuredRowOffsets(slotKeys, rowMeasurements.heights, Math.max(view.rowHeight, rowMeasurements.floor), view.headerHeight)
    : rowOffsets(count, view.rowHeight, view.headerHeight), [wrapping, slotKeys, rowMeasurements, count, view.rowHeight, view.headerHeight]);
  const revealAt = revealBy?.at;
  const held = useMemo<Held | null>(() => {
    if (!revealAt) return null;
    const hours = new Map(items.map(item => [item.key, revealAt(item.item.value)]));
    const reveal = Float64Array.from(slotKeys, key => hours.get(key) ?? -Infinity);
    return { reveal, sorted: Float64Array.from(reveal).sort(), slots: new Map(slotKeys.map((key, slot) => [key, slot])) };
  }, [revealAt, items, slotKeys]);
  const revealHour = held && revealBy ? revealBy.clock.hour() : null;
  const revealCut = held && revealHour !== null ? revealedThrough(held.sorted, revealHour) : null;
  // The hour matters only through the rows it reveals (`revealCut`): a render inside the same cut keeps the layout.
  const hourHeld = useRef(revealHour);
  hourHeld.current = revealHour;
  const offsets = useMemo(() => held && revealCut !== null && hourHeld.current !== null ? revealedOffsets(fullOffsets, held, hourHeld.current) : fullOffsets,
    [fullOffsets, held, revealCut]);
  const committedGeometry = useRef({ heightScope, rowMeasurements });
  const measuring = committedGeometry.current.heightScope !== heightScope || committedGeometry.current.rowMeasurements !== rowMeasurements;
  useLayoutEffect(() => {
    committedGeometry.current = { heightScope, rowMeasurements };
    const element = scroll.current;
    if (element && bottomAnchor.current) element.scrollTop = element.scrollHeight;
    if (element && focusedKey !== null && focusAnchor.current) {
      const index = slotKeys.indexOf(focusedKey);
      if (index >= 0) element.scrollTop = scrollToShow(offsets, index, element.scrollTop, view.height, view.headerHeight);
    }
  }, [heightScope, rowMeasurements, offsets, focusedKey, slotKeys, view.height, view.headerHeight]);
  const [start, end] = count > WINDOWING_THRESHOLD ? widenOverAhead(held, hourHeld.current, ...visibleRange(offsets, view.top, view.height), count) : [0, count];
  // Held ahead: after each commit and each frame, each mounted row's place and reveal state (`data-reveal`, never
  // rendered, so a render keeps a running entry) and the body's height are painted at the clock's hour.
  const body = useRef<HTMLTableSectionElement>(null);
  const revealed = useRef<{ full: readonly number[]; held: Held; header: number; cut: number; hour: number } | null>(null);
  const enterRows = useRef(!tokens.reducedMotion);
  enterRows.current = !tokens.reducedMotion;
  const paintRevealed = useCallback((enter: boolean) => {
    const now = revealed.current, tbody = body.current;
    if (!now || !tbody) return;
    const shown = revealedOffsets(now.full, now.held, now.hour);
    tbody.style.height = `${shown[shown.length - 1] - now.header}px`;
    for (const row of tbody.children as HTMLCollectionOf<HTMLElement>) {
      const slot = now.held.slots.get(row.dataset.tableRow ?? '');
      if (slot == null) continue;
      row.style.transform = `translateY(${shown[slot] - now.header}px)`;
      const ahead = now.held.reveal[slot] > now.hour, was = row.dataset.reveal;
      if (ahead) { if (was !== 'ahead') { row.dataset.reveal = 'ahead'; row.inert = true; } }
      else if (was === 'ahead') { if (enter) row.dataset.reveal = 'in'; else delete row.dataset.reveal; row.inert = false; }
    }
  }, []);
  useLayoutEffect(() => {
    revealed.current = held && revealCut !== null && hourHeld.current !== null
      ? { full: fullOffsets, held, header: view.headerHeight, cut: revealCut, hour: hourHeld.current } : null;
    if (revealed.current) paintRevealed(false);
  });
  useLayoutEffect(() => {
    if (!revealBy) return;
    return revealBy.clock.subscribe(() => {
      const now = revealed.current;
      if (!now) return;
      const hour = revealBy.clock.hour(), cut = revealedThrough(now.held.sorted, hour);
      if (cut === now.cut) return;
      const forward = hour > now.hour && revealBy.clock.playing();
      revealed.current = { ...now, cut, hour };
      paintRevealed(forward && enterRows.current);
    });
  }, [revealBy, paintRevealed]);
  // Keep previously visible DOM rows through the glide, even when their new
  // rank is outside the viewport. The existing motion hook owns their slots.
  const previousWindow = committedWindow.current;
  const crossing = previousWindow?.items !== items ? previousWindow?.keys : undefined;
  const departing = previousWindow?.items === items ? previousWindow.departing : NO_KEYS;
  const mounted = shownMotion(moving, start, end, !tokens.reducedMotion,
    row => row.key === focusedKey || (!tokens.reducedMotion && (departing.has(row.key) || crossing?.has(row.key) === true)));
  const memoRow = useMemoRows();
  const liveColumns = live ? shownColumns.filter(column => column.paint).map(column => column.key).join('\u0000') : null;
  const maxima = useMemo(() => new Map(columns.filter(column => column.bar != null).map(column => [column.key,
    column.bar === 'visible' ? barMaximum(ordered, column.value) : column.bar as number | bigint])), [columns, ordered]);
  const barRevision = useRevision(shownColumns.filter(column => column.bar != null).flatMap(column => [column.key, maxima.get(column.key)]));
  const update = {};
  const revisions = new Map(mounted.map(row => [row.key, rowRevision?.(row.item.value) ?? [update]]));
  const headers = shownColumns.map(column => column.sortable === false || unsorted ? column.header
    : <>{column.header}{(sort.column === column.key || column.width === 'max-content') && <span className="table-sort" aria-hidden="true"
      style={{ visibility: sort.column === column.key ? undefined : 'hidden' }}> {sort.direction === 'descending' ? '\u25be' : '\u25b4'}</span>}</>);
  const layoutKeys = useRevision(shownColumns.flatMap(column => [column.key, column.width, column.numeric, column.align, column.label, column.headerRevision, column.renderMissing,
    column.sortable === false || unsorted]));
  const headerKeys = useRevision([layoutKeys, sort.column, sort.direction]);
  const widthColumns = shownColumns.filter(column => column.width === 'max-content');
  const widthKeys = new Map<string, ReadonlyMap<string, RowRevision>>(widthColumns.length ? mounted.map(row => [row.key, new Map<string, RowRevision>(widthColumns.map(column => {
    const key = column.widthKey?.(row.item.value) ?? revisions.get(row.key)!;
    const node = column === shownColumns[0] ? nesting?.get(row.key) : undefined;
    return [column.key, [column.key, column.numeric, column.align, ...(typeof key === 'string' ? [key] : key), node?.level, node?.group]];
  }))]) : []);
  const naturalRevision = useRevision([headerKeys, ...[...widthKeys].flatMap(([key, columns]) =>
    [key, columns.size, ...[...columns.values()].flatMap(revision => [revision.length, ...revision])])]);
  const mountedKeys = useRevision(mounted.map(row => row.key));
  // A rank marker whose hold has ended is not drawn again (its row mounting, or rendering for another reason).
  const renderedAt = performance.now();
  useWhyCommit('Table', { rows, columns, live, sortedFrom, rowMeasurements, natural, fit, view, focusedKey, description, toggled, localSort, mounted: mountedKeys });
  useLayoutEffect(() => {
    const before = committedWindow.current;
    committedWindow.current = { items, keys: new Set(items.slice(start, end).map(row => row.key)),
      departing: before?.items === items ? before.departing : tokens.reducedMotion ? NO_KEYS : before?.keys ?? NO_KEYS };
  }, [items, start, end, tokens.reducedMotion]);
  const visible = useMemo(() => ordered.filter((row, slot) => (slot >= start && slot < end) || rowKey(row) === focusedKey
    || (!tokens.reducedMotion && (departing.has(rowKey(row)) || crossing?.has(rowKey(row)) === true))),
    [ordered, start, end, departing, crossing, focusedKey, rowKey, tokens.reducedMotion]);
  // Report only a changed set of mounted rows. Rebuilt columns re-sort into a new
  // array, and a widget whose columns follow the reported rows would otherwise loop.
  const reported = useRef<{ rows: readonly T[]; callback: typeof onVisibleRows } | null>(null);
  useEffect(() => {
    const held = reported.current;
    const before = held && held.callback === onVisibleRows ? held.rows : null;
    if (before && before.length === visible.length && before.every((row, index) => row === visible[index])) return;
    reported.current = { rows: visible, callback: onVisibleRows };
    onVisibleRows?.(visible);
  }, [visible, onVisibleRows]);
  const naturalCells = useRef<{ observer: ResizeObserver; observed: Map<Element, NaturalCell>;
    measurements: Map<string | undefined, Map<string, NaturalMeasurement>> } | null>(null);
  useLayoutEffect(() => {
    const pending = new Map<Element, { key: string; width: number }>();
    const observed = new Map<Element, NaturalCell>();
    const measurements = new Map<string | undefined, Map<string, NaturalMeasurement>>();
    let frame: (() => void) | null = null;
    const observer = new ResizeObserver(entries => {
      for (const { target, contentRect } of entries) {
        if (!target.isConnected) continue;
        const element = target as HTMLElement;
        const cell = observed.get(target);
        if (!cell) continue;
        if (cell.padding === undefined) {
          const css = getComputedStyle(element.closest('th, td')!);
          cell.padding = parseFloat(css.paddingLeft) + parseFloat(css.paddingRight);
        }
        const width = contentRect.width + cell.padding;
        let rowMeasurements = measurements.get(cell.row);
        if (!rowMeasurements) measurements.set(cell.row, rowMeasurements = new Map());
        const cached = rowMeasurements.get(cell.key);
        const applied = currentNatural.current;
        if (cached && sameRevision(cached.revision, cell.revision) && cached.width === width && applied.scope === widthScope
          && (applied.widths.get(cell.key) ?? 0) >= Math.ceil(width)) continue;
        rowMeasurements.set(cell.key, { revision: cell.revision, width, padding: cell.padding });
        pending.set(target, { key: cell.key, width: Math.max(pending.get(target)?.width ?? 0, width) });
      }
      if (frame) return;
      // Readings land with the frame's data, so what they commit is taken in the same frame.
      frame = engine.next('data', () => {
        frame = null;
        const readings = [...pending].filter(([element]) => element.isConnected).map(([, reading]) => reading);
        pending.clear();
        if (!readings.length) return;
        const before = currentNatural.current;
        const previous = before.scope === widthScope ? before.widths : new Map<string, number>();
        const widths = growColumnWidths(previous, readings);
        if (before.scope === widthScope && widths === previous) return;
        const next = { scope: widthScope, widths };
        currentNatural.current = next;
        // Measurements render with the next frame, a tick while playing: never a commit of their own then.
        inFrame(() => setNatural(next));
      });
    });
    naturalCells.current = { observer, observed, measurements };
    return () => { observer.disconnect(); frame?.(); naturalCells.current = null; };
  }, [widthScope]);
  useLayoutEffect(() => {
    const held = naturalCells.current!;
    if (!widthColumns.length) return;
    const { observer, observed } = held;
    for (const element of observed.keys()) if (!element.isConnected) { observer.unobserve(element); observed.delete(element); }
    for (const element of scroll.current?.querySelectorAll<HTMLElement>('[data-table-natural]') ?? []) {
      const key = element.dataset.tableNatural!;
      const row = element.closest<HTMLElement>('[data-table-row]')?.dataset.tableRow;
      const revision = row === undefined ? headerKeys : widthKeys.get(row)?.get(key) ?? layoutKeys;
      const cached = held.measurements.get(row)?.get(key);
      const previous = observed.get(element);
      observed.set(element, { key, row, revision, padding: previous?.padding ?? cached?.padding });
      if (!previous) observer.observe(element);
      else if (!sameRevision(previous.revision, revision)) { observer.unobserve(element); observer.observe(element); }
    }
  }, [naturalRevision, widthScope]);
  useEffect(() => {
    const held = naturalCells.current;
    if (!held?.measurements.size) return;
    const live = new Set(moving.map(row => row.key));
    for (const key of held.measurements.keys()) if (key !== undefined && !live.has(key)) held.measurements.delete(key);
  }, [moving]);
  const measuredRows = useRef<{ observer: ResizeObserver; observed: Set<Element> } | null>(null);
  // Measure only mounted rows. Prefix geometry uses the last measured height of
  // each identity; new rows use the minimum until they enter the viewport.
  useLayoutEffect(() => {
    if (!wrapping || !scroll.current) return;
    const pending = new Map<Element, readonly [string, number]>();
    let frame: (() => void) | null = null;
    const observer = new ResizeObserver(entries => {
      for (const entry of entries) {
        if (!entry.target.isConnected) continue;
        const element = entry.target as HTMLElement;
        pending.set(entry.target, [element.dataset.tableRow!, entry.borderBoxSize[0]?.blockSize ?? element.offsetHeight]);
      }
      if (frame) return;
      // Readings land with the frame's data, so what they commit is taken in the same frame.
      frame = engine.next('data', () => {
        frame = null;
        const readings = [...pending].filter(([element]) => element.isConnected).map(([, reading]) => reading);
        pending.clear();
        if (!readings.length) return;
        // Only a row taller than the rows' own height moves the rows below it: a row mounting at that height, or re-observed
        // at the height applied, commits nothing (run 13: a commit after each row arriving in every table).
        const applied = committedGeometry.current.rowMeasurements;
        if (withRowHeights(applied, readings, currentView.current.rowHeight, heightScope) === applied) return;
        inFrame(() => setRowMeasurements(before => withRowHeights(before, readings, currentView.current.rowHeight, heightScope)));
      });
    });
    measuredRows.current = { observer, observed: new Set() };
    return () => { observer.disconnect(); frame?.(); measuredRows.current = null; };
  }, [wrapping, heightScope]);
  useLayoutEffect(() => {
    const held = measuredRows.current;
    if (!held) return;
    for (const row of held.observed) if (!row.isConnected) { held.observer.unobserve(row); held.observed.delete(row); }
    for (const row of scroll.current?.querySelectorAll('tbody tr') ?? []) {
      if (!held.observed.has(row)) { held.observed.add(row); held.observer.observe(row); }
    }
  }, [mountedKeys, heightScope, wrapping]);
  // A gone row's height is read by no offset: it is dropped once gone rows outnumber the rows, so a row leaving costs no commit.
  useEffect(() => {
    if (rowMeasurements.heights.size <= Math.max(64, 2 * slotKeys.length)) return;
    const live = new Set(slotKeys);
    setRowMeasurements({ ...rowMeasurements, heights: new Map([...rowMeasurements.heights].filter(([key]) => live.has(key))) });
  }, [slotKeys, rowMeasurements]);
  useEffect(() => () => hideTooltip(owner), [owner]);
  // A scrollable tooltip this table owns takes the wheel, as the chart kits' do; the layer
  // itself ignores the pointer. Registered natively so the page scroll can be prevented.
  useEffect(() => {
    const element = scroll.current;
    if (!element) return;
    const wheel = (event: WheelEvent) => {
      const delta = event.deltaY * (event.deltaMode === 1 ? tableSize * 1.4 : 1);
      if (scrollTooltip(owner, delta, event.deltaMode === 2)) event.preventDefault();
    };
    element.addEventListener('wheel', wheel, { passive: false });
    return () => element.removeEventListener('wheel', wheel);
  }, [owner, tableSize]);
  const pointAt = (element: HTMLElement) => { const rect = element.getBoundingClientRect(); return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 }; };
  const contentFor = useCallback((row: T) => {
    if (!tooltip && !hiddenColumns.length) return null;
    const held = tipCache.current, key = rowKey(row);
    const values = live?.read() ?? null;
    // Every input of the combined content: the row, the widget's builder, the hidden and shown
    // columns, the row's key, the live values and the window height that decides whether it scrolls.
    if (held?.row !== row || held.build !== tooltip || held.hidden !== hiddenColumns || held.shown !== shownColumns || held.key !== key
      || held.viewport !== viewportHeight || held.values !== values) {
      // A hidden painted column gives its live value, as its cell would paint it.
      const hidden = values ? hiddenColumns.map(column => column.paint ? { ...column, text: (item: T) => {
        const key = liveKey(rowKey(item), column.key), value = values.get(key);
        return value == null || isMissing(value) ? null : column.paint!(value, key);
      } } : column) : hiddenColumns;
      tipCache.current = { row, build: tooltip, hidden: hiddenColumns, shown: shownColumns, key, viewport: viewportHeight, values,
        content: withHidden(tooltip?.(row) ?? null, row, hidden, shownColumns, key, viewportHeight) };
    }
    return tipCache.current!.content;
  }, [tooltip, hiddenColumns, shownColumns, rowKey, viewportHeight, live]);
  const clearTip = () => { activeTip.current = null; hideTooltip(owner); };
  const show = (key: string, row: T, origin: 'pointer' | 'keyboard', x: number, y: number) => {
    const content = contentFor(row);
    if (content) { activeTip.current = { key, origin, x, y }; showTooltip(owner, content, x, y); }
    return content;
  };
  useLayoutEffect(() => {
    const tip = activeTip.current;
    if (tip) {
      const row = ordered.find(row => rowKey(row) === tip.key);
      if (row === undefined) { activeTip.current = null; hideTooltip(owner); }
      else refreshTooltip(owner, () => {
        const content = contentFor(row);
        if (!content) activeTip.current = null;
        return content;
      });
    }
    const focused = focusedKey === null ? undefined : ordered.find(row => rowKey(row) === focusedKey);
    const content = focused === undefined ? null : contentFor(focused);
    const next = content && focusedKey !== null ? { key: focusedKey, text: tooltipText(content) } : null;
    if (description?.key !== next?.key || description?.text !== next?.text) setDescription(next);
  }, [ordered, rowKey, focusedKey, contentFor, owner, description]);
  // Painted cells: found after each commit, written in a frame whenever the live values move, never by a render.
  const liveCells = useRef<LiveCell<T>[]>([]);
  const livePaint = useRef<((force: boolean) => void) | null>(null);
  const liveLatest = useRef({ ordered, rowKey, liveSort, sortValues,
    orderDiffers: (_values: LiveValues<SortValue>): boolean => false, refreshTip: () => {} });
  liveLatest.current = { ordered, rowKey, liveSort, sortValues,
    orderDiffers: values => {
      const by = (row: T) => values.get(liveKey(rowKey(row), liveSort!));
      const next = treeChildren ? treeRows(rows, treeChildren, isOpen, arrangeBy(by), rowKey).rows : arrangeBy(by)(rows, 1);
      return next.length !== ordered.length || next.some((row, index) => rowKey(row) !== rowKey(ordered[index]));
    },
    refreshTip: () => {
      const tip = activeTip.current;
      const row = tip ? ordered.find(item => rowKey(item) === tip.key) : undefined;
      if (row !== undefined) refreshTooltip(owner, () => contentFor(row));
    } };
  useLayoutEffect(() => {
    if (!live) return;
    let painted: LiveValues<SortValue> | null = null, sortHeld: (() => void) | null = null;
    const task: FrameTask = { frame: () => { paint(false); return false; } };
    const wake = () => scheduler.wake(task);
    const paint = (force: boolean) => {
      const at = liveLatest.current, values = live.read();
      if (!force && values === painted) return;
      const moved = values !== painted;
      painted = values;
      const maxima = new Map<string, number | bigint>();
      for (const cell of liveCells.current) {
        if (cell.from) paintFrom(cell.from, cell.column, values.get(cell.from.key));
        const value = values.get(cell.key);
        if (value == null || isMissing(value)) { cell.text.put(cell.column.missingText?.(cell.key) ?? '—', values.missing(cell.key)); cell.bar?.put('0%'); continue; }
        cell.text.put(cell.column.paint!(value, cell.key), '');
        if (!cell.bar) continue;
        let maximum = maxima.get(cell.column.key);
        if (maximum === undefined) {
          const column = cell.column.key;
          maximum = cell.column.bar === 'visible' ? barMaximum(at.ordered, row => values.get(liveKey(at.rowKey(row), column))) : cell.column.bar as number | bigint;
          maxima.set(column, maximum);
        }
        cell.bar.put(typeof value === 'string' ? '0%' : `${barFraction(value, maximum) * 100}%`);
      }
      if (moved) at.refreshTip();
      if (at.liveSort === null || values === at.sortValues) return;
      if (!at.orderDiffers(values)) return;
      // A new order is structure: taken in the slot's task, painted again there from the newest values.
      if (!structureOpen()) { sortHeld ??= atNextSlot(() => { sortHeld = null; paint(true); }); return; }
      setSortedFrom({ live, values });
    };
    livePaint.current = paint;
    const remove = scheduler.add(task);
    const stop = live.subscribe(wake);
    return () => { remove(); stop(); sortHeld?.(); livePaint.current = null; };
  }, [live]);
  useLayoutEffect(() => {
    if (!live) { liveCells.current = []; return; }
    const painting = new Map(shownColumns.filter(column => column.paint).map(column => [column.key, column]));
    const cells: LiveCell<T>[] = [];
    for (const element of scroll.current?.querySelectorAll<HTMLElement>('[data-table-live]') ?? []) {
      const column = painting.get(element.dataset.tableLiveColumn ?? '');
      if (!column) continue;
      const bar = element.closest('.table-cell')?.querySelector<HTMLElement>('.table-cellbar');
      const fromKey = element.dataset.tableLiveFrom;
      const from = fromKey ? { key: fromKey, bar: bar ? new BarCell(bar.style, true) : null, element, tone: undefined } : null;
      cells.push({ key: element.dataset.tableLive!, column, text: textCell(element), bar: bar && !column.barOf ? new BarCell(bar.style, true) : null, from });
    }
    liveCells.current = cells;
    livePaint.current?.(true);
  });
  const open = (row: T, x: number, y: number) => {
    const items = rowMenu?.(row);
    if (!items?.length) return false;
    clearTip(); contextMenu.open(x, y, items);
    return true;
  };
  /** Whether the row at `slot` shows at the clock's hour as the frame last painted it (held ahead: revealed). */
  const revealedNow = (slot: number) => { const now = revealed.current; return !now || now.held.reveal[slot] <= now.hour; };
  const focusSlot = (slot: number) => {
    // A leaving row keeps focus while it fades, even once no row is left to move to. A row held ahead is never focused.
    if (slot < 0 || slot >= ordered.length || !revealedNow(slot)) return;
    const top = scrollToShow(offsets, slot, view.top, view.height, view.headerHeight);
    bottomAnchor.current = false;
    focusAnchor.current = true;
    setFocusedKey(rowKey(ordered[slot]));
    setView(before => ({ ...before, top }));
    scroll.current?.scrollTo({ top });
    requestAnimationFrame(() => scroll.current?.querySelector<HTMLElement>(`tr[aria-rowindex="${slot + 2}"]:not([aria-hidden="true"])`)?.focus());
  };
  const trackKeys = useRevision(shownColumns.flatMap(column => [column.key, column.width]));
  const tracks = useMemo(() => shownColumns.map(column => ({ key: column.key, width: column.width })), [trackKeys]);
  const grid = useMemo(() => tableGrid(tracks, rank, natural.scope === widthScope ? natural.widths : new Map()),
    [tracks, rank, natural, widthScope]);
  // Hide or restore one prioritised column per pass before paint, until the table itself fits (the scroller's padding masks a little overflow).
  useLayoutEffect(() => {
    const element = tableElement.current ?? scroll.current;
    if (!element || !order.length) return;
    const held = fit.scope === fitScope ? fit.fit : FULL_FIT;
    const next = fitColumns(held, order, element.clientWidth, element.scrollWidth);
    if (next !== held || fit.scope !== fitScope) setFit({ scope: fitScope, fit: next });
  }, [order, fitScope, fit, grid, view.width, headerKeys, rowHeight, fonts]);
  const naturalContent = (column: TableColumn<T>, content: ReactNode) => column.width === 'max-content'
    ? <span className="table-natural" data-table-natural={column.key}>{content}</span> : content;
  return <div ref={scroll} className={`chart-table${density === 'compact' ? ' is-compact' : ''}${wrapping ? ' is-wrapped' : ''}${measuring ? ' is-measuring' : ''}`}
    style={{ '--table-columns': grid, '--table-row-height': view.rowHeight > 0 ? `${view.rowHeight}px` : undefined,
      '--table-header-height': `${view.headerHeight || 24}px`, '--table-cell-y': rowHeight == null ? undefined : '0px', '--row-glide': `${glideMs}ms`, '--table-bar-glide': tokens.reducedMotion || motionNow(hoursPerSecond) === undefined ? '0ms' : '300ms' } as CSSProperties}
    onWheel={() => { focusAnchor.current = false; }} onPointerDown={() => { focusAnchor.current = false; }}
    onScroll={event => { const element = event.currentTarget; const top = element.scrollTop; bottomAnchor.current = element.scrollHeight - element.clientHeight - top < 1; setView(before => before.top === top ? before : { ...before, top }); }} onPointerLeave={() => { if (activeTip.current?.origin === 'pointer') clearTip(); }}>
    <table ref={tableElement} aria-label={label} aria-rowcount={ordered.length + 1} role={tree ? 'treegrid' : undefined}>
      <thead><tr>{rank && <th scope="col" className="table-rank">#</th>}{shownColumns.map((column, index) => <th key={column.key} scope="col"
        className={`${column.numeric ? 'is-numeric ' : ''}align-${columnAlign(column)}`} aria-label={column.label} data-tooltip={column.hint}
        aria-sort={column.sortable === false || unsorted ? undefined : sort.column === column.key ? sort.direction : 'none'}>
        {column.sortable === false || unsorted ? naturalContent(column, headers[index]) : <button type="button" aria-label={column.label} onClick={() => {
          const next = nextSort(sort, column.key, column.numeric ? 'descending' : 'ascending', column.direction);
          if (setOption) setOption('tableSort', sortOptionValue(next)); else setLocalSort(next);
        }}>{naturalContent(column, headers[index])}</button>}
      </th>)}</tr></thead>
      <tbody ref={body} style={{ height: offsets[count] - view.headerHeight }}>{mounted.map(row => {
        const marker = ranks.get(row.key);
        const change = marker && marker.until > renderedAt ? marker.delta : 0;
        const node = nesting?.get(row.key);
        const opens = node?.group === true && node.count > 0;
        const events: RowEvents = {
          onPointerMove: event => show(row.key, row.item.value, 'pointer', event.clientX, event.clientY),
          onFocus: event => {
            if (event.target !== event.currentTarget) return;
            focusAnchor.current = true;
            setFocusedKey(row.key);
            const point = pointAt(event.currentTarget); const content = show(row.key, row.item.value, 'keyboard', point.x, point.y);
            setDescription(content ? { key: row.key, text: tooltipText(content) } : null);
          },
          onBlur: () => { if (activeTip.current?.origin === 'keyboard') clearTip(); setFocusedKey(null); setDescription(null); },
          onClick: () => { if (!node?.group) onRowClick?.(row.item.value); else if (opens) toggle(row.item.value, !node.open); },
          onContextMenu: event => {
            if (keyboardMenu.current) { event.preventDefault(); keyboardMenu.current = false; }
            else if (open(row.item.value, event.clientX, event.clientY)) event.preventDefault();
          },
          onKeyDown: event => {
            if (event.target !== event.currentTarget) return;
            if ((event.key === 'PageUp' || event.key === 'PageDown') && scrollTooltip(owner, event.key === 'PageUp' ? -1 : 1, true)) {
              event.preventDefault(); return;
            }
            if (event.key === 'ContextMenu' || (event.key === 'F10' && event.shiftKey)) {
              const point = pointAt(event.currentTarget);
              if (open(row.item.value, point.x, point.y)) { event.preventDefault(); keyboardMenu.current = true; }
            } else if ((event.key === 'Enter' || event.key === ' ') && node?.group) {
              event.preventDefault(); if (!event.repeat && opens) toggle(row.item.value, !node.open);
            } else if ((event.key === 'Enter' || event.key === ' ') && onRowClick) {
              event.preventDefault(); if (!event.repeat) onRowClick(row.item.value);
            } else if (event.key === 'Escape') clearTip();
            else if (node && (event.key === 'ArrowRight' || event.key === 'ArrowLeft')) {
              // Tree keys open or close a group, then step into its first row or out to its header row.
              event.preventDefault();
              if (event.key === 'ArrowRight' && opens) {
                if (!node.open) toggle(row.item.value, true); else focusSlot(row.slot + 1);
              } else if (event.key === 'ArrowLeft') {
                if (opens && node.open) toggle(row.item.value, false);
                else if (node.parent !== null) { const slot = ordered.findIndex(item => rowKey(item) === node.parent); if (slot >= 0) focusSlot(slot); }
              }
            } else if (['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) {
              event.preventDefault();
              // Only over the rows the clock has revealed.
              const step = event.key === 'ArrowDown' || event.key === 'Home' ? 1 : -1;
              const slot = shownSlot(event.key === 'Home' ? 0 : event.key === 'End' ? ordered.length - 1 : row.slot + step, step, ordered.length, revealedNow);
              if (slot != null) focusSlot(slot);
            }
          },
          onPointerDown: () => { keyboardMenu.current = false; },
        };
        const isMuted = muted?.(row.item.value);
        const revision = [...revisions.get(row.key)!, layoutKeys, cellBars, rank, rankMarkers, row.item.rank, change, change && marker!.until,
          barRevision, row.phase, row.slot, offsets[row.slot] - view.headerHeight, liveColumns,
          node?.level, node?.size, node?.position, node?.group, node?.count, node?.open, description?.key === row.key,
          isMuted, !!onRowClick];
        return memoRow(row.key, revision, handlers => <tr data-table-row={row.key} className={`${movingRowClass(row)}${isMuted ? ' is-muted' : ''}${node?.group ? ' is-group' : ''}${opens || (onRowClick && !node?.group) ? ' is-activatable' : ''}`} tabIndex={row.phase === 'leaving' ? -1 : 0}
          aria-hidden={row.phase === 'leaving' || undefined} aria-rowindex={row.slot + 2}
          aria-level={node?.level} aria-setsize={node?.size} aria-posinset={node?.position} aria-expanded={opens ? node.open : undefined}
          aria-describedby={description?.key === row.key ? `${owner}-description` : undefined}
          style={{ transform: `translateY(${offsets[row.slot] - view.headerHeight}px)` }}
          {...handlers}>
          {rank && <td className="table-rank">{row.item.rank}{rankMarkers && change !== 0 && <span key={marker!.until} className={`table-delta ${change > 0 ? 'is-positive' : 'is-negative'}`}>{change > 0 ? '▲' : '▼'}{Math.abs(change)}</span>}</td>}
          {shownColumns.map((column, index) => {
            if (live && column.paint) {
              // Painted from the live values in a frame; React renders the empty cell once.
              const tone = column.cellClass?.(row.item.value);
              const structureBar = column.barOf?.(row.item.value) ?? null;
              const from = structureBar === 'cell' && column.barCell ? liveKey(row.key, column.barCell.id) : undefined;
              const text = <span className={tone ? `table-live ${tone}` : 'table-live'} data-table-live={liveKey(row.key, column.key)} data-table-live-column={column.key}
                data-table-live-from={from} />;
              const shown = node && index === 0 ? <TreeCell level={node.level} group={node.group} count={node.count} open={node.open}>{text}</TreeCell> : text;
              const bar = column.barOf ? <CellBar value={cellBars ? structureBar === 'cell' ? 0 : structureBar : null} maximum={typeof column.bar === 'string' || column.bar == null ? 0 : column.bar}>{shown}</CellBar>
                : <span className="table-cell">{cellBars && <i className="table-cellbar" aria-hidden="true" />}<span>{shown}</span></span>;
              return <td key={column.key} className={`${column.numeric ? 'is-numeric ' : ''}align-${columnAlign(column)}`}>
                {naturalContent(column, column.bar != null ? bar : shown)}
              </td>;
            }
            const value = column.value(row.item.value), missing = isMissing(value);
            const cell = cellContent(column, row.item.value, value);
            const content = node && index === 0 ? <TreeCell level={node.level} group={node.group} count={node.count} open={node.open}>{cell}</TreeCell> : cell;
            return <td key={column.key} className={`${column.numeric ? 'is-numeric ' : ''}align-${columnAlign(column)}`}>
              {naturalContent(column, column.bar != null ? <CellBar value={cellBars && !missing && (typeof value === 'number' || typeof value === 'bigint') ? value : null}
                maximum={maxima.get(column.key) ?? 0}>{content}</CellBar> : content)}
            </td>;
          })}
        </tr>, events);
      })}</tbody>
    </table>
    {description && <span id={`${owner}-description`} className="sr-only">{description.text}</span>}
    {contextMenu.menu}
  </div>;
}

/** A nested row's first cell, indented by depth; a group header row carries the timeline lanes' disclosure mark. */
function TreeCell({ level, group, count, open, children }: Pick<TreeRow, 'level' | 'group' | 'count' | 'open'> & { children: ReactNode }) {
  return <span className={`table-tree${group ? ' is-group' : ''}`} style={{ '--table-tree-depth': level - 1 } as CSSProperties}>
    {group && <svg className="table-disclosure" viewBox="-6 -6 12 12" aria-hidden="true">
      {count > 0 && <path d={open ? 'M-4 -2L4 -2L0 3Z' : 'M-2 -4L3 0L-2 4Z'} />}</svg>}
    <span className="table-tree-content">{children}</span></span>;
}

/** Composed cells may sort by one value and draw a bar from another. */
/** A cell's bar and tone painted from another live cell of its row (`barCell`): no bar and no tone while it has no
 * value. */
function paintFrom<T>(from: FromCell, column: TableColumn<T>, value: SortValue | undefined) {
  const known = typeof value === 'number' && Number.isFinite(value);
  const maximum = typeof column.bar === 'number' || typeof column.bar === 'bigint' ? column.bar : 0;
  from.bar?.put(known ? `${barFraction(column.barCell?.value ? column.barCell.value(value) : value, maximum) * 100}%` : '0%');
  const tone = known ? column.barCell?.tone?.(value) : undefined;
  if (tone === from.tone) return;
  if (from.tone) from.element.classList.remove(from.tone);
  if (tone) from.element.classList.add(tone);
  from.tone = tone;
}

export function CellBar({ value, maximum, children }: {
  value: number | bigint | null; maximum: number | bigint; children: ReactNode;
}) {
  return <span className="table-cell">{value !== null && <i className="table-cellbar" aria-hidden="true"
    style={{ transform: barScale(`${barFraction(value, maximum) * 100}%`) }} />}<span>{children}</span></span>;
}

export function CountryCell({ name, nameFailure, flag, flagReason, flagDetail, flagPending }: {
  name: string; nameFailure?: Error; flag?: string; flagReason?: string; flagDetail?: string; flagPending?: boolean;
}) {
  return <span className="table-country" data-detail={nameFailure?.message} title={name}><CountryFlagUrl className="table-country-flag"
    url={flag} detail={flagDetail} reason={flagReason} pending={flagPending} fixedSize /><span>{name}</span></span>;
}
