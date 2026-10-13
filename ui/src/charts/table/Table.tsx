import { useCallback, useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { CSSProperties, ReactNode } from 'react';
import { flushSync } from 'react-dom';
import { useContextMenu } from '../../components/ContextMenu';
import { CountryFlagUrl } from '../../components/CountryFlag';
import type { MenuItem } from '../../components/ContextMenu';
import type { WidgetProps } from '../../dashboard/definitions';
import { hideTooltip, refreshTooltip, showTooltip, tooltipText } from '../tooltip';
import type { TooltipContent } from '../tooltip';
import { movingRowClass, useRowMotion } from '../useRowMotion';
import { useChartTokens } from '../useChartTokens';
import { easeTau } from '../easing';
import { barFraction, barMaximum, densityRowHeight, groupedRows, isMissing, nextSort, rankChanges, measuredRowOffsets, rowOffsets, scrollToShow, shownMotion, sortRows, visibleRange } from './logic.ts';
import type { RankChange, Sort, TableGroup } from './logic';
import { cellContent, columnAlign, growColumnWidths, tableGrid } from './columns.ts';
import type { TableColumn } from './columns';
import { sortKey } from './options.ts';
import '../charts.css';
import './table.css';

export type { TableColumn } from './columns';
const WINDOWING_THRESHOLD = 120;
const NO_KEYS: ReadonlySet<string> = new Set();
/** Unchanged markers keep their state, so new row objects in the same order do not re-render the table. */
const sameRanks = (before: ReadonlyMap<string, RankChange>, next: ReadonlyMap<string, RankChange>) => before.size === next.size
  && [...next].every(([key, change]) => { const held = before.get(key); return held?.rank === change.rank && held.delta === change.delta && held.until === change.until; });
export interface TableProps<T> {
  label: string; rows: readonly T[]; columns: readonly TableColumn<T>[]; rowKey: (row: T) => string;
  /** Current recorded tag order may differ from a country's stable motion key. */
  tieKey?: (row: T) => string;
  /** Optional grouping keeps the same row identities, geometry and exact values. */
  groupBy?: (row: T) => TableGroup;
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
}

/** style-5 Scoreboard: real table semantics, only the viewport's rows mounted. */
export default function Table<T>({ label, rows, columns, rowKey, tieKey = rowKey, groupBy, sort: storedSort, setOption, sortOptionValue = sortKey, cellBars = true,
  rank = false, rankMarkers = true, density = 'comfortable', rowHeight, onRowClick, muted, menu: rowMenu,
  tooltip, onVisibleRows, hoursPerSecond = 1 }: TableProps<T>) {
  const owner = useId();
  const tokens = useChartTokens();
  const keyboardMenu = useRef(false);
  const contextMenu = useContextMenu(`${label} actions`);
  const scroll = useRef<HTMLDivElement>(null);
  const [view, setView] = useState({ top: 0, height: 0, width: 0, rowHeight: 0, headerHeight: 0 });
  const [focusedKey, setFocusedKey] = useState<string | null>(null);
  const [description, setDescription] = useState<{ key: string; text: string } | null>(null);
  const tipCache = useRef<{ row: T; build: NonNullable<TableProps<T>['tooltip']>; content: TooltipContent } | null>(null);
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
  // Rebuilt column objects with the same accessor (such as new labels for another column) keep the order.
  const sortValue = sortedColumn?.value;
  const ordered = useMemo(() => {
    const value = sortValue ?? (() => null);
    return groupBy ? groupedRows(rows, groupBy, value, sort.direction, rowKey, tieKey)
      : sortRows(rows, value, sort.direction, rowKey, tieKey);
  }, [rows, sortValue, sort.direction, rowKey, tieKey, groupBy]);
  const items = useMemo(() => ordered.map((item, slot) => ({ key: rowKey(item), item: { value: item, rank: slot + 1 } })), [ordered, rowKey]);
  const moving = useRowMotion(items);
  const committedWindow = useRef<{ items: typeof items; keys: ReadonlySet<string> } | null>(null);
  const [departing, setDeparting] = useState(NO_KEYS);
  const glideMs = easeTau(tokens.motion, hoursPerSecond, tokens.reducedMotion) * 2.5;
  const [ranks, setRanks] = useState<ReadonlyMap<string, RankChange>>(new Map());
  const rankSort = useRef('');
  const ordering = `${groupBy ? 'grouped/' : ''}${sortKey(sort)}`;
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
  // Each mounted header/cell is observed once. Its first observation and every
  // resize arrive before paint, and a wider one grows its track in that frame.
  const naturalCells = useRef<{ observer: ResizeObserver; observed: Set<Element> } | null>(null);
  useLayoutEffect(() => {
    const observer = new ResizeObserver(entries => {
      const readings = entries.filter(({ target }) => target.isConnected).map(({ target }) => {
        const element = target as HTMLElement;
        const css = getComputedStyle(element.closest('th, td')!);
        // Keep fractional layout pixels; offsetWidth rounds down and can trigger ellipsis.
        return { key: element.dataset.tableNatural!, width: parseFloat(getComputedStyle(element).width) + parseFloat(css.paddingLeft) + parseFloat(css.paddingRight) };
      });
      flushSync(() => setNatural(before => {
        const previous = before.scope === widthScope ? before.widths : new Map<string, number>();
        const widths = growColumnWidths(previous, readings);
        return before.scope === widthScope && widths === previous ? before : { scope: widthScope, widths };
      }));
    });
    naturalCells.current = { observer, observed: new Set() };
    return () => { observer.disconnect(); naturalCells.current = null; };
  }, [widthScope]);
  useLayoutEffect(() => {
    const { observer, observed } = naturalCells.current!;
    for (const element of observed) if (!element.isConnected) { observer.unobserve(element); observed.delete(element); }
    for (const element of scroll.current?.querySelectorAll('[data-table-natural]') ?? []) {
      if (!observed.has(element)) { observed.add(element); observer.observe(element); }
    }
  }, [columns, visible, widthScope]);
  const measuredRows = useRef<{ observer: ResizeObserver; observed: Set<Element> } | null>(null);
  // Measure only mounted rows. Prefix geometry uses the last measured height of
  // each identity; new rows use the minimum until they enter the viewport.
  useLayoutEffect(() => {
    if (!wrapping || !scroll.current) return;
    const observer = new ResizeObserver(entries => {
      const readings = entries.filter(entry => entry.target.isConnected).map(entry => {
        const element = entry.target as HTMLElement;
        return [element.dataset.tableRow!, entry.borderBoxSize[0]?.blockSize ?? element.offsetHeight] as const;
      });
      flushSync(() => setRowMeasurements(before => {
        const previous = before.scope === heightScope ? before.heights : new Map<string, number>();
        let heights: Map<string, number> | undefined;
        for (const [key, height] of readings) if (previous.get(key) !== height) {
          heights ??= new Map(previous); heights.set(key, height);
        }
        return heights ? { scope: heightScope, heights } : before;
      }));
    });
    measuredRows.current = { observer, observed: new Set() };
    return () => { observer.disconnect(); measuredRows.current = null; };
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
  const pointAt = (element: HTMLElement) => { const rect = element.getBoundingClientRect(); return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 }; };
  const contentFor = useCallback((row: T) => {
    if (!tooltip) return null;
    if (tipCache.current?.row !== row || tipCache.current.build !== tooltip) tipCache.current = { row, build: tooltip, content: tooltip(row) };
    return tipCache.current.content;
  }, [tooltip]);
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
  const grid = tableGrid(columns, rank, natural.scope === widthScope ? natural.widths : new Map());
  const naturalContent = (column: TableColumn<T>, content: ReactNode) => column.width === 'max-content'
    ? <span className="table-natural" data-table-natural={column.key}>{content}</span> : content;
  return <div ref={scroll} className={`chart-table${density === 'compact' ? ' is-compact' : ''}${wrapping ? ' is-wrapped' : ''}${measuring ? ' is-measuring' : ''}`}
    style={{ '--table-columns': grid, '--table-row-height': view.rowHeight > 0 ? `${view.rowHeight}px` : undefined,
      '--table-header-height': `${view.headerHeight || 24}px`, '--table-cell-y': rowHeight == null ? undefined : '0px', '--row-glide': `${glideMs}ms`, '--table-bar-glide': tokens.reducedMotion ? '0ms' : '300ms' } as CSSProperties}
    onWheel={() => { focusAnchor.current = false; }} onPointerDown={() => { focusAnchor.current = false; }}
    onScroll={event => { const element = event.currentTarget; const top = element.scrollTop; bottomAnchor.current = element.scrollHeight - element.clientHeight - top < 1; setView(before => before.top === top ? before : { ...before, top }); }} onPointerLeave={() => { if (activeTip.current?.origin === 'pointer') clearTip(); }}>
    <table aria-label={label} aria-rowcount={ordered.length + 1}>
      <thead><tr>{rank && <th scope="col" className="table-rank">#</th>}{columns.map(column => <th key={column.key} scope="col"
        className={`${column.numeric ? 'is-numeric ' : ''}align-${columnAlign(column)}`} aria-label={column.label} data-tooltip={column.hint}
        aria-sort={column.sortable === false ? undefined : sort.column === column.key ? sort.direction : 'none'}>
        {column.sortable === false ? naturalContent(column, column.header) : <button type="button" aria-label={column.label} onClick={() => {
          const next = nextSort(sort, column.key, column.numeric ? 'descending' : 'ascending', column.direction);
          if (setOption) setOption('tableSort', sortOptionValue(next)); else setLocalSort(next);
        }}>{naturalContent(column, <>{column.header}{(sort.column === column.key || column.width === 'max-content') && <span className="table-sort" aria-hidden="true" style={{ visibility: sort.column === column.key ? undefined : 'hidden' }}> {sort.direction === 'descending' ? '▾' : '▴'}</span>}</>)}</button>}
      </th>)}</tr></thead>
      <tbody style={{ height: offsets[count] - view.headerHeight }}>{mounted.map(row => {
        const change = ranks.get(row.key)?.delta ?? 0;
        return <tr key={row.key} data-table-row={row.key} className={`${movingRowClass(row)}${muted?.(row.item.value) ? ' is-muted' : ''}${onRowClick ? ' is-activatable' : ''}`} tabIndex={row.phase === 'leaving' ? -1 : 0}
          aria-hidden={row.phase === 'leaving' || undefined} aria-rowindex={row.slot + 2}
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
          onClick={() => onRowClick?.(row.item.value)}
          onContextMenu={event => {
            if (keyboardMenu.current) { event.preventDefault(); keyboardMenu.current = false; }
            else if (open(row.item.value, event.clientX, event.clientY)) event.preventDefault();
          }}
          onKeyDown={event => {
            if (event.target !== event.currentTarget) return;
            if (event.key === 'ContextMenu' || (event.key === 'F10' && event.shiftKey)) {
              const point = pointAt(event.currentTarget);
              if (open(row.item.value, point.x, point.y)) { event.preventDefault(); keyboardMenu.current = true; }
            } else if ((event.key === 'Enter' || event.key === ' ') && onRowClick) {
              event.preventDefault(); if (!event.repeat) onRowClick(row.item.value);
            } else if (event.key === 'Escape') clearTip();
            else if (['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) {
              event.preventDefault();
              const slot = event.key === 'Home' ? 0 : event.key === 'End' ? ordered.length - 1 : Math.max(0, Math.min(ordered.length - 1, row.slot + (event.key === 'ArrowDown' ? 1 : -1)));
              const top = scrollToShow(offsets, slot, view.top, view.height, view.headerHeight);
              bottomAnchor.current = false;
              focusAnchor.current = true;
              setFocusedKey(rowKey(ordered[slot]));
              setView(before => ({ ...before, top }));
              scroll.current?.scrollTo({ top });
              requestAnimationFrame(() => scroll.current?.querySelector<HTMLElement>(`tr[aria-rowindex="${slot + 2}"]:not([aria-hidden="true"])`)?.focus());
            }
          }} onPointerDown={() => { keyboardMenu.current = false; }}>
          {rank && <td className="table-rank">{row.item.rank}{rankMarkers && change !== 0 && <span className={`table-delta ${change > 0 ? 'is-positive' : 'is-negative'}`}>{change > 0 ? '▲' : '▼'}{Math.abs(change)}</span>}</td>}
          {columns.map(column => {
            const value = column.value(row.item.value);
            const missing = isMissing(value);
            const content = cellContent(column, row.item.value, value);
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
