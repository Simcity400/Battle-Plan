import { createContext, use, useCallback, useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { CSSProperties, ReactNode } from 'react';
import { inFrame } from '../../dashboard/frameUpdates.ts';
import { useContextMenu } from '../../components/ContextMenu';
import { CountryFlagUrl } from '../../components/CountryFlag';
import type { MenuItem } from '../../components/ContextMenu';
import type { WidgetProps } from '../../dashboard/definitions';
import { hideTooltip, refreshTooltip, scrollTooltip, showTooltip, tooltipText } from '../tooltip';
import { liveKey } from '../live';
import { useWhyCommit } from '../../dashboard/widgetPerf.ts';
import type { Live, LiveValues } from '../live';
import { scheduler } from '../scheduler';
import type { FrameTask } from '../scheduler';
import { BarCell, textCell } from '../../dashboard/follow/painters.ts';
import { barGlides, rowGlides } from '../../engine/glide.ts';
import { rowFades } from '../../engine/entry.ts';
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
const WINDOWING_THRESHOLD = 120;
const NO_KEYS: ReadonlySet<string> = new Set();
const NO_RANKS: ReadonlyMap<string, RankChange> = new Map();
const RANK_HOLD_MS = 2500;
interface NaturalCell { key: string; row: string | undefined; revision: RowRevision; padding?: number }
interface NaturalMeasurement { revision: RowRevision; width: number; padding: number }
const NO_COLUMNS: readonly TableColumn<never>[] = [];
const sameRanks = (before: ReadonlyMap<string, RankChange>, next: ReadonlyMap<string, RankChange>) => before.size === next.size
  && [...next].every(([key, change]) => { const held = before.get(key); return held?.rank === change.rank && held.delta === change.delta && held.until === change.until; });
export interface TableProps<T> {
  label: string; rows: readonly T[]; columns: readonly TableColumn<T>[]; rowKey: (row: T) => string;
  rowRevision?: (row: T) => RowRevision;
  tieKey?: (row: T) => string;
  groupBy?: (row: T) => TableGroup;
  tree?: TableTree<T>;
  unsorted?: boolean;
  sort: Sort; setOption?: WidgetProps['setOption'];
  sortOptionValue?: (sort: Sort) => string;
  cellBars?: boolean; rank?: boolean; rankMarkers?: boolean; density?: 'comfortable' | 'compact';
  rowHeight?: number;
  onRowClick?: (row: T) => void;
  muted?: (row: T) => boolean;
  menu?: (row: T) => MenuItem[];
  tooltip?: (row: T) => TooltipContent;
  onVisibleRows?: (rows: readonly T[]) => void;
  hoursPerSecond?: number;
  narrow?: NarrowTable;
  live?: Live<LiveValues<SortValue>>;
  reveal?: { clock: RevealClock; at: (row: T) => number };
}

interface Held { reveal: Float64Array; sorted: Float64Array; slots: ReadonlyMap<string, number> }

function revealedThrough(sorted: Float64Array, hour: number): number {
  let low = 0, high = sorted.length;
  while (low < high) { const middle = (low + high) >> 1; if (sorted[middle] <= hour) low = middle + 1; else high = middle; }
  return low;
}

function widenOverAhead(held: Held | null, hour: number | null, start: number, end: number, count: number): [number, number] {
  if (!held || hour === null) return [start, end];
  while (start > 0 && held.reveal[start - 1] > hour) start--;
  while (end < count && held.reveal[end] > hour) end++;
  return [start, end];
}
function revealedOffsets(offsets: readonly number[], held: Held, hour: number): readonly number[] {
  if (revealedThrough(held.sorted, hour) === held.sorted.length) return offsets;
  const shown = [offsets[0]];
  for (let slot = 0; slot < offsets.length - 1; slot++) shown.push(shown[slot] + (held.reveal[slot] > hour ? 0 : offsets[slot + 1] - offsets[slot]));
  return shown;
}

interface FromCell { key: string; bar: BarCell | null; element: HTMLElement; tone: string | undefined }
interface LiveCell<T> { key: string; column: TableColumn<T>; text: TextCell; bar: BarCell | null; from: FromCell | null }

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
  const order = useRevision(narrow === 'hide' ? hideOrder(columns).filter(key => (unsorted || key !== sort.column) && key !== columns[0]?.key) : []);
  const [fit, setFit] = useState<{ scope: string; fit: ColumnFit }>({ scope: '', fit: FULL_FIT });
  const fitScope = JSON.stringify([order, widthScope, rank]);
  const columnFit = fit.scope === fitScope ? fit.fit : FULL_FIT;
  const shownColumns = useMemo(() => {
    if (!columnFit.hidden.length) return columns;
    const hidden = new Set(columnFit.hidden);
    return columns.filter(column => !hidden.has(column.key));
  }, [columns, columnFit]);
  const hiddenColumns = useMemo(() => columnFit.hidden.length
    ? columns.filter(column => columnFit.hidden.includes(column.key) && !column.inTooltip) : NO_COLUMNS as readonly TableColumn<T>[], [columns, columnFit]);
  const liveSort = live && !unsorted && sortedColumn && (sortedColumn.paint || sortedColumn.sortsLive) ? sortedColumn.key : null;
  const [sortedFrom, setSortedFrom] = useState<{ live: typeof live; values: LiveValues<SortValue> } | null>(null);
  const sortValues = liveSort === null ? null : sortedFrom && sortedFrom.live === live ? sortedFrom.values : live!.read();
  const columnValue = sortedColumn?.value;
  const sortValue = useMemo(() => sortValues && liveSort !== null ? (row: T) => sortValues.get(liveKey(rowKey(row), liveSort)) : columnValue,
    [sortValues, liveSort, rowKey, columnValue]);
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
  const committedWindow = useRef<{ items: typeof items; keys: ReadonlySet<string>; departing: ReadonlySet<string> } | null>(null);
  const motionNow = useMotionNow();
  const glideMs = easeTau(tokens.motion, motionNow(hoursPerSecond), tokens.reducedMotion) * 2.5;
  const barGlideMs = tokens.reducedMotion || motionNow(hoursPerSecond) === undefined ? 0 : BAR_GLIDE_MS;
  const barMotion = useRef({ reduced: tokens.reducedMotion, given: hoursPerSecond });
  barMotion.current = { reduced: tokens.reducedMotion, given: hoursPerSecond };
  const barGlide = useCallback(() => barMotion.current.reduced || motionNow(barMotion.current.given) === undefined ? 0 : BAR_GLIDE_MS, [motionNow]);
  const ordering = `${groupBy ? 'grouped/' : ''}${tree ? 'tree/' : ''}${unsorted ? 'given' : sortKey(sort)}`;
  const committedRanks = useRef<{ ordering: string; ranks: ReadonlyMap<string, RankChange> }>({ ordering: '', ranks: NO_RANKS });
  const drawnRanks = useMemo(() => {
    const at = performance.now();
    if (!rank || !rankMarkers) return { ranks: NO_RANKS, at };
    const held = committedRanks.current;
    const next = rankChanges(held.ranks, items.map(item => item.key), at, RANK_HOLD_MS, held.ordering !== ordering);
    return { ranks: sameRanks(held.ranks, next) ? held.ranks : next, at };
  }, [items, rank, rankMarkers, ordering]);
  const ranks = drawnRanks.ranks;
  useLayoutEffect(() => {
    const shift = performance.now() - drawnRanks.at;
    let shown: Map<string, RankChange> | null = null;
    for (const [key, item] of ranks) {
      if (!item.delta || item.until !== drawnRanks.at + RANK_HOLD_MS) continue;
      shown ??= new Map(ranks);
      shown.set(key, { ...item, until: item.until + shift });
    }
    committedRanks.current = { ordering, ranks: shown ?? ranks };
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
  }, [drawnRanks, ordering]);
  useLayoutEffect(() => {
    const element = scroll.current;
    if (!element) return;
    if (!comfortable) return;
    const base = rowHeight != null && Number.isFinite(rowHeight) && rowHeight > 0 ? rowHeight : comfortable;
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
  const body = useRef<HTMLTableSectionElement>(null);
  const revealed = useRef<{ full: readonly number[]; held: Held; header: number; cut: number; hour: number } | null>(null);
  const rowGlide = useRef(0);
  const drawnSlots = useRef<ReadonlyMap<string, { slot: number; rank: number }>>(new Map());
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
      rowGlides.to(row.style, shown[slot] - now.header, rowGlide.current);
      const ahead = now.held.reveal[slot] > now.hour, was = row.dataset.reveal;
      if (ahead) { if (was !== 'ahead') { row.dataset.reveal = 'ahead'; row.inert = true; } }
      else if (was === 'ahead') { if (enter) { row.dataset.reveal = 'in'; rowFades.enter(row); } else delete row.dataset.reveal; row.inert = false; }
    }
  }, []);
  rowGlide.current = measuring ? 0 : glideMs;
  useLayoutEffect(() => {
    revealed.current = held && revealCut !== null && hourHeld.current !== null
      ? { full: fullOffsets, held, header: view.headerHeight, cut: revealCut, hour: hourHeld.current } : null;
    if (revealed.current) paintRevealed(false);
    for (const row of body.current?.children as HTMLCollectionOf<HTMLElement> | undefined ?? []) {
      const drawn = drawnSlots.current.get(row.dataset.tableRow ?? '');
      if (!drawn) continue;
      const index = String(drawn.slot + 2);
      if (row.getAttribute('aria-rowindex') !== index) row.setAttribute('aria-rowindex', index);
      const rankCell = row.firstElementChild;
      if (rank && !rankMarkers && rankCell?.classList.contains('table-rank') && rankCell.textContent !== String(drawn.rank)) rankCell.textContent = String(drawn.rank);
      if (!revealed.current && drawn.slot < offsets.length) rowGlides.to(row.style, offsets[drawn.slot] - view.headerHeight, rowGlide.current);
      if (row.classList.contains('is-entering') && row.dataset.reveal !== 'ahead' && !enteredRows.has(row)) { enteredRows.add(row); rowFades.enter(row); }
      if (row.classList.contains('is-leaving')) rowFades.leave(row);
      else rowFades.stay(row);
    }
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
  drawnSlots.current = new Map(mounted.map(row => [row.key, { slot: row.slot, rank: row.item.rank }]));
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
      frame = engine.next('data', () => {
        frame = null;
        const readings = [...pending].filter(([element]) => element.isConnected).map(([, reading]) => reading);
        pending.clear();
        if (!readings.length) return;
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
  useEffect(() => {
    if (rowMeasurements.heights.size <= Math.max(64, 2 * slotKeys.length)) return;
    const live = new Set(slotKeys);
    setRowMeasurements({ ...rowMeasurements, heights: new Map([...rowMeasurements.heights].filter(([key]) => live.has(key))) });
  }, [slotKeys, rowMeasurements]);
  useEffect(() => () => hideTooltip(owner), [owner]);
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
    if (held?.row !== row || held.build !== tooltip || held.hidden !== hiddenColumns || held.shown !== shownColumns || held.key !== key
      || held.viewport !== viewportHeight || held.values !== values) {
      tipCache.current = { row, build: tooltip, hidden: hiddenColumns, shown: shownColumns, key, viewport: viewportHeight, values,
        content: withHidden(tooltip?.(row) ?? null, row, hiddenColumns, shownColumns, key, viewportHeight, values) };
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
    let painted: LiveValues<SortValue> | null = null;
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
        if (value == null || isMissing(value)) { const reason = values.missing(cell.key); cell.text.put(cell.column.missingText?.(cell.key, reason) ?? '—', reason); cell.bar?.put('0%'); continue; }
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
      setSortedFrom({ live, values });
    };
    livePaint.current = paint;
    const remove = scheduler.add(task);
    const stop = live.subscribe(wake);
    return () => { remove(); stop(); livePaint.current = null; };
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
      const from = fromKey ? { key: fromKey, bar: bar ? new BarCell(bar.style, true, barGlide) : null, element, tone: undefined } : null;
      cells.push({ key: element.dataset.tableLive!, column, text: textCell(element), bar: bar && !column.barOf ? new BarCell(bar.style, true, barGlide) : null, from });
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
  const revealedNow = (slot: number) => { const now = revealed.current; return !now || now.held.reveal[slot] <= now.hour; };
  const focusSlot = (slot: number) => {
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
  useLayoutEffect(() => {
    const element = tableElement.current ?? scroll.current;
    if (!element || !order.length) return;
    const held = fit.scope === fitScope ? fit.fit : FULL_FIT;
    const next = fitColumns(held, order, element.clientWidth, element.scrollWidth);
    if (next !== held || fit.scope !== fitScope) setFit({ scope: fitScope, fit: next });
  }, [order, fitScope, fit, grid, view.width, headerKeys, rowHeight, fonts]);
  const naturalContent = (column: TableColumn<T>, content: ReactNode) => column.width === 'max-content'
    ? <span className="table-natural" data-table-natural={column.key}>{content}</span> : content;
  return <BarGlideMs value={barGlideMs}><div ref={scroll} className={`chart-table${density === 'compact' ? ' is-compact' : ''}${wrapping ? ' is-wrapped' : ''}${measuring ? ' is-measuring' : ''}`}
    style={{ '--table-columns': grid, '--table-row-height': view.rowHeight > 0 ? `${view.rowHeight}px` : undefined,
      '--table-header-height': `${view.headerHeight || 24}px`, '--table-cell-y': rowHeight == null ? undefined : '0px', '--row-glide': `${glideMs}ms` } as CSSProperties}
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
              event.preventDefault();
              if (event.key === 'ArrowRight' && opens) {
                if (!node.open) toggle(row.item.value, true); else focusSlot(row.slot + 1);
              } else if (event.key === 'ArrowLeft') {
                if (opens && node.open) toggle(row.item.value, false);
                else if (node.parent !== null) { const slot = ordered.findIndex(item => rowKey(item) === node.parent); if (slot >= 0) focusSlot(slot); }
              }
            } else if (['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) {
              event.preventDefault();
              const step = event.key === 'ArrowDown' || event.key === 'Home' ? 1 : -1;
              const slot = shownSlot(event.key === 'Home' ? 0 : event.key === 'End' ? ordered.length - 1 : row.slot + step, step, ordered.length, revealedNow);
              if (slot != null) focusSlot(slot);
            }
          },
          onPointerDown: () => { keyboardMenu.current = false; },
        };
        const isMuted = muted?.(row.item.value);
        const placedRevision = rankMarkers ? [row.item.rank, row.slot] : NO_PLACE;
        const revision = [...revisions.get(row.key)!, layoutKeys, cellBars, rank, rankMarkers, ...placedRevision, change, change && marker!.until,
          barRevision, row.phase, liveColumns,
          node?.level, node?.size, node?.position, node?.group, node?.count, node?.open, description?.key === row.key,
          isMuted, !!onRowClick];
        return memoRow(row.key, revision, handlers => <tr data-table-row={row.key} className={`${movingRowClass(row)}${isMuted ? ' is-muted' : ''}${node?.group ? ' is-group' : ''}${opens || (onRowClick && !node?.group) ? ' is-activatable' : ''}`} tabIndex={row.phase === 'leaving' ? -1 : 0}
          aria-hidden={row.phase === 'leaving' || undefined} aria-rowindex={row.slot + 2}
          aria-level={node?.level} aria-setsize={node?.size} aria-posinset={node?.position} aria-expanded={opens ? node.open : undefined}
          aria-describedby={description?.key === row.key ? `${owner}-description` : undefined}
          {...handlers}>
          {rank && <td className="table-rank">{row.item.rank}{rankMarkers && change !== 0 && <span key={marker!.until} className={`table-delta ${change > 0 ? 'is-positive' : 'is-negative'}`}>{change > 0 ? '▲' : '▼'}{Math.abs(change)}</span>}</td>}
          {shownColumns.map((column, index) => {
            if (live && column.paint) {
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
  </div></BarGlideMs>;
}

function TreeCell({ level, group, count, open, children }: Pick<TreeRow, 'level' | 'group' | 'count' | 'open'> & { children: ReactNode }) {
  return <span className={`table-tree${group ? ' is-group' : ''}`} style={{ '--table-tree-depth': level - 1 } as CSSProperties}>
    {group && <svg className="table-disclosure" viewBox="-6 -6 12 12" aria-hidden="true">
      {count > 0 && <path d={open ? 'M-4 -2L4 -2L0 3Z' : 'M-2 -4L3 0L-2 4Z'} />}</svg>}
    <span className="table-tree-content">{children}</span></span>;
}

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

const NO_PLACE: readonly unknown[] = [];
const enteredRows = new WeakSet<HTMLElement>();

const BarGlideMs = createContext(0);
const BAR_GLIDE_MS = 300;

export function CellBar({ value, maximum, children }: {
  value: number | bigint | null; maximum: number | bigint; children: ReactNode;
}) {
  const glide = use(BarGlideMs);
  const bar = useRef<HTMLElement>(null);
  const fraction = value === null ? null : barFraction(value, maximum);
  useLayoutEffect(() => { if (bar.current && fraction !== null) barGlides.to(bar.current.style, fraction, glide); }, [fraction, glide]);
  return <span className="table-cell">{fraction !== null && <i ref={bar} className="table-cellbar" aria-hidden="true" />}<span>{children}</span></span>;
}

export function CountryCell({ name, nameFailure, flag, flagReason, flagDetail, flagPending }: {
  name: string; nameFailure?: Error; flag?: string; flagReason?: string; flagDetail?: string; flagPending?: boolean;
}) {
  return <span className="table-country" data-detail={nameFailure?.message} title={name}><CountryFlagUrl className="table-country-flag"
    url={flag} detail={flagDetail} reason={flagReason} pending={flagPending} fixedSize /><span>{name}</span></span>;
}
