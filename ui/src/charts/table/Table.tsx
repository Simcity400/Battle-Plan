import { useCallback, useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { CSSProperties, ReactNode } from 'react';
import { flushSync } from 'react-dom';
import { useContextMenu } from '../../components/ContextMenu';
import { CountryFlagUrl } from '../../components/CountryFlag';
import type { MenuItem } from '../../components/ContextMenu';
import type { WidgetProps } from '../../dashboard/definitions';
import { hideTooltip, refreshTooltip, scrollTooltip, showTooltip, tooltipText } from '../tooltip';
import type { TooltipContent } from '../tooltip';
import { movingRowClass, useRowMotion } from '../useRowMotion';
import { useChartTokens, useViewportHeight } from '../useChartTokens';
import { easeTau } from '../easing';
import { FULL_FIT, barFraction, barMaximum, densityRowHeight, fitColumns, groupedRows, isMissing, nextSort, rankChanges, measuredRowOffsets, rowOffsets, scrollToShow, shownMotion, sortRows, treeRows, visibleRange } from './logic.ts';
import type { ColumnFit, RankChange, Sort, TableGroup, TableTree, TreeRow } from './logic';
import { cellContent, columnAlign, growColumnWidths, hideOrder, tableGrid, withHidden } from './columns.ts';
import type { TableColumn } from './columns';
import { sortKey } from './options.ts';
import type { NarrowTable } from './options';
import { useParityRows } from '../../dashboard/parityRows';
import type { ParityPreparation } from '../../dashboard/parityRows';
import '../charts.css';
import './table.css';

export type { TableColumn } from './columns';
export type { TableTree } from './logic';
const WINDOWING_THRESHOLD = 120;
const NO_KEYS: ReadonlySet<string> = new Set();
const NO_COLUMNS: readonly TableColumn<never>[] = [];
/** Unchanged markers keep their state, so new row objects in the same order do not re-render the table. */
const sameRanks = (before: ReadonlyMap<string, RankChange>, next: ReadonlyMap<string, RankChange>) => before.size === next.size
  && [...next].every(([key, change]) => { const held = before.get(key); return held?.rank === change.rank && held.delta === change.delta && held.until === change.until; });
export interface TableProps<T> {
  label: string; rows: readonly T[]; columns: readonly TableColumn<T>[]; rowKey: (row: T) => string;
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
  /** Complete recording operands, with optional history preparation. */
  parity?: { complete: boolean; preparation?: ParityPreparation; item?: (row: T) => unknown; reads?: unknown };
  hoursPerSecond?: number;
  /** Narrow tables hide prioritised columns into the row tooltip, or scroll sideways. */
  narrow?: NarrowTable;
}

/** style-5 Scoreboard: real table semantics, only the viewport's rows mounted. */
export default function Table<T>({ label, rows, columns, rowKey, tieKey = rowKey, groupBy, tree, unsorted = false, sort: storedSort, setOption, sortOptionValue = sortKey, cellBars = true,
  rank = false, rankMarkers = true, density = 'comfortable', rowHeight, onRowClick, muted, menu: rowMenu,
  tooltip, onVisibleRows, parity, hoursPerSecond = 1, narrow = 'hide' }: TableProps<T>) {
  const owner = useId();
  const tokens = useChartTokens();
  const viewportHeight = useViewportHeight();
  const keyboardMenu = useRef(false);
  const contextMenu = useContextMenu(`${label} actions`);
  const scroll = useRef<HTMLDivElement>(null);
  const tableElement = useRef<HTMLTableElement>(null);
  const [view, setView] = useState({ top: 0, height: 0, width: 0, rowHeight: 0, headerHeight: 0 });
  const [focusedKey, setFocusedKey] = useState<string | null>(null);
  const [description, setDescription] = useState<{ key: string; text: string } | null>(null);
  const tipCache = useRef<{ row: T; build: TableProps<T>['tooltip']; hidden: readonly TableColumn<T>[]; shown: readonly TableColumn<T>[];
    key: string; viewport: number; content: TooltipContent | null } | null>(null);
  const activeTip = useRef<{ key: string; origin: 'pointer' | 'keyboard'; x: number; y: number } | null>(null);
  const [localSort, setLocalSort] = useState<Sort | null>(null);
  const { tableSize, tableFont, wrapping } = useMemo(() => {
    if (typeof document === 'undefined' || typeof getComputedStyle !== 'function') return { tableSize: 12, tableFont: 'sans-serif', wrapping: true };
    const rootStyle = getComputedStyle(document.documentElement);
    return { tableSize: parseFloat(rootStyle.getPropertyValue('--type-sm')) || 12,
      tableFont: rootStyle.getPropertyValue('--font-tables'), wrapping: document.documentElement.dataset.wordWrap !== 'false' };
  }, [tokens]);
  const widthScope = JSON.stringify([density, tableFont, tableSize, wrapping, columns.map(column => [column.key, column.label, column.width])]);
  const [natural, setNatural] = useState<{ scope: string; widths: ReadonlyMap<string, number> }>({ scope: widthScope, widths: new Map() });
  const heightScope = `${widthScope}/${view.width}/${rowHeight ?? ''}`;
  const bottomAnchor = useRef(false);
  const focusAnchor = useRef(false);
  const [rowMeasurements, setRowMeasurements] = useState<{ scope: string; heights: ReadonlyMap<string, number> }>({ scope: heightScope, heights: new Map() });
  const requestedSort = setOption ? storedSort : localSort ?? storedSort;
  const sortedColumn = columns.find(column => column.key === requestedSort.column && column.sortable !== false)
    ?? columns.find(column => column.sortable !== false);
  const sort = { column: sortedColumn?.key ?? '', direction: sortedColumn?.direction ?? requestedSort.direction };
  // The row's identity (the first column) and the column it is sorted by never hide;
  // rows kept in their given order have no sort column.
  const order = useMemo(() => narrow === 'hide' ? hideOrder(columns).filter(key => (unsorted || key !== sort.column) && key !== columns[0]?.key) : [],
    [narrow, columns, sort.column, unsorted]);
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
  // Rebuilt column objects with the same accessor (such as new labels for another column) keep the order.
  const sortValue = sortedColumn?.value;
  // Groups the table holds itself: each toggled row's key and whether it is now open.
  const [toggled, setToggled] = useState<ReadonlyMap<string, boolean>>(new Map());
  const treeChildren = tree?.children, treeOpen = tree?.open, treeInitial = tree?.initiallyOpen, onToggle = tree?.onToggle;
  const isOpen = useCallback((row: T) => treeOpen ? treeOpen(row) : toggled.get(rowKey(row)) ?? treeInitial?.(row) ?? true,
    [treeOpen, treeInitial, toggled, rowKey]);
  const arrange = useCallback((list: readonly T[], level: number) => {
    if (unsorted) return list.slice();
    const value = sortValue ?? (() => null);
    return groupBy && level === 1 ? groupedRows(list, groupBy, value, sort.direction, rowKey, tieKey)
      : sortRows(list, value, sort.direction, rowKey, tieKey);
  }, [unsorted, sortValue, sort.direction, rowKey, tieKey, groupBy]);
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
  if (import.meta.env.MODE === 'parity') {
    useParityRows(scroll, () => {
      // Parity covers every nested row, whether its group is open or not.
      const full = treeChildren ? treeRows(rows, treeChildren, isOpen, arrange, rowKey, true) : null;
      const all = full?.rows ?? ordered;
      return { kind: 'table', label,
        complete: parity?.complete ?? true, total: all.length,
        columns: columns.map(column => ({ key: column.key, label: column.label })), sort, readOutcomes: parity?.reads,
        rows: all.map((row, index) => {
          const node = full?.nesting.get(rowKey(row));
          return { key: rowKey(row), index, ...(node ? { level: node.level, parent: node.parent, open: node.group ? node.open : undefined } : {}),
            item: parity?.item ? parity.item(row) : row, cells: columns.map(column => ({ key: column.key, value: column.value(row) })) };
        }) };
    }, parity?.preparation);
  }
  const moving = useRowMotion(items);
  const committedWindow = useRef<{ items: typeof items; keys: ReadonlySet<string> } | null>(null);
  const [departing, setDeparting] = useState(NO_KEYS);
  const glideMs = easeTau(tokens.motion, hoursPerSecond, tokens.reducedMotion) * 2.5;
  const [ranks, setRanks] = useState<ReadonlyMap<string, RankChange>>(new Map());
  const rankSort = useRef('');
  const ordering = `${groupBy ? 'grouped/' : ''}${tree ? 'tree/' : ''}${unsorted ? 'given' : sortKey(sort)}`;
  useLayoutEffect(() => {
    const reset = rankSort.current !== ordering;
    rankSort.current = ordering;
    setRanks(before => {
      if (!rank || !rankMarkers) return before.size ? new Map() : before;
      const next = rankChanges(before, items.map(item => item.key), performance.now(), 2500, reset);
      return sameRanks(before, next) ? before : next;
    });
  }, [items, rank, rankMarkers, ordering]);
  useEffect(() => {
    let due = Infinity;
    for (const item of ranks.values()) if (item.delta) due = Math.min(due, item.until);
    if (!Number.isFinite(due)) return;
    const timer = window.setTimeout(() => setRanks(before => rankChanges(before, items.map(item => item.key), performance.now())), Math.max(0, due - performance.now()) + 16);
    return () => window.clearTimeout(timer);
  }, [ranks, items]);
  useLayoutEffect(() => {
    const element = scroll.current;
    if (!element) return;
    const css = getComputedStyle(element);
    const comfortable = parseFloat(css.getPropertyValue('--table-comfortable-height'));
    const compact = parseFloat(css.getPropertyValue('--table-compact-height'));
    const base = rowHeight != null && Number.isFinite(rowHeight) && rowHeight > 0 ? rowHeight : comfortable;
    // The stylesheet's row heights are for 12px text, the default interface's --type-sm.
    const measuredHeight = densityRowHeight(base, density, compact / comfortable) * tableSize / 12;
    const headerHeight = Math.max(parseFloat(css.getPropertyValue('--table-header-min-height')), tableSize * 1.5 + 6);
    const measure = () => {
      const top = element.scrollTop, height = element.clientHeight, width = element.clientWidth;
      setView(before => before.top === top && before.height === height && before.width === width && before.rowHeight === measuredHeight
        && before.headerHeight === headerHeight ? before : { top, height, width, rowHeight: measuredHeight, headerHeight });
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, [density, rowHeight, tableSize, tableFont]);
  const count = moving.reduce((count, row) => tokens.reducedMotion && row.phase === 'leaving' ? count : Math.max(count, row.slot + 1), ordered.length);
  const leavingSlots = JSON.stringify(moving.filter(row => row.phase === 'leaving' && row.slot >= items.length).map(row => [row.slot, row.key]));
  const slotKeys = useMemo(() => {
    const keys = Array.from({ length: count }, (_, slot) => items[slot]?.key ?? '');
    for (const [slot, key] of JSON.parse(leavingSlots) as [number, string][]) keys[slot] = key;
    return keys;
  }, [count, items, leavingSlots]);
  const offsets = useMemo(() => wrapping
    ? measuredRowOffsets(slotKeys, rowMeasurements.scope === heightScope ? rowMeasurements.heights : new Map(), view.rowHeight, view.headerHeight)
    : rowOffsets(count, view.rowHeight, view.headerHeight), [wrapping, slotKeys, rowMeasurements, heightScope, count, view.rowHeight, view.headerHeight]);
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
  const [start, end] = count > WINDOWING_THRESHOLD ? visibleRange(offsets, view.top, view.height) : [0, count];
  // Keep previously visible DOM rows through the glide, even when their new
  // rank is outside the viewport. The existing motion hook owns their slots.
  const previousWindow = committedWindow.current;
  const crossing = previousWindow?.items !== items ? previousWindow?.keys : undefined;
  const mounted = shownMotion(moving, start, end, !tokens.reducedMotion,
    row => row.key === focusedKey || (!tokens.reducedMotion && (departing.has(row.key) || crossing?.has(row.key) === true)));
  useLayoutEffect(() => {
    const before = committedWindow.current;
    committedWindow.current = { items, keys: new Set(items.slice(start, end).map(row => row.key)) };
    if (before?.items === items) return;
    const keys = tokens.reducedMotion ? NO_KEYS : before?.keys ?? NO_KEYS;
    setDeparting(held => held.size || keys.size ? keys : held);
  }, [items, start, end, tokens.reducedMotion]);
  useEffect(() => {
    if (!departing.size) return;
    const timer = window.setTimeout(() => setDeparting(NO_KEYS), glideMs);
    return () => window.clearTimeout(timer);
  }, [departing, glideMs]);
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
  // Each mounted header/cell is observed once. Coalesce its readings before the
  // next paint so changing tracks cannot feed back into observer delivery.
  const naturalCells = useRef<{ observer: ResizeObserver; observed: Set<Element> } | null>(null);
  useLayoutEffect(() => {
    const pending = new Map<Element, { key: string; width: number }>();
    let frame = 0;
    const observer = new ResizeObserver(entries => {
      for (const { target } of entries) {
        if (!target.isConnected) continue;
        const element = target as HTMLElement;
        const css = getComputedStyle(element.closest('th, td')!);
        // Keep fractional layout pixels; offsetWidth rounds down and can trigger ellipsis.
        const width = parseFloat(getComputedStyle(element).width) + parseFloat(css.paddingLeft) + parseFloat(css.paddingRight);
        pending.set(target, { key: element.dataset.tableNatural!, width: Math.max(pending.get(target)?.width ?? 0, width) });
      }
      if (frame) return;
      frame = requestAnimationFrame(() => {
        frame = 0;
        const readings = [...pending].filter(([element]) => element.isConnected).map(([, reading]) => reading);
        pending.clear();
        if (!readings.length) return;
        flushSync(() => setNatural(before => {
          const previous = before.scope === widthScope ? before.widths : new Map<string, number>();
          const widths = growColumnWidths(previous, readings);
          return before.scope === widthScope && widths === previous ? before : { scope: widthScope, widths };
        }));
      });
    });
    naturalCells.current = { observer, observed: new Set() };
    return () => { observer.disconnect(); cancelAnimationFrame(frame); naturalCells.current = null; };
  }, [widthScope]);
  useLayoutEffect(() => {
    const { observer, observed } = naturalCells.current!;
    for (const element of observed) if (!element.isConnected) { observer.unobserve(element); observed.delete(element); }
    for (const element of scroll.current?.querySelectorAll('[data-table-natural]') ?? []) {
      if (!observed.has(element)) { observed.add(element); observer.observe(element); }
    }
  }, [columns, shownColumns, visible, widthScope]);
  const measuredRows = useRef<{ observer: ResizeObserver; observed: Set<Element> } | null>(null);
  // Measure only mounted rows. Prefix geometry uses the last measured height of
  // each identity; new rows use the minimum until they enter the viewport.
  useLayoutEffect(() => {
    if (!wrapping || !scroll.current) return;
    const pending = new Map<Element, readonly [string, number]>();
    let frame = 0;
    const observer = new ResizeObserver(entries => {
      for (const entry of entries) {
        if (!entry.target.isConnected) continue;
        const element = entry.target as HTMLElement;
        pending.set(entry.target, [element.dataset.tableRow!, entry.borderBoxSize[0]?.blockSize ?? element.offsetHeight]);
      }
      if (frame) return;
      frame = requestAnimationFrame(() => {
        frame = 0;
        const readings = [...pending].filter(([element]) => element.isConnected).map(([, reading]) => reading);
        pending.clear();
        if (!readings.length) return;
        flushSync(() => setRowMeasurements(before => {
          const previous = before.scope === heightScope ? before.heights : new Map<string, number>();
          let heights: Map<string, number> | undefined;
          for (const [key, height] of readings) if (previous.get(key) !== height) {
            heights ??= new Map(previous); heights.set(key, height);
          }
          return heights ? { scope: heightScope, heights } : before;
        }));
      });
    });
    measuredRows.current = { observer, observed: new Set() };
    return () => { observer.disconnect(); cancelAnimationFrame(frame); measuredRows.current = null; };
  }, [wrapping, heightScope]);
  useLayoutEffect(() => {
    const held = measuredRows.current;
    if (!held) return;
    for (const row of held.observed) if (!row.isConnected) { held.observer.unobserve(row); held.observed.delete(row); }
    for (const row of scroll.current?.querySelectorAll('tbody tr') ?? []) {
      if (!held.observed.has(row)) { held.observed.add(row); held.observer.observe(row); }
    }
  }, [mounted, heightScope, wrapping]);
  useEffect(() => {
    const live = new Set(slotKeys);
    setRowMeasurements(before => {
      if ([...before.heights.keys()].every(key => live.has(key))) return before;
      return { ...before, heights: new Map([...before.heights].filter(([key]) => live.has(key))) };
    });
  }, [slotKeys]);
  const maxima = useMemo(() => new Map(columns.filter(column => column.bar != null).map(column => [column.key,
    column.bar === 'visible' ? barMaximum(ordered, column.value) : column.bar as number | bigint])), [columns, ordered]);
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
    // Every input of the combined content: the row, the widget's builder, the hidden and shown
    // columns, the row's key and the window height that decides whether it scrolls.
    if (held?.row !== row || held.build !== tooltip || held.hidden !== hiddenColumns || held.shown !== shownColumns || held.key !== key || held.viewport !== viewportHeight)
      tipCache.current = { row, build: tooltip, hidden: hiddenColumns, shown: shownColumns, key, viewport: viewportHeight,
        content: withHidden(tooltip?.(row) ?? null, row, hiddenColumns, shownColumns, key, viewportHeight) };
    return tipCache.current!.content;
  }, [tooltip, hiddenColumns, shownColumns, rowKey, viewportHeight]);
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
    setDescription(content && focusedKey !== null ? { key: focusedKey, text: tooltipText(content) } : null);
  }, [ordered, rowKey, focusedKey, contentFor, owner]);
  const open = (row: T, x: number, y: number) => {
    const items = rowMenu?.(row);
    if (!items?.length) return false;
    clearTip(); contextMenu.open(x, y, items);
    return true;
  };
  const focusSlot = (slot: number) => {
    // A leaving row keeps focus while it fades, even once no row is left to move to.
    if (slot < 0 || slot >= ordered.length) return;
    const top = scrollToShow(offsets, slot, view.top, view.height, view.headerHeight);
    bottomAnchor.current = false;
    focusAnchor.current = true;
    setFocusedKey(rowKey(ordered[slot]));
    setView(before => ({ ...before, top }));
    scroll.current?.scrollTo({ top });
    requestAnimationFrame(() => scroll.current?.querySelector<HTMLElement>(`tr[aria-rowindex="${slot + 2}"]:not([aria-hidden="true"])`)?.focus());
  };
  const grid = tableGrid(shownColumns, rank, natural.scope === widthScope ? natural.widths : new Map());
  // Hide or restore one prioritised column per pass before paint, until the table itself fits (the scroller's padding masks a little overflow).
  useLayoutEffect(() => {
    const element = tableElement.current ?? scroll.current;
    if (!element || !order.length) return;
    setFit(before => {
      const held = before.scope === fitScope ? before.fit : FULL_FIT;
      const next = fitColumns(held, order, element.clientWidth, element.scrollWidth);
      return next === held && before.scope === fitScope ? before : { scope: fitScope, fit: next };
    });
  }, [order, fitScope, fit, grid, view.width]);
  const naturalContent = (column: TableColumn<T>, content: ReactNode) => column.width === 'max-content'
    ? <span className="table-natural" data-table-natural={column.key}>{content}</span> : content;
  return <div ref={scroll} className={`chart-table${density === 'compact' ? ' is-compact' : ''}${wrapping ? ' is-wrapped' : ''}${measuring ? ' is-measuring' : ''}`}
    style={{ '--table-columns': grid, '--table-row-height': view.rowHeight > 0 ? `${view.rowHeight}px` : undefined,
      '--table-header-height': `${view.headerHeight || 24}px`, '--table-cell-y': rowHeight == null ? undefined : '0px', '--row-glide': `${glideMs}ms`, '--table-bar-glide': tokens.reducedMotion ? '0ms' : '300ms' } as CSSProperties}
    onWheel={() => { focusAnchor.current = false; }} onPointerDown={() => { focusAnchor.current = false; }}
    onScroll={event => { const element = event.currentTarget; const top = element.scrollTop; bottomAnchor.current = element.scrollHeight - element.clientHeight - top < 1; setView(before => before.top === top ? before : { ...before, top }); }} onPointerLeave={() => { if (activeTip.current?.origin === 'pointer') clearTip(); }}>
    <table ref={tableElement} aria-label={label} aria-rowcount={ordered.length + 1} role={tree ? 'treegrid' : undefined}>
      <thead><tr>{rank && <th scope="col" className="table-rank">#</th>}{shownColumns.map(column => <th key={column.key} scope="col"
        className={`${column.numeric ? 'is-numeric ' : ''}align-${columnAlign(column)}`} aria-label={column.label} data-tooltip={column.hint}
        aria-sort={column.sortable === false || unsorted ? undefined : sort.column === column.key ? sort.direction : 'none'}>
        {column.sortable === false || unsorted ? naturalContent(column, column.header) : <button type="button" aria-label={column.label} onClick={() => {
          const next = nextSort(sort, column.key, column.numeric ? 'descending' : 'ascending', column.direction);
          if (setOption) setOption('tableSort', sortOptionValue(next)); else setLocalSort(next);
        }}>{naturalContent(column, <>{column.header}{(sort.column === column.key || column.width === 'max-content') && <span className="table-sort" aria-hidden="true" style={{ visibility: sort.column === column.key ? undefined : 'hidden' }}> {sort.direction === 'descending' ? '▾' : '▴'}</span>}</>)}</button>}
      </th>)}</tr></thead>
      <tbody style={{ height: offsets[count] - view.headerHeight }}>{mounted.map(row => {
        const change = ranks.get(row.key)?.delta ?? 0;
        const node = nesting?.get(row.key);
        const opens = node?.group === true && node.count > 0;
        return <tr key={row.key} data-table-row={row.key} className={`${movingRowClass(row)}${muted?.(row.item.value) ? ' is-muted' : ''}${node?.group ? ' is-group' : ''}${opens || (onRowClick && !node?.group) ? ' is-activatable' : ''}`} tabIndex={row.phase === 'leaving' ? -1 : 0}
          aria-hidden={row.phase === 'leaving' || undefined} aria-rowindex={row.slot + 2}
          aria-level={node?.level} aria-setsize={node?.size} aria-posinset={node?.position} aria-expanded={opens ? node.open : undefined}
          aria-describedby={description?.key === row.key ? `${owner}-description` : undefined}
          style={{ transform: `translateY(${offsets[row.slot] - view.headerHeight}px)` }}
          onPointerMove={event => show(row.key, row.item.value, 'pointer', event.clientX, event.clientY)}
          onFocus={event => {
            if (event.target !== event.currentTarget) return;
            focusAnchor.current = true;
            setFocusedKey(row.key);
            const point = pointAt(event.currentTarget); const content = show(row.key, row.item.value, 'keyboard', point.x, point.y);
            setDescription(content ? { key: row.key, text: tooltipText(content) } : null);
          }}
          onBlur={() => { if (activeTip.current?.origin === 'keyboard') clearTip(); setFocusedKey(null); setDescription(null); }}
          onClick={() => { if (!node?.group) onRowClick?.(row.item.value); else if (opens) toggle(row.item.value, !node.open); }}
          onContextMenu={event => {
            if (keyboardMenu.current) { event.preventDefault(); keyboardMenu.current = false; }
            else if (open(row.item.value, event.clientX, event.clientY)) event.preventDefault();
          }}
          onKeyDown={event => {
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
              focusSlot(event.key === 'Home' ? 0 : event.key === 'End' ? ordered.length - 1 : Math.max(0, Math.min(ordered.length - 1, row.slot + (event.key === 'ArrowDown' ? 1 : -1))));
            }
          }} onPointerDown={() => { keyboardMenu.current = false; }}>
          {rank && <td className="table-rank">{row.item.rank}{rankMarkers && change !== 0 && <span className={`table-delta ${change > 0 ? 'is-positive' : 'is-negative'}`}>{change > 0 ? '▲' : '▼'}{Math.abs(change)}</span>}</td>}
          {shownColumns.map((column, index) => {
            const value = column.value(row.item.value);
            const missing = isMissing(value);
            const cell = cellContent(column, row.item.value, value);
            const content = node && index === 0 ? <TreeCell node={node}>{cell}</TreeCell> : cell;
            return <td key={column.key} className={`${column.numeric ? 'is-numeric ' : ''}align-${columnAlign(column)}`}>
              {naturalContent(column, column.bar != null ? <CellBar value={cellBars && !missing && (typeof value === 'number' || typeof value === 'bigint') ? value : null}
                maximum={maxima.get(column.key) ?? 0}>{content}</CellBar> : content)}
            </td>;
          })}
        </tr>;
      })}</tbody>
    </table>
    {description && <span id={`${owner}-description`} className="sr-only">{description.text}</span>}
    {contextMenu.menu}
  </div>;
}

/** A nested row's first cell, indented by depth; a group header row carries the timeline lanes' disclosure mark. */
function TreeCell({ node, children }: { node: TreeRow; children: ReactNode }) {
  return <span className={`table-tree${node.group ? ' is-group' : ''}`} style={{ '--table-tree-depth': node.level - 1 } as CSSProperties}>
    {node.group && <svg className="table-disclosure" viewBox="-6 -6 12 12" aria-hidden="true">
      {node.count > 0 && <path d={node.open ? 'M-4 -2L4 -2L0 3Z' : 'M-2 -4L3 0L-2 4Z'} />}</svg>}
    <span className="table-tree-content">{children}</span></span>;
}

/** Composed cells may sort by one value and draw a bar from another. */
export function CellBar({ value, maximum, children }: {
  value: number | bigint | null; maximum: number | bigint; children: ReactNode;
}) {
  return <span className="table-cell">{value !== null && <i className="table-cellbar" aria-hidden="true"
    style={{ width: `${barFraction(value, maximum) * 100}%` }} />}<span>{children}</span></span>;
}

export function CountryCell({ name, flag, flagReason, flagPending }: {
  name: string; flag?: string; flagReason?: string; flagPending?: boolean;
}) {
  return <span className="table-country" title={name}><CountryFlagUrl className="table-country-flag"
    url={flag} reason={flagReason} pending={flagPending} /><span>{name}</span></span>;
}
