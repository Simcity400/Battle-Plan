import { useCallback, useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { useContextMenu } from '../../components/ContextMenu';
import type { MenuItem } from '../../components/ContextMenu';
import { fitTooltip, hideTooltip, showTooltip, tooltipStore, tooltipText } from '../tooltip.ts';
import type { TooltipContent } from '../tooltip';
import { useRowMotion, movingRowClass } from '../useRowMotion';
import { useChartTokens, useViewportHeight } from '../useChartTokens';
import { anchoredScrollTop, cardHeightsAt, cardPageTarget, hasLogPartTip, keyTarget, logDate, logItems, logOffsets, logRowColumns, MONTH_HEIGHT, newCardKeys, ROW_HEIGHT, rowId, scrollToShow, shownMotion, tipRow, updateCardHeights, visibleRange } from './logData.ts';
import type { CardMeasurements, LogColumns, LogDensity, LogItem, LogPage, LogRow, LogTip } from './logData';
import { logPageItems, logPageRange, logPageTarget, logPixelTarget, logRowOrdinals, logRowTarget, wantedLogPages } from './logPages.ts';
import type { LogRowTarget } from './logPages';
import { EMPTY_LOG_REVEALS, logReveals, logVisibleEntries } from './logReveal.ts';
import { logicalScroll, physicalScroll, scrollGeometry, scrollItemTop, wheelScrollDelta } from '../scrollGeometry.ts';
import '../charts.css';
import './log.css';

export interface LogListProps<T> {
  /** The rows, newest first (`newestFirst`). The same array while unchanged. */
  rows?: readonly LogRow<T>[];
  /** Instead of eagerly loading every row, stable server windows with their
   * counts. Loaded pages replace their placeholder with actual filtered rows. */
  pages?: readonly LogPage<T>[];
  /** Hold only the windows in view, with a row-height margin, and any pending
   * keyboard destination. The product hook owns loading, errors and retries. */
  onWantedPages?: (keys: readonly string[]) => void;
  /** What the list is, for assistive technology. */
  label: string;
  density: LogDensity;
  /** A divider before each month's first row. */
  months: boolean;
  /** New rows flash in, moved rows glide and gone rows fade out; off, rows simply
   * appear, move and go. */
  animate: boolean;
  /** When supplied, entry motion means a row first revealed during forward
   * playback. Null shows the whole campaign without live arrival motion. */
  playhead?: number | null;
  /** Cards own their entire contents, including dates; rows keep the date column. */
  layout?: 'row' | 'card';
  /** A card's estimated height in layout pixels until its drawn contents are
   * measured. Row layouts can also supply their exact height. */
  rowHeight?: (row: LogRow<T>, density: LogDensity) => number;
  /** The row's cells after its date, or the whole card. */
  renderRow: (row: LogRow<T>) => ReactNode;
  /** The grid columns after the date; per-row tracks avoid reserving space for
   * optional content such as a live meter on a completed row. */
  columns?: LogColumns<T>;
  /** The row as one sentence, for assistive technology. */
  describe: (row: LogRow<T>) => string;
  /** The row's tooltip forms, fullest first (`fitTooltip` shows the fullest that fits). */
  tooltip?: (row: LogRow<T>) => readonly TooltipContent[];
  /** Demand a detail only for the tooltip currently reached by pointer or keys. */
  onTooltip?: (row: LogRow<T> | null) => void;
  /** The row's right-click menu, such as Jump to. */
  menu?: (row: LogRow<T>) => MenuItem[];
  /** Shown when there are no rows. */
  empty?: ReactNode;
  /** The rows drawn now (in view, with a margin), for a widget that names only
   * those, such as countries and flags as recorded at each row's hour. */
  onVisible?: (rows: readonly LogRow<T>[]) => void;
  /** Preserve the first visible row while recording windows are released. */
  anchorRows?: boolean;
}

const EMPTY_ROWS: readonly LogRow<never>[] = [];
/** An unchanged viewport keeps its state, so re-placing the rows never re-renders for nothing. */
const viewAt = (top: number, height: number, width: number) => (before: { top: number; height: number; width: number }) =>
  before.top === top && before.height === height && before.width === width ? before : { top, height, width };

/** A log: HTML rows newest first, each with its date,
 * that flash in as playback reaches them and glide when their order changes
 * (`useRowMotion`). Only the rows in view are drawn. Hovering a row, or reaching it
 * with the arrow keys, shows its tooltip fitted to the window, read out for the
 * keyboard. Whoever acted last owns the tooltip (`LogTip`, as a chart's hover passes
 * between pointer and keys), and it follows what its row is about (`rowId`) across
 * the row's keys, kept current or hidden as playback changes the rows. Right-click,
 * the context-menu key or Enter opens the row's menu. */
export default function LogList<T>({ rows = EMPTY_ROWS, pages, onWantedPages, label, density, months, animate, playhead, layout = 'row', rowHeight, renderRow, columns, describe, tooltip, onTooltip, menu, empty, onVisible, anchorRows = false }: LogListProps<T>) {
  const owner = useId();
  const listId = useId();
  const viewportHeight = useViewportHeight();
  const { font, fontSize, reducedMotion } = useChartTokens();
  const cardFont = JSON.stringify([font, fontSize.small, fontSize.body, fontSize.title]);
  // A callback ref: the scroller comes and goes with the rows (an empty log has
  // none), and each one is measured and observed as it attaches.
  const [scroller, setScroller] = useState<HTMLDivElement | null>(null);
  const [view, setView] = useState({ top: 0, height: 0, width: 0 });
  const [active, setActive] = useState<string | null>(null);
  const [pending, setPending] = useState<{ pageKey: string; offset: number } | null>(null);
  const [spoken, setSpoken] = useState('');
  const tip = useRef<LogTip | null>(null);
  const cardElements = useRef(new Map<string, HTMLDivElement>());
  const cardObserver = useRef<ResizeObserver | null>(null);
  const knownCards = useRef(new Set<string>());
  const [measurements, setMeasurements] = useState<CardMeasurements>({ width: 0, density, heights: new Map() });
  const actions = useContextMenu(`${label} actions`);

  const items = useMemo(() => pages ? logPageItems(pages, months) : logItems(rows, months), [rows, pages, months]);
  const heldKeys = useMemo(() => pages ? new Set(items.map(item => item.key)) : undefined, [pages, items]);
  const motionItems = useMemo(() => items.map(item => ({ key: item.key, item })), [items]);
  const moving = useRowMotion<LogItem<T>>(motionItems);
  const measuredHeights = cardHeightsAt(measurements, view.width, density, cardFont);
  const offsets = useMemo(() => logOffsets(items, density, row =>
    (layout === 'card' ? measuredHeights.get(row.key) : undefined) ?? rowHeight?.(row, density) ?? ROW_HEIGHT[density]),
  [items, density, layout, measuredHeights, rowHeight]);
  const priorLayout = useRef<{ items: readonly LogItem<T>[]; offsets: readonly number[]; top: number } | null>(null);
  const indexById = useMemo(() => new Map(items.flatMap((item, index) => item.kind === 'row' ? [[rowId(item.row), index] as const] : [])), [items]);
  const rowIds = useMemo(() => items.flatMap(item => item.kind === 'row' ? [rowId(item.row)] : []), [items]);
  const rowSlots = useMemo(() => items.flatMap((item, index) => item.kind === 'row' ? [index] : []), [items]);
  const byId = useMemo(() => new Map(items.flatMap(item => item.kind === 'row' ? [[rowId(item.row), item.row] as const] : [])), [items]);
  const ordinals = useMemo(() => logRowOrdinals(items), [items]);
  const pendingTarget = useMemo(() => pending ? logPageTarget(items, ordinals, pending.pageKey, pending.offset) : null, [items, ordinals, pending]);
  const pagesByKey = useMemo(() => new Map(pages?.map(page => [page.key, page])), [pages]);
  const revealHistory = useRef(EMPTY_LOG_REVEALS);
  const reveals = useMemo(() => logReveals(revealHistory.current, items, playhead), [items, playhead]);
  useLayoutEffect(() => { revealHistory.current = reveals.history; }, [reveals]);
  const explicitMotion = pages !== undefined || playhead !== undefined;
  const continueMotion = animate && (playhead === undefined || (typeof playhead === 'number'
    && typeof revealHistory.current.playhead === 'number' && playhead >= revealHistory.current.playhead));
  const totalHeight = offsets[items.length];
  const geometry = useMemo(() => scrollGeometry(totalHeight, view.height), [totalHeight, view.height]);
  // Keep exact logical pixels for wheel and keyboard movement even when the
  // browser rounds a much smaller native scrollTop to its layout precision.
  const scrollPosition = useRef({ logical: 0, physical: 0 });
  const setLogicalTop = useCallback((requested: number, height = scroller?.clientHeight ?? view.height) => {
    if (!scroller) return;
    const space = scrollGeometry(totalHeight, height);
    const top = Math.max(0, Math.min(space.logicalRange, requested));
    scroller.scrollTop = physicalScroll(space, top);
    scrollPosition.current = { logical: top, physical: scroller.scrollTop };
    if (priorLayout.current) priorLayout.current.top = top;
    setView(viewAt(top, height, scroller.clientWidth));
  }, [scroller, totalHeight, view.height]);
  const readScroll = useCallback((element: HTMLDivElement) => {
    const height = element.clientHeight;
    const space = scrollGeometry(totalHeight, height);
    const top = element.scrollTop === scrollPosition.current.physical
      ? Math.min(space.logicalRange, scrollPosition.current.logical) : logicalScroll(space, element.scrollTop);
    scrollPosition.current = { logical: top, physical: element.scrollTop };
    if (priorLayout.current) priorLayout.current.top = top;
    setView(viewAt(top, height, element.clientWidth));
  }, [totalHeight]);
  useLayoutEffect(() => {
    const previous = priorLayout.current;
    const top = anchorRows && previous && (previous.items !== items || (layout === 'card' && previous.offsets !== offsets))
      ? anchoredScrollTop(previous.items, previous.offsets, items, offsets, previous.top)
      : scrollPosition.current.logical;
    setLogicalTop(top);
    priorLayout.current = { items, offsets, top: scrollPosition.current.logical };
  }, [items, offsets, anchorRows, setLogicalTop, layout]);
  const sticky = months ? MONTH_HEIGHT : 0;
  const monthStarts = useMemo(() => items.flatMap<{ top: number; label: string | null }>((item, index) => item.kind === 'month' ? [{ top: offsets[index], label: item.label }]
    : item.kind === 'page' ? [{ top: offsets[index], label: null }] : []), [items, offsets]);

  // The viewport, measured in the list's own layout pixels (the widget's Size zoom included).
  useLayoutEffect(() => {
    if (!scroller) return;
    const measure = () => setLogicalTop(scrollPosition.current.logical, scroller.clientHeight);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(scroller);
    return () => observer.disconnect();
  }, [scroller, setLogicalTop]);
  useEffect(() => {
    if (!scroller || !geometry.compressed) return;
    const wheel = (event: WheelEvent) => {
      if (event.ctrlKey || !event.deltaY) return;
      event.preventDefault();
      setLogicalTop(scrollPosition.current.logical + wheelScrollDelta(event.deltaY, event.deltaMode, ROW_HEIGHT[density], scroller.clientHeight));
    };
    scroller.addEventListener('wheel', wheel, { passive: false });
    return () => scroller.removeEventListener('wheel', wheel);
  }, [scroller, geometry.compressed, density, setLogicalTop]);

  const finishCardEntry = useCallback((event: AnimationEvent) => {
    if (event.target === event.currentTarget && event.animationName === 'log-card-in') {
      (event.currentTarget as HTMLDivElement).classList.remove('is-entering');
    }
  }, []);
  const attachCard = useCallback((key: string, node: HTMLDivElement | null) => {
    const previous = cardElements.current.get(key);
    if (previous) {
      cardObserver.current?.unobserve(previous);
      previous.removeEventListener('animationend', finishCardEntry);
      previous.removeEventListener('animationcancel', finishCardEntry);
    }
    if (node) {
      cardElements.current.set(key, node);
      cardObserver.current?.observe(node);
      node.addEventListener('animationend', finishCardEntry);
      node.addEventListener('animationcancel', finishCardEntry);
    } else cardElements.current.delete(key);
  }, [finishCardEntry]);

  // Measure layout pixels, so a widget's Size zoom never changes its virtual
  // height. Only drawn cards need measuring; unseen cards use the estimate.
  useLayoutEffect(() => {
    if (layout !== 'card') return;
    const measure = (nodes: readonly HTMLDivElement[]) => {
      if (cardObserver.current !== observer) return;
      const sizes = nodes.map(node => ({ key: node.dataset.logKey!, height: node.offsetHeight }));
      setMeasurements(previous => updateCardHeights(previous, view.width, density, sizes, cardFont, heldKeys));
    };
    const observer = new ResizeObserver(entries => measure(entries.map(entry => entry.target as HTMLDivElement)));
    cardObserver.current = observer;
    const nodes = [...cardElements.current.values()];
    for (const node of nodes) observer.observe(node);
    measure(nodes);
    return () => { observer.disconnect(); cardObserver.current = null; };
  }, [layout, view.width, density, cardFont, heldKeys]);

  useLayoutEffect(() => {
    if (layout !== 'card') return;
    if (heldKeys) for (const key of knownCards.current) if (!heldKeys.has(key)) knownCards.current.delete(key);
    const enabled = continueMotion && !reducedMotion;
    if (!enabled) for (const node of cardElements.current.values()) node.classList.remove('is-entering');
    for (const key of newCardKeys(explicitMotion ? moving.filter(row => reveals.entering.has(row.key)) : moving, knownCards.current, enabled)) {
      // Add the animation on its committed mount. The class is deliberately
      // outside React's row phase: virtualisation can remount an entering row.
      cardElements.current.get(key)?.classList.add('is-entering');
    }
    for (const item of items) knownCards.current.add(item.key);
  }, [items, layout, continueMotion, reducedMotion, explicitMotion, reveals, heldKeys]);

  const tipFor = useCallback((row: LogRow<T>) => {
    const forms = tooltip?.(row);
    return forms?.length ? fitTooltip(forms, viewportHeight) : null;
  }, [tooltip, viewportHeight]);
  const element = useCallback((id: string) => scroller?.querySelector<HTMLElement>(`[data-log-id="${CSS.escape(id)}"]`) ?? null, [scroller]);
  const clearTip = useCallback(() => { tip.current = null; hideTooltip(owner); setSpoken(''); onTooltip?.(null); }, [owner, onTooltip]);
  /** Shows the tooltip of the row `next` names for whoever owns it: beside the
   * keyboard's row (read out) or at the pointer. A row that is gone takes it away. */
  const showTip = useCallback((next: LogTip) => {
    if (next.by === 'pointer') setPending(null);
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
    onTooltip?.(row);
    tip.current = { ...next, x, y };
    showTooltip(owner, content, x, y, `${next.id}:${row.key}:${byId.size}`);
  }, [byId, tipFor, clearTip, element, tooltip, describe, owner, onTooltip]);

  // The tooltip shown, the pointer's or the keyboard's, stays current as playback changes the rows.
  useEffect(() => {
    const at = tip.current;
    if (at && tooltipStore.get()?.owner === owner) showTip(at);
  }, [showTip, owner]);
  useEffect(() => () => hideTooltip(owner), [owner]);
  useEffect(() => () => onTooltip?.(null), [onTooltip]);

  const reveal = useCallback((id: string) => {
    const index = indexById.get(id);
    if (index == null || !scroller) return;
    const top = scrollToShow(offsets, index, scrollPosition.current.logical, scroller.clientHeight, sticky);
    setLogicalTop(top);
  }, [indexById, scroller, offsets, sticky, setLogicalTop]);
  const openMenu = (id: string, x: number, y: number) => {
    const row = byId.get(id);
    const entries = row && menu?.(row);
    if (entries?.length) actions.open(x, y, entries);
  };
  // A navigation key takes the tooltip from the pointer: the keyboard's row, shown
  // whole with its tooltip beside it (placed once it is drawn) and read out.
  const focusRow = useCallback((id: string) => {
    setPending(null);
    setActive(id);
    reveal(id);
    // Leave a focused part's tooltip before showing the whole card.
    if (layout === 'card' && scroller?.contains(document.activeElement) && document.activeElement !== scroller) scroller.focus({ preventScroll: true });
    tip.current = { id, by: 'keyboard', x: 0, y: 0 };
    requestAnimationFrame(() => { if (tip.current?.id === id && tip.current.by === 'keyboard') showTip(tip.current); });
  }, [reveal, showTip, layout, scroller]);

  const focusTarget = (target: LogRowTarget<T>) => {
    if (target.row) { focusRow(rowId(target.row)); return; }
    if (target.pageKey == null) return;
    if (layout === 'card' && scroller?.contains(document.activeElement) && document.activeElement !== scroller) scroller.focus({ preventScroll: true });
    setActive(null);
    clearTip();
    setPending({ pageKey: target.pageKey, offset: target.offset });
    const error = pagesByKey.get(target.pageKey)?.error;
    setSpoken(error ? 'Couldn’t load rows.' : 'Loading rows.');
    if (!scroller) return;
    const top = offsets[target.index] + target.offset * ROW_HEIGHT[density];
    setLogicalTop(scrollToShow([top, top + ROW_HEIGHT[density]], 0, scrollPosition.current.logical, scroller.clientHeight, sticky));
  };

  useEffect(() => {
    if (!pending) return;
    if (pendingTarget?.row) focusRow(rowId(pendingTarget.row));
    else if (!pendingTarget) { setPending(null); setSpoken('No rows match in this window.'); }
    else {
      const error = pagesByKey.get(pending.pageKey)?.error;
      setSpoken(error ? 'Couldn’t load rows.' : 'Loading rows.');
    }
  }, [pending, pendingTarget, pagesByKey, focusRow]);

  const current = active && byId.has(active) ? active : null;
  const onKeyDown = (event: React.KeyboardEvent) => {
    const itemIndex = current ? indexById.get(current) : undefined;
    const index = itemIndex == null ? pendingTarget?.ordinal ?? -1 : ordinals[itemIndex];
    if (event.key === 'Escape') { setPending(null); clearTip(); return; }
    if ((event.key === 'Enter' || event.key === 'ContextMenu' || (event.key === 'F10' && event.shiftKey)) && current) {
      event.preventDefault();
      const box = element(current)?.getBoundingClientRect();
      if (box) openMenu(current, box.left + 72, box.bottom);
      return;
    }
    const page = Math.max(1, Math.floor(view.height / ROW_HEIGHT[density]) - 1);
    let target = keyTarget(event.key, Math.max(0, index), ordinals[items.length], page);
    if (pages && (layout === 'card' || rowHeight) && (event.key === 'PageDown' || event.key === 'PageUp')) {
      event.preventDefault();
      const destination = logPixelTarget(items, offsets, ordinals, Math.max(0, index), view.height, event.key === 'PageDown' ? 1 : -1);
      if (destination) focusTarget(destination);
      return;
    }
    if (!pages && layout === 'card' && (event.key === 'PageDown' || event.key === 'PageUp')) {
      target = cardPageTarget(offsets, rowSlots, Math.max(0, index), view.height, event.key === 'PageDown' ? 1 : -1);
    }
    // The first arrow press reaches the newest row.
    if (index < 0 && (event.key === 'ArrowDown' || event.key === 'ArrowUp')) target = 0;
    if (target == null) return;
    event.preventDefault();
    const destination = logRowTarget(items, ordinals, target);
    if (destination) focusTarget(destination);
  };

  const [start, end] = pages ? logPageRange(offsets, view.top, view.height, density) : visibleRange(offsets, view.top, view.height);
  const entryHistory = useRef<{ seen: ReadonlySet<string>; entering: ReadonlySet<string> }>({ seen: new Set(), entering: new Set() });
  const visibleEntries = useMemo(() => {
    // Preserve the initial-render rule of useRowMotion: already-present rows
    // do not arrive. A mounted animation then survives later staying phases.
    const arriving = new Set(moving.filter(row => row.phase === 'entering' && reveals.entering.has(row.key)).map(row => row.key));
    return logVisibleEntries(items, start, end, arriving, entryHistory.current.seen, entryHistory.current.entering, continueMotion);
  }, [items, start, end, reveals, continueMotion]);
  useLayoutEffect(() => { entryHistory.current = visibleEntries; }, [visibleEntries]);
  const drawn = useMemo(() => items.slice(start, end).flatMap(item => item.kind === 'row' ? [item.row] : []), [items, start, end]);
  useEffect(() => { onVisible?.(drawn); }, [drawn, onVisible]);
  const heldIndex = current == null ? undefined : indexById.get(current);
  const heldItem = heldIndex == null ? undefined : items[heldIndex];
  const heldPage = heldItem && heldItem.kind !== 'page' ? heldItem.pageKey : undefined;
  const wanted = useMemo(() => wantedLogPages(items, start, end, [heldPage, pending?.pageKey].filter((key): key is string => key != null)), [items, start, end, heldPage, pending?.pageKey]);
  useEffect(() => { onWantedPages?.(wanted); }, [onWantedPages, wanted]);
  useEffect(() => () => onWantedPages?.([]), [onWantedPages]);
  const count = items.length;
  const place = (slot: number) => slot < count ? offsets[slot] : offsets[count] + (slot - count) * ROW_HEIGHT[density];
  const shown = shownMotion(moving, start, end, animate && (!explicitMotion || continueMotion), moved => moved.item.kind === 'row' && rowId(moved.item.row) === current, layout)
    .filter(moved => moved.phase !== 'leaving' || (moved.item.kind !== 'page' && (moved.item.pageKey == null || pagesByKey.get(moved.item.pageKey)?.rows != null)));
  // The month of the rows at the top, held above them once its own divider has scrolled away.
  const topMonth = months && view.top > 0 ? monthStarts.filter(month => month.top <= view.top).at(-1) : undefined;
  const optionId = (id: string) => `${listId}-${encodeURIComponent(JSON.stringify(id))}`;

  if (!ordinals[items.length]) return <div className="log-empty">{empty}</div>;
  return <div className={`log${pages ? ' is-paged' : ''}${geometry.compressed ? ' is-compressed' : ''}${layout === 'card' ? ' is-card-log' : ''}${density === 'compact' ? ' is-compact' : ''}${animate ? '' : ' no-motion'}`}
    style={{ '--log-columns': typeof columns === 'string' ? columns : undefined } as React.CSSProperties}>
    {topMonth?.label && <div className="log-month is-sticky" aria-hidden="true">{topMonth.label}</div>}
    <div ref={setScroller} className="log-scroll" role="listbox" aria-label={label} tabIndex={0}
      aria-activedescendant={current ? optionId(current) : undefined}
      onScroll={event => readScroll(event.currentTarget)}
      onKeyDown={onKeyDown}
      onFocus={event => {
        if (event.target !== event.currentTarget || pending) return;
        if (rowIds.length) setActive(previous => previous && byId.has(previous) ? previous : rowIds[0]);
        else if (pages) setSpoken('Loading rows.');
      }}
      onFocusCapture={event => { if (event.target !== event.currentTarget && event.target instanceof Element && hasLogPartTip(event.target, event.currentTarget)) { setPending(null); clearTip(); } }}
      onBlur={() => { setPending(null); clearTip(); }}>
      <div className="log-body" style={{ height: geometry.extent }}>
        {shown.map(moved => {
          const item = moved.item;
          const style = { transform: `translateY(${scrollItemTop(geometry, place(moved.slot), view.top)}px)` };
          if (item.kind === 'page') {
            const margin = ROW_HEIGHT[density] * 6;
            const top = Math.max(place(moved.slot), view.top - margin);
            const bottom = Math.min(place(moved.slot) + item.page.count * ROW_HEIGHT[density], view.top + view.height + margin);
            const height = Math.max(0, bottom - top);
            const labelTop = Math.max(0, Math.min(Math.max(0, height - ROW_HEIGHT[density]), view.top - top));
            return <div key={moved.key} className="log-page" style={{ transform: `translateY(${scrollItemTop(geometry, top, view.top)}px)`, height }} aria-hidden="true">
              <span style={{ transform: `translateY(${labelTop}px)` }}>{item.page.error ? 'Couldn’t load rows.' : 'Loading rows…'}</span>
            </div>;
          }
          const motion = !animate ? 'moving-row' : explicitMotion && layout === 'row' && item.kind === 'row' && moved.phase !== 'leaving'
            ? `moving-row${visibleEntries.entering.has(moved.key) ? ' is-entering' : ''}`
            : movingRowClass(layout === 'card' && moved.phase === 'entering' ? { ...moved, phase: 'staying' } : moved);
          if (item.kind === 'month') return <div key={moved.key} className={`log-month ${motion}`} style={style} aria-hidden="true">{item.label}</div>;
          const row = item.row;
          const id = rowId(row);
          // A row fading out is no longer the row its id names: it neither answers nor is read.
          const leaving = moved.phase === 'leaving';
          const date = logDate(row.hour);
          return <div key={moved.key} ref={layout === 'card' ? node => attachCard(moved.key, node) : undefined}
            id={leaving ? undefined : optionId(id)} data-log-id={leaving ? undefined : id} data-log-key={moved.key} role="option"
            aria-selected={!leaving && id === current} aria-hidden={leaving || undefined} aria-label={describe(row)}
            className={`log-row${layout === 'card' ? ' log-card' : ''} ${motion}${!leaving && id === current ? ' is-active' : ''}`}
            style={{ ...style, ...(layout === 'row' && rowHeight ? { height: rowHeight(row, density) } : {}), '--log-columns': columns == null ? undefined : logRowColumns(columns, row) } as React.CSSProperties}
            onPointerMove={event => {
              if (leaving) return;
              if (event.target instanceof Element && hasLogPartTip(event.target, event.currentTarget)) { setPending(null); clearTip(); return; }
              showTip({ id, by: 'pointer', x: event.clientX, y: event.clientY });
            }}
            onPointerLeave={() => { if (tip.current?.by === 'pointer') clearTip(); }}
            onClick={() => { if (!leaving) { setPending(null); setActive(id); } }}
            onContextMenu={event => { event.preventDefault(); if (leaving) return; setPending(null); setActive(id); openMenu(id, event.clientX, event.clientY); }}>
            {layout === 'row' && <span className="log-when"><span>{row.by && <small>by </small>}{date.day}</span>{date.year && <small>{date.year}</small>}</span>}
            {renderRow(row)}
          </div>;
        })}
      </div>
    </div>
    <span className="sr-only" aria-live="polite">{spoken}</span>
    {actions.menu}
  </div>;
}
