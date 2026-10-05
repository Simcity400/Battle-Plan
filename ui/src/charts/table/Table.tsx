import { useCallback, useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { CSSProperties, ReactNode } from 'react';
import { useContextMenu } from '../../components/ContextMenu';
import type { MenuItem } from '../../components/ContextMenu';
import type { WidgetProps } from '../../dashboard/definitions';
import { hideTooltip, refreshTooltip, showTooltip, tooltipText } from '../tooltip';
import type { TooltipContent } from '../tooltip';
import { movingRowClass, useRowMotion } from '../useRowMotion';
import { useChartTokens } from '../useChartTokens';
import { easeTau } from '../easing';
import { barFraction, barMaximum, densityRowHeight, isMissing, nextSort, rankChanges, rowOffsets, scrollToShow, shownMotion, sortRows, visibleRange } from './logic.ts';
import type { RankChange, Sort } from './logic';
import { cellContent, columnAlign, growColumnWidths, tableGrid } from './columns.ts';
import type { TableColumn } from './columns';
import { sortKey } from './options.ts';
import '../charts.css';
import './table.css';

export type { TableColumn } from './columns';
const WINDOWING_THRESHOLD = 120;
export interface TableProps<T> {
  label: string; rows: readonly T[]; columns: readonly TableColumn<T>[]; rowKey: (row: T) => string;
  sort: Sort; setOption?: WidgetProps['setOption'];
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
export default function Table<T>({ label, rows, columns, rowKey, sort: storedSort, setOption, cellBars = true,
  rank = false, rankMarkers = true, density = 'comfortable', rowHeight, onRowClick, muted, menu: rowMenu,
  tooltip, onVisibleRows, hoursPerSecond = 1 }: TableProps<T>) {
  const owner = useId();
  const tokens = useChartTokens();
  const keyboardMenu = useRef(false);
  const contextMenu = useContextMenu(`${label} actions`);
  const scroll = useRef<HTMLDivElement>(null);
  const [view, setView] = useState({ top: 0, height: 0, rowHeight: 0, headerHeight: 0 });
  const [focusedKey, setFocusedKey] = useState<string | null>(null);
  const [description, setDescription] = useState<{ key: string; text: string } | null>(null);
  const tipCache = useRef<{ row: T; build: NonNullable<TableProps<T>['tooltip']>; content: TooltipContent } | null>(null);
  const activeTip = useRef<{ key: string; origin: 'pointer' | 'keyboard'; x: number; y: number } | null>(null);
  const [localSort, setLocalSort] = useState<Sort | null>(null);
  const widthScope = JSON.stringify([density, tokens.font, columns.map(column => [column.key, column.label, column.width])]);
  const [natural, setNatural] = useState<{ scope: string; widths: ReadonlyMap<string, number> }>({ scope: widthScope, widths: new Map() });
  const requestedSort = setOption ? storedSort : localSort ?? storedSort;
  const sortedColumn = columns.find(column => column.key === requestedSort.column && column.sortable !== false)
    ?? columns.find(column => column.sortable !== false);
  const sort = { column: sortedColumn?.key ?? '', direction: requestedSort.direction };
  const ordered = useMemo(() => {
    return sortRows(rows, sortedColumn?.value ?? (() => null), sort.direction, rowKey);
  }, [rows, sortedColumn, sort.direction, rowKey]);
  const items = useMemo(() => ordered.map((item, slot) => ({ key: rowKey(item), item: { value: item, rank: slot + 1 } })), [ordered, rowKey]);
  const moving = useRowMotion(items);
  const committedWindow = useRef<{ items: typeof items; keys: ReadonlySet<string> } | null>(null);
  const [departing, setDeparting] = useState<ReadonlySet<string>>(new Set());
  const glideMs = easeTau(tokens.motion, hoursPerSecond, tokens.reducedMotion) * 2.5;
  const [ranks, setRanks] = useState<ReadonlyMap<string, RankChange>>(new Map());
  const rankSort = useRef('');
  const ordering = sortKey(sort);
  useLayoutEffect(() => {
    const reset = rankSort.current !== ordering;
    rankSort.current = ordering;
    setRanks(before => rank && rankMarkers ? rankChanges(before, items.map(item => item.key), performance.now(), 2500, reset) : new Map());
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
    const measuredHeight = densityRowHeight(base, density, compact / comfortable);
    const headerHeight = parseFloat(css.getPropertyValue('--table-header-height'));
    const measure = () => setView({ top: element.scrollTop, height: element.clientHeight,
      rowHeight: measuredHeight, headerHeight });
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, [density, rowHeight]);
  const count = moving.reduce((count, row) => tokens.reducedMotion && row.phase === 'leaving' ? count : Math.max(count, row.slot + 1), ordered.length);
  const offsets = useMemo(() => rowOffsets(count, view.rowHeight, view.headerHeight), [count, view.rowHeight, view.headerHeight]);
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
    if (before?.items !== items) setDeparting(tokens.reducedMotion ? new Set() : before?.keys ?? new Set());
  }, [items, start, end, tokens.reducedMotion]);
  useEffect(() => {
    if (!departing.size) return;
    const timer = window.setTimeout(() => setDeparting(new Set()), glideMs);
    return () => window.clearTimeout(timer);
  }, [departing, glideMs]);
  const visible = useMemo(() => ordered.filter((row, slot) => (slot >= start && slot < end) || rowKey(row) === focusedKey
    || (!tokens.reducedMotion && (departing.has(rowKey(row)) || crossing?.has(rowKey(row)) === true))),
    [ordered, start, end, departing, crossing, focusedKey, rowKey, tokens.reducedMotion]);
  useEffect(() => { onVisibleRows?.(visible); }, [visible, onVisibleRows]);
  useLayoutEffect(() => {
    const elements = [...scroll.current?.querySelectorAll<HTMLElement>('[data-table-natural]') ?? []];
    if (!elements.length) return;
    const measure = () => {
      const readings = elements.map(element => {
        const cell = element.closest('th, td')!;
        const css = getComputedStyle(cell);
        return { key: element.dataset.tableNatural!, width: element.offsetWidth + parseFloat(css.paddingLeft) + parseFloat(css.paddingRight) };
      });
      setNatural(before => {
        const previous = before.scope === widthScope ? before.widths : new Map<string, number>();
        const widths = growColumnWidths(previous, readings);
        return before.scope === widthScope && widths === previous ? before : { scope: widthScope, widths };
      });
    };
    measure();
    const observer = new ResizeObserver(measure);
    elements.forEach(element => observer.observe(element));
    return () => observer.disconnect();
  }, [columns, visible, widthScope]);
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
  return <div ref={scroll} className={`chart-table${density === 'compact' ? ' is-compact' : ''}`}
    style={{ '--table-columns': grid, '--table-row-height': view.rowHeight > 0 ? `${view.rowHeight}px` : undefined,
      '--table-cell-y': rowHeight == null ? undefined : '0px', '--row-glide': `${glideMs}ms`, '--table-bar-glide': tokens.reducedMotion ? '0ms' : '300ms' } as CSSProperties}
    onScroll={event => { const top = event.currentTarget.scrollTop; setView(before => ({ ...before, top })); }} onPointerLeave={() => { if (activeTip.current?.origin === 'pointer') clearTip(); }}>
    <table aria-label={label} aria-rowcount={ordered.length + 1}>
      <thead><tr>{rank && <th scope="col" className="table-rank">#</th>}{columns.map(column => <th key={column.key} scope="col"
        className={`${column.numeric ? 'is-numeric ' : ''}align-${columnAlign(column)}`} aria-label={column.label}
        aria-sort={column.sortable === false ? undefined : sort.column === column.key ? sort.direction : 'none'}>
        {column.sortable === false ? naturalContent(column, column.header) : <button type="button" aria-label={column.label} onClick={() => {
          const next = nextSort(sort, column.key, column.numeric ? 'descending' : 'ascending');
          if (setOption) setOption('tableSort', sortKey(next)); else setLocalSort(next);
        }}>{naturalContent(column, <>{column.header}{(sort.column === column.key || column.width === 'max-content') && <span className="table-sort" aria-hidden="true" style={{ visibility: sort.column === column.key ? undefined : 'hidden' }}> {sort.direction === 'descending' ? '▾' : '▴'}</span>}</>)}</button>}
      </th>)}</tr></thead>
      <tbody style={{ height: count * view.rowHeight }}>{mounted.map(row => {
        const change = ranks.get(row.key)?.delta ?? 0;
        return <tr key={row.key} className={`${movingRowClass(row)}${muted?.(row.item.value) ? ' is-muted' : ''}${onRowClick ? ' is-activatable' : ''}`} tabIndex={row.phase === 'leaving' ? -1 : 0}
          aria-hidden={row.phase === 'leaving' || undefined} aria-rowindex={row.slot + 2}
          aria-describedby={description?.key === row.key ? `${owner}-description` : undefined}
          style={{ transform: `translateY(${row.slot * view.rowHeight}px)` }}
          onPointerMove={event => show(row.key, row.item.value, 'pointer', event.clientX, event.clientY)}
          onFocus={event => {
            if (event.target !== event.currentTarget) return;
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
              scroll.current?.scrollTo({ top: scrollToShow(offsets, slot, view.top, view.height, view.headerHeight) });
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

export function CountryCell({ name, flag }: { name: string; flag?: string }) {
  return <span className="table-country" title={name}>{flag ? <img src={flag} alt="" /> : <i className="table-flag-missing" aria-label="Flag not in this recording" />}<span>{name}</span></span>;
}
