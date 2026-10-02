import { useCallback, useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { useContextMenu } from '../../components/ContextMenu';
import type { MenuItem } from '../../components/ContextMenu';
import { fitTooltip, hideTooltip, showTooltip, tooltipStore, tooltipText } from '../tooltip.ts';
import type { TooltipContent } from '../tooltip';
import { useRowMotion, movingRowClass } from '../useRowMotion';
import { useViewportHeight } from '../useChartTokens';
import { keyTarget, logDate, logItems, logOffsets, MONTH_HEIGHT, ROW_HEIGHT, rowId, scrollToShow, shownMotion, tipRow, visibleRange } from './logData.ts';
import type { LogDensity, LogItem, LogRow, LogTip } from './logData';
import '../charts.css';
import './log.css';

export interface LogListProps<T> {
  /** The rows, newest first (`newestFirst`). The same array while unchanged. */
  rows: readonly LogRow<T>[];
  /** What the list is, for assistive technology. */
  label: string;
  density: LogDensity;
  /** A divider before each month's first row. */
  months: boolean;
  /** New rows flash in, moved rows glide and gone rows fade out; off, rows simply
   * appear, move and go. */
  animate: boolean;
  /** The row's cells after its date, laid out by `columns`. */
  renderRow: (row: LogRow<T>) => ReactNode;
  /** The grid columns of those cells (CSS `grid-template-columns`). */
  columns: string;
  /** The row as one sentence, for assistive technology. */
  describe: (row: LogRow<T>) => string;
  /** The row's tooltip forms, fullest first (`fitTooltip` shows the fullest that fits). */
  tooltip?: (row: LogRow<T>) => readonly TooltipContent[];
  /** The row's right-click menu, such as Jump to. */
  menu?: (row: LogRow<T>) => MenuItem[];
  /** Shown when there are no rows. */
  empty?: ReactNode;
  /** The rows drawn now (in view, with a margin), for a widget that names only
   * those, such as countries and flags as recorded at each row's hour. */
  onVisible?: (rows: readonly LogRow<T>[]) => void;
}

/** A log (spec 2.2d, mock-up style-5): HTML rows newest first, each with its date,
 * that flash in as playback reaches them and glide when their order changes
 * (`useRowMotion`). Only the rows in view are drawn. Hovering a row, or reaching it
 * with the arrow keys, shows its tooltip fitted to the window, read out for the
 * keyboard. Whoever acted last owns the tooltip (`LogTip`, as a chart's hover passes
 * between pointer and keys), and it follows what its row is about (`rowId`) across
 * the row's keys, kept current or hidden as playback changes the rows. Right-click,
 * the context-menu key or Enter opens the row's menu. */
export default function LogList<T>({ rows, label, density, months, animate, renderRow, columns, describe, tooltip, menu, empty, onVisible }: LogListProps<T>) {
  const owner = useId();
  const listId = useId();
  const viewportHeight = useViewportHeight();
  // A callback ref: the scroller comes and goes with the rows (an empty log has
  // none), and each one is measured and observed as it attaches.
  const [scroller, setScroller] = useState<HTMLDivElement | null>(null);
  const [view, setView] = useState({ top: 0, height: 0 });
  const [active, setActive] = useState<string | null>(null);
  const [spoken, setSpoken] = useState('');
  const tip = useRef<LogTip | null>(null);
  const actions = useContextMenu(`${label} actions`);

  const items = useMemo(() => logItems(rows, months), [rows, months]);
  const motionItems = useMemo(() => items.map(item => ({ key: item.key, item })), [items]);
  const moving = useRowMotion<LogItem<T>>(motionItems);
  const offsets = useMemo(() => logOffsets(items, density), [items, density]);
  const indexById = useMemo(() => new Map(items.flatMap((item, index) => item.kind === 'row' ? [[rowId(item.row), index] as const] : [])), [items]);
  const rowIds = useMemo(() => items.flatMap(item => item.kind === 'row' ? [rowId(item.row)] : []), [items]);
  const byId = useMemo(() => new Map(rows.map(row => [rowId(row), row])), [rows]);
  const sticky = months ? MONTH_HEIGHT : 0;
  const monthStarts = useMemo(() => items.flatMap((item, index) => item.kind === 'month' ? [{ top: offsets[index], label: item.label }] : []), [items, offsets]);

  // The viewport, measured in the list's own layout pixels (the widget's Size zoom included).
  useLayoutEffect(() => {
    if (!scroller) return;
    const measure = () => setView({ top: scroller.scrollTop, height: scroller.clientHeight });
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(scroller);
    return () => observer.disconnect();
  }, [scroller]);

  const tipFor = useCallback((row: LogRow<T>) => {
    const forms = tooltip?.(row);
    return forms?.length ? fitTooltip(forms, viewportHeight) : null;
  }, [tooltip, viewportHeight]);
  const element = useCallback((id: string) => scroller?.querySelector<HTMLElement>(`[data-log-id="${CSS.escape(id)}"]`) ?? null, [scroller]);
  const clearTip = useCallback(() => { tip.current = null; hideTooltip(owner); setSpoken(''); }, [owner]);
  /** Shows the tooltip of the row `next` names for whoever owns it: beside the
   * keyboard's row (read out) or at the pointer. A row that is gone takes it away. */
  const showTip = useCallback((next: LogTip) => {
    const row = tipRow(next, byId);
    const content = row && tipFor(row);
    if (!row || !content) { clearTip(); return; }
    let { x, y } = next;
    if (next.by === 'keyboard') {
      const box = element(next.id)?.getBoundingClientRect();
      if (box) { x = box.right - 24; y = box.top + box.height / 2; }
      const forms = tooltip?.(row);
      setSpoken(forms?.length ? tooltipText(forms[0]) : describe(row));
    }
    tip.current = { ...next, x, y };
    showTooltip(owner, content, x, y, `${next.id}:${row.key}:${rows.length}`);
  }, [byId, tipFor, clearTip, element, tooltip, describe, owner, rows.length]);

  // The tooltip shown, the pointer's or the keyboard's, stays current as playback changes the rows.
  useEffect(() => {
    const at = tip.current;
    if (at && tooltipStore.get()?.owner === owner) showTip(at);
  }, [showTip, owner]);
  useEffect(() => () => hideTooltip(owner), [owner]);

  const reveal = (id: string) => {
    const index = indexById.get(id);
    if (index == null || !scroller) return;
    const top = scrollToShow(offsets, index, scroller.scrollTop, scroller.clientHeight, sticky);
    if (top !== scroller.scrollTop) scroller.scrollTop = top;
  };
  const openMenu = (id: string, x: number, y: number) => {
    const row = byId.get(id);
    const entries = row && menu?.(row);
    if (entries?.length) actions.open(x, y, entries);
  };
  // A navigation key takes the tooltip from the pointer: the keyboard's row, shown
  // whole with its tooltip beside it (placed once it is drawn) and read out.
  const focusRow = (id: string) => {
    setActive(id);
    reveal(id);
    tip.current = { id, by: 'keyboard', x: 0, y: 0 };
    requestAnimationFrame(() => { if (tip.current?.id === id && tip.current.by === 'keyboard') showTip(tip.current); });
  };

  const current = active && byId.has(active) ? active : null;
  const onKeyDown = (event: React.KeyboardEvent) => {
    const index = current ? rowIds.indexOf(current) : -1;
    if (event.key === 'Escape') { clearTip(); return; }
    if ((event.key === 'Enter' || event.key === 'ContextMenu' || (event.key === 'F10' && event.shiftKey)) && current) {
      event.preventDefault();
      const box = element(current)?.getBoundingClientRect();
      if (box) openMenu(current, box.left + 72, box.bottom);
      return;
    }
    const page = Math.max(1, Math.floor(view.height / ROW_HEIGHT[density]) - 1);
    let target = keyTarget(event.key, Math.max(0, index), rowIds.length, page);
    // The first arrow press reaches the newest row.
    if (index < 0 && (event.key === 'ArrowDown' || event.key === 'ArrowUp')) target = 0;
    if (target == null) return;
    event.preventDefault();
    focusRow(rowIds[target]);
  };

  const [start, end] = visibleRange(offsets, view.top, view.height);
  const drawn = useMemo(() => items.slice(start, end).flatMap(item => item.kind === 'row' ? [item.row] : []), [items, start, end]);
  useEffect(() => { onVisible?.(drawn); }, [drawn, onVisible]);
  const count = items.length;
  const place = (slot: number) => slot < count ? offsets[slot] : offsets[count] + (slot - count) * ROW_HEIGHT[density];
  const shown = shownMotion(moving, start, end, animate, moved => moved.item.kind === 'row' && rowId(moved.item.row) === current);
  // The month of the rows at the top, held above them once its own divider has scrolled away.
  const topMonth = months && view.top > 0 ? monthStarts.filter(month => month.top <= view.top).at(-1) : undefined;
  const optionId = (id: string) => `${listId}-${id.replace(/[^\w-]/g, '_')}`;

  if (!rows.length) return <div className="log-empty">{empty}</div>;
  return <div className={`log${density === 'compact' ? ' is-compact' : ''}${animate ? '' : ' no-motion'}`}
    style={{ '--log-columns': columns } as React.CSSProperties}>
    {topMonth && <div className="log-month is-sticky" aria-hidden="true">{topMonth.label}</div>}
    <div ref={setScroller} className="log-scroll" role="listbox" aria-label={label} tabIndex={0}
      aria-activedescendant={current ? optionId(current) : undefined}
      onScroll={event => setView({ top: event.currentTarget.scrollTop, height: event.currentTarget.clientHeight })}
      onKeyDown={onKeyDown}
      onFocus={event => { if (event.target === event.currentTarget && !current && rowIds.length) setActive(rowIds[0]); }}
      onBlur={clearTip}>
      <div className="log-body" style={{ height: offsets[count] }}>
        {shown.map(moved => {
          const item = moved.item;
          const style = { transform: `translateY(${place(moved.slot)}px)` };
          const motion = animate ? movingRowClass(moved) : 'moving-row';
          if (item.kind === 'month') return <div key={moved.key} className={`log-month ${motion}`} style={style} aria-hidden="true">{item.label}</div>;
          const row = item.row;
          const id = rowId(row);
          // A row fading out is no longer the row its id names: it neither answers nor is read.
          const leaving = moved.phase === 'leaving';
          const date = logDate(row.hour);
          return <div key={moved.key} id={leaving ? undefined : optionId(id)} data-log-id={leaving ? undefined : id} role="option"
            aria-selected={!leaving && id === current} aria-hidden={leaving || undefined} aria-label={describe(row)}
            className={`log-row ${motion}${!leaving && id === current ? ' is-active' : ''}`} style={style}
            onPointerMove={event => { if (!leaving) showTip({ id, by: 'pointer', x: event.clientX, y: event.clientY }); }}
            onPointerLeave={() => { if (tip.current?.by === 'pointer') { tip.current = null; hideTooltip(owner); } }}
            onClick={() => { if (!leaving) setActive(id); }}
            onContextMenu={event => { event.preventDefault(); if (leaving) return; setActive(id); openMenu(id, event.clientX, event.clientY); }}>
            <span className="log-when"><span>{row.by && <small>by </small>}{date.day}</span>{date.year && <small>{date.year}</small>}</span>
            {renderRow(row)}
          </div>;
        })}
      </div>
    </div>
    <span className="sr-only" aria-live="polite">{spoken}</span>
    {actions.menu}
  </div>;
}
