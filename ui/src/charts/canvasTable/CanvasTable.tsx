import { useId, useLayoutEffect, useRef, useState } from 'react';
import type { CSSProperties } from 'react';
import { useContextMenu } from '../../components/ContextMenu';
import type { MenuItem } from '../../components/ContextMenu';
import type { PlaybackController } from '../../dashboard/playback';
import { liveLoop } from '../../live/liveLoop';
import { hideTooltip, showTooltip, tooltipText } from '../tooltip';
import type { TooltipContent } from '../tooltip';
import { easeTau } from '../easing';
import { useChartTokens } from '../useChartTokens';
import { watchCanvasSize } from '../surface';
import { cancelGeometry, invalidateGeometry } from '../geometry';
import { fontFamilies, loadedFontFamily } from '../fonts';
import { FULL_FIT, fitColumns, rankChanges } from '../table/logic';
import type { ColumnFit, RankChange, Sort } from '../table/logic';
import { sortKey } from '../table/options';
import { FlagPictures, PROBE_HTML, TableMotion, TextWidths, flagText, paintRows, readLook } from './paint';
import type { ColumnBox, FrameRow, Motion, PaintMoment, PaintRow, PaintState, TableLook } from './paint';
import '../charts.css';
import '../table/table.css';
import './canvasTable.css';

/** `text`: a plain left-aligned cell (its `width` a grid track, the name column's by default). */
export interface CanvasColumn { key: string; label: string; hint?: string; kind: 'country' | 'numeric' | 'text'; width?: string }

export interface CanvasTableProps<Data> {
  label: string;
  columns: readonly CanvasColumn[];
  rank: boolean;
  /** ▲/▼ beside a rank that changed, held `RANK_HOLD_MS` as the DOM table holds it. */
  rankMarkers?: boolean;
  density: 'comfortable' | 'compact';
  clock: PlaybackController;
  active: boolean;
  /** The moment shown now: asked every frame, so cheap, and the same object while unchanged. */
  moment: () => PaintMoment<Data> | null;
  /** Lets the source wake the painter when a moment or a label lands; returns how to stop. */
  bind: (wake: () => void) => () => void;
  /** None: the rows keep the order given and the header offers no sorting. */
  sort?: Sort;
  onSort?: (column: string) => void;
  tooltip?: (data: Data, row: PaintRow<Data>) => TooltipContent | null;
  menu?: (data: Data) => readonly MenuItem[];
  onRowClick?: (data: Data) => void;
}

/** How long a rank change's marker shows (the DOM table's hold). */
const RANK_HOLD_MS = 2500;

/** A ranked table whose rows are painted in one canvas each frame: rows glide, flash in, fade out
 * and grow their bars there, so a moving table is one layer and never lays out the page. React
 * renders the header and settings; the rows come from `moment` every frame. A hidden table
 * holds the rows for screen readers; the canvas takes the pointer and the keys. */
export default function CanvasTable<Data>(props: CanvasTableProps<Data>) {
  const { label, columns, rank, density, clock, active } = props;
  const owner = useId();
  const tokens = useChartTokens();
  const contextMenu = useContextMenu(`${label} actions`);
  const root = useRef<HTMLDivElement>(null);
  const head = useRef<HTMLTableRowElement>(null);
  const probe = useRef<HTMLTableSectionElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const spoken = useRef<HTMLTableSectionElement>(null);
  const spokenRead = useRef<{ moment: PaintMoment<Data>; key: string } | null>(null);
  const live = useRef<HTMLParagraphElement>(null);
  const latest = useRef(props);
  latest.current = props;
  const menuOpen = useRef(contextMenu.open);
  menuOpen.current = contextMenu.open;
  // Numeric columns as wide as their widest value yet, never narrower.
  const [widths, setWidths] = useState<readonly number[]>([]);
  // Columns that do not fit hide, the last first, and say their values in the row's tooltip.
  const [fit, setFit] = useState<{ key: string; fit: ColumnFit }>({ key: '', fit: FULL_FIT });
  const geometry = useRef<(() => void) | null>(null);
  // The sorted column never hides: the order it sets must stay in sight.
  const order = columns.filter(column => column.kind === 'numeric' && column.key !== props.sort?.column).map(column => column.key).reverse();
  const fitKey = JSON.stringify([order, density]);
  const held = fit.key === fitKey ? fit.fit : FULL_FIT;
  const cellOf = new Map(columns.filter(column => column.kind !== 'country').map((column, index) => [column.key, index]));
  const shownColumns = columns.filter(column => !held.hidden.includes(column.key));
  const hiddenColumns = columns.filter(column => held.hidden.includes(column.key));
  const columnsKey = JSON.stringify(shownColumns.map(column => [column.key, column.kind]));
  const geometryKey = JSON.stringify(columns.map(column => [column.key, column.kind, column.label, column.width]));
  const speechKey = JSON.stringify([rank, columns.map(column => [column.key, column.kind])]);
  const headTable = useRef<HTMLTableElement>(null);
  const fitting = useRef({ held, order, fitKey });
  const hiddenRef = useRef(hiddenColumns);
  hiddenRef.current = hiddenColumns;

  useLayoutEffect(() => {
    const element = canvas.current, scroller = root.current, header = head.current, sample = probe.current?.rows[0];
    if (!active || !element || !scroller || !header || !sample) return;
    const context = element.getContext('2d');
    if (!context) return;
    let look: TableLook;
    let styleDue = true, headerHeight = 0, availableHeight = 0;
    let boxes: ColumnBox[] = [];
    // Each numeric column's header as wide as its label needs: a column is never narrower.
    let headerNeeds: number[] = [];
    // Screen pixels per layout pixel: the widget's Size zoom scales what the page measures, not what is painted.
    const zoom = () => {
      const rect = element.getBoundingClientRect();
      return { rect, x: element.clientWidth ? rect.width / element.clientWidth : 1, y: element.clientHeight ? rect.height / element.clientHeight : 1 };
    };
    const measureColumns = () => {
      const { rect, x: scale } = zoom(), base = rect.left;
      const needs: number[] = [];
      [...header.cells].forEach((cell, index) => {
        const column = rank ? shownColumns[index - 1] : shownColumns[index];
        const at = column && cellOf.get(column.key);
        if (at !== undefined) needs[at] = Math.ceil(cell.scrollWidth);
      });
      const measured = [...header.cells].map((cell, index) => {
        const box = cell.getBoundingClientRect();
        const column = rank ? shownColumns[index - 1] : shownColumns[index];
        return { left: (box.left - base) / scale, width: box.width / scale, numeric: column?.kind === 'numeric', country: column?.kind === 'country', text: column?.kind === 'text', rank: rank && index === 0,
          cell: column ? cellOf.get(column.key) ?? -1 : -1 };
      });
      return { needs, measured };
    };
    let text = new TextWidths();
    const motion = new TableMotion();
    const flags = new FlagPictures(() => wake());
    const surface = watchCanvasSize(element, () => wake());
    let scrollTop = scroller.scrollTop, hover: string | null = null, focus: string | null = null, first = true;
    let shown: PaintMoment<Data> | null = null, grown = widths, rowCount = -1;
    let cellIndex = 0;
    const speechColumns = columns.map(column => column.kind === 'country' ? -1 : cellIndex++);
    // Rank changes against the ranks last drawn; another sort order starts afresh.
    let ranks: ReadonlyMap<string, RankChange> = new Map(), ordering = '', rankTimer = 0;
    const markRanks = (moment: PaintMoment<Data>, now: number) => {
      if (!(latest.current.rankMarkers ?? true) || !rank) { ranks = new Map(); return; }
      const order = latest.current.sort ? sortKey(latest.current.sort) : '';
      ranks = rankChanges(ranks, moment.rows.map(row => row.key), now, RANK_HOLD_MS, order !== ordering);
      ordering = order;
      let due = Infinity;
      for (const change of ranks.values()) if (change.delta) due = Math.min(due, change.until);
      clearTimeout(rankTimer);
      if (Number.isFinite(due)) rankTimer = window.setTimeout(wake, Math.max(0, due - now) + 16);
    };
    const body = element.parentElement!;
    // The scroll is as tall as the rows; the canvas covers what of them shows, under the header.
    const size = () => {
      body.style.height = `${rowCount * look.rowHeight}px`;
      element.style.top = `${headerHeight}px`;
      element.style.height = `${Math.max(0, Math.min(rowCount * look.rowHeight, availableHeight - headerHeight))}px`;
    };
    // The row painted under a point: the one drawn last (on top) that is not leaving, where it is in this frame.
    let painted: readonly FrameRow[] = [];
    let pointer: { clientX: number; clientY: number } | null = null, tipped: PaintRow<Data> | undefined;
    const rowAt = (clientY: number): PaintRow<Data> | undefined => {
      const { rect, y: scale } = zoom();
      const y = (clientY - rect.top) / scale + scrollTop;
      for (let index = painted.length - 1; index >= 0; index--) {
        const candidate = painted[index];
        if (!candidate.leaving && y >= candidate.y * look.rowHeight && y < (candidate.y + 1) * look.rowHeight) return candidate.row as PaintRow<Data>;
      }
      return undefined;
    };
    // The accessible copy holds still while playing and shows the moment playback stopped at.
    const speak = (moment: PaintMoment<Data>) => {
      const body = spoken.current;
      if (!body || (moment === spokenRead.current?.moment && speechKey === spokenRead.current.key) || clock.getSnapshot().playing) return;
      spokenRead.current = { moment, key: speechKey };
      const rows = moment.rows;
      const previous = [...body.rows], added = document.createDocumentFragment();
      for (let index = previous.length - 1; index >= rows.length; index--) previous[index].remove();
      rows.forEach((row, index) => {
        const tr = previous[index] ?? document.createElement('tr');
        const values = [...(rank ? [row.rank === undefined ? '' : String(row.rank)] : []),
          ...columns.map((column, at) => column.kind === 'country' ? `${row.country?.name ?? ''}${row.country && flagText(row.country) ? ` (${flagText(row.country)})` : ''}`
            : row.cells[speechColumns[at]]?.text ?? '')];
        const cells = [...tr.cells];
        while (cells.length > values.length) cells.pop()!.remove();
        while (cells.length < values.length) { const cell = document.createElement('td'); tr.appendChild(cell); cells.push(cell); }
        values.forEach((value, at) => { if (cells[at].textContent !== value) cells[at].textContent = value; });
        if (!previous[index]) added.appendChild(tr);
      });
      body.appendChild(added);
    };
    // Grows numeric columns to the widest text of the rows in view, as the DOM table measures the
    // rows it mounts: from text, not layout.
    const cellColumns = columns.filter(column => column.kind !== 'country');
    const fit = (rows: readonly FrameRow[]) => {
      let next: number[] | null = null;
      for (const { row } of rows) row.cells.forEach((cell, index) => {
        if (cellColumns[index]?.kind !== 'numeric') return;
        const width = Math.max(headerNeeds[index] ?? 0, Math.ceil(Math.max(look.cellMin, text.tabular(context, look.numericFont, cell.text) + look.cellPad) + 2 * look.cellX));
        if (width > (next ?? grown)[index] || (next ?? grown)[index] === undefined) (next ??= [...grown])[index] = width;
      });
      if (next) { grown = next; setWidths(next); }
    };
    const motionNow = (): Motion => {
      const snapshot = clock.getSnapshot();
      const speed = snapshot.playing ? snapshot.hoursPerSecond : undefined;
      const glide = easeTau(tokens.motion, speed, tokens.reducedMotion) * 2.5;
      const moving = speed !== undefined && !tokens.reducedMotion;
      return { glide, bar: moving ? 300 : 0, enter: tokens.reducedMotion ? 0 : 700, leave: tokens.reducedMotion ? 0 : 450 };
    };
    // Painted only when something drawn changed: a still table costs one check a frame.
    let dirty = true, moving = false, fitDue = true;
    let painting: PaintState;
    const paint = () => {
      if (!painting) return;
      const now = performance.now();
      const moment = latest.current.moment();
      if (moment && moment !== shown) {
        motion.show(moment, now, motionNow(), first);
        markRanks(moment, now);
        first = false; shown = moment; dirty = true; fitDue = true;
        if (moment.rows.length !== rowCount) { rowCount = moment.rows.length; size(); }
      }
      if (shown) speak(shown);
      if (!dirty && !moving) return;
      dirty = false;
      const { layoutWidth, layoutHeight, backing } = surface.read();
      if (!layoutWidth || !layoutHeight) return;
      if (element.width !== backing.width) element.width = backing.width;
      if (element.height !== backing.height) element.height = backing.height;
      context.setTransform(backing.scaleX, 0, 0, backing.scaleY, 0, 0);
      const frame = motion.frame(now, motionNow().enter, scrollTop / look.rowHeight - 1, (scrollTop + layoutHeight) / look.rowHeight + 1);
      painted = frame.rows;
      if (fitDue) { fitDue = false; fit(frame.rows); }
      // A still pointer follows the rows moving under it, and its tooltip the values they show.
      if (pointer) {
        const row = rowAt(pointer.clientY);
        if ((row?.key ?? null) !== hover || row !== tipped) { hover = row?.key ?? null; tipped = row; tip(row, pointer.clientX, pointer.clientY); }
      }
      painting.look = look; painting.columns = boxes; painting.widths = text; painting.width = layoutWidth; painting.height = layoutHeight;
      painting.scrollTop = scrollTop; painting.hover = hover; painting.focus = focus; painting.now = now;
      paintRows(painting, frame.rows);
      moving = frame.moving;
      // While the clock plays, its own frames paint the next one; paused, motion asks for frames itself.
      if (moving && !clock.getSnapshot().playing) loop.wake();
    };
    const loop = liveLoop(clock, paint);
    const wake = () => { dirty = true; loop.wake(); };
    const measure = () => {
      const styled = styleDue;
      const nextLook = styled ? readLook(sample, scroller) : look;
      const { needs, measured } = measureColumns();
      const height = scroller.clientHeight, headHeight = header.offsetHeight;
      const { held, order, fitKey } = fitting.current;
      const table = headTable.current!;
      // The tracks are the header row's: its overflow is what does not fit.
      const nextFit = fitColumns(held, order, table.clientWidth, Math.max(table.scrollWidth, header.scrollWidth));
      const changed = styled || !painting || height !== availableHeight || headHeight !== headerHeight
        || needs.length !== headerNeeds.length || needs.some((need, index) => need !== headerNeeds[index])
        || measured.length !== boxes.length || measured.some((box, index) => {
          const previous = boxes[index];
          return box.left !== previous.left || box.width !== previous.width;
        });
      return () => {
        styleDue = false;
        if (nextFit !== held) setFit({ key: fitKey, fit: nextFit });
        if (!changed) return;
        look = nextLook;
        painting ??= { context, look, columns: boxes, widths: text, flags, width: 0, height: 0, scrollTop, hover, focus, focusColour: tokens.accent,
          delta: key => { const change = ranks.get(key); return change && change.until > painting.now ? change.delta : 0; }, now: 0 };
        if (styled) text = new TextWidths();
        headerNeeds = needs; boxes = measured;
        headerHeight = headHeight; availableHeight = height;
        if (rowCount >= 0) size();
        fitDue = true;
        wake();
      };
    };
    const invalidate = () => invalidateGeometry(measure);
    geometry.current = invalidate;
    const stopSource = latest.current.bind(wake);
    const scrolled = () => { scrollTop = scroller.scrollTop; fitDue = true; wake(); };
    const resized = new ResizeObserver(invalidate);
    resized.observe(header);
    resized.observe(scroller);
    // A column that grows moves every column after it; the header row and scroller keep their size.
    for (const cell of header.cells) resized.observe(cell);
    const restyled = () => { styleDue = true; invalidate(); };
    const restyle = new MutationObserver(restyled);
    // A font that loads after the first measure changes every width measured with it.
    const fontsLoaded = (event: FontFaceSetLoadEvent) => {
      const families = new Set(look ? [look.font, look.numericFont, look.rankFont, look.deltaFont]
        .flatMap(font => fontFamilies(font.slice(font.indexOf('px ') + 3))) : []);
      if (!look || loadedFontFamily(event, families)) restyled();
    };
    document.fonts?.addEventListener('loadingdone', fontsLoaded);
    restyle.observe(document.documentElement, { attributes: true });
    const tip = (row: PaintRow<Data> | undefined, x: number, y: number) => {
      const base = row && latest.current.tooltip?.(row.data, row);
      const hidden = hiddenRef.current;
      const extra = row && hidden.map((column, index) => ({ label: column.label, value: row.cells[cellOf.get(column.key) ?? -1]?.text ?? '—', divider: index === 0 && !!base?.rows.length }));
      const content = !row ? null : !extra?.length ? base : base ? { ...base, rows: [...base.rows, ...extra] } : { title: row.country?.name ?? '', rows: extra };
      if (content) showTooltip(owner, content, x, y, row!.key); else hideTooltip(owner);
      return content;
    };
    const move = (event: PointerEvent) => {
      pointer = { clientX: event.clientX, clientY: event.clientY };
      const row = rowAt(event.clientY);
      if ((row?.key ?? null) !== hover) { hover = row?.key ?? null; wake(); }
      tipped = row;
      tip(row, event.clientX, event.clientY);
    };
    const leave = () => { pointer = null; tipped = undefined; if (hover !== null) { hover = null; wake(); } hideTooltip(owner); };
    const click = (event: MouseEvent) => { const row = rowAt(event.clientY); if (row) latest.current.onRowClick?.(row.data); };
    const menu = (event: MouseEvent) => {
      const row = rowAt(event.clientY), items = row && latest.current.menu?.(row.data);
      if (!items?.length) return;
      event.preventDefault(); hideTooltip(owner); menuOpen.current(event.clientX, event.clientY, items);
    };
    const keys = (event: KeyboardEvent) => {
      const rows = shown?.rows ?? [];
      if (!rows.length) return;
      const at = Math.max(0, rows.findIndex(row => row.key === focus));
      const target = event.key === 'ArrowDown' ? Math.min(rows.length - 1, focus === null ? 0 : at + 1) : event.key === 'ArrowUp' ? Math.max(0, at - 1)
        : event.key === 'Home' ? 0 : event.key === 'End' ? rows.length - 1 : null;
      const { rect: box, x: zoomX, y: zoomY } = zoom(), height = element.clientHeight;
      if (target !== null) {
        event.preventDefault();
        focus = rows[target].key;
        const top = target * look.rowHeight;
        if (top < scrollTop) scroller.scrollTop = top; else if (top + look.rowHeight > scrollTop + height) scroller.scrollTop = top + look.rowHeight - height;
        const content = tip(rows[target], box.left + 24 * zoomX, box.top + (top - scroller.scrollTop + look.rowHeight) * zoomY);
        if (live.current) live.current.textContent = content ? tooltipText(content) : rows[target].country?.name ?? '';
        wake();
        return;
      }
      const row = rows.find(item => item.key === focus);
      if (!row) return;
      if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); latest.current.onRowClick?.(row.data); }
      else if (event.key === 'Escape') hideTooltip(owner);
      else if (event.key === 'ContextMenu' || (event.key === 'F10' && event.shiftKey)) {
        const items = latest.current.menu?.(row.data);
        if (items?.length) { event.preventDefault(); menuOpen.current(box.left + 24 * zoomX, box.top + (rows.indexOf(row) * look.rowHeight - scrollTop + look.rowHeight) * zoomY, items); }
      }
    };
    const blur = () => { focus = null; hideTooltip(owner); wake(); };
    scroller.addEventListener('scroll', scrolled, { passive: true });
    element.addEventListener('pointermove', move);
    element.addEventListener('pointerleave', leave);
    element.addEventListener('click', click);
    element.addEventListener('contextmenu', menu);
    element.addEventListener('keydown', keys);
    element.addEventListener('blur', blur);
    invalidate();
    return () => {
      cancelGeometry(measure); geometry.current = null;
      loop.stop(); stopSource(); surface.disconnect(); clearTimeout(rankTimer); resized.disconnect(); restyle.disconnect(); hideTooltip(owner);
      document.fonts?.removeEventListener('loadingdone', fontsLoaded);
      scroller.removeEventListener('scroll', scrolled);
      element.removeEventListener('pointermove', move); element.removeEventListener('pointerleave', leave); element.removeEventListener('click', click);
      element.removeEventListener('contextmenu', menu); element.removeEventListener('keydown', keys); element.removeEventListener('blur', blur);
    };
  }, [active, clock, columnsKey, rank, density, tokens, props.bind]); // eslint-disable-line react-hooks/exhaustive-deps

  useLayoutEffect(() => {
    fitting.current = { held, order, fitKey };
    geometry.current?.();
  }, [geometryKey, fitKey, held, widths, props.sort?.direction]);
  const tracks = [...(rank ? ['var(--table-rank-width)'] : []), ...shownColumns.map(column => column.width ?? (column.kind === 'country'
    ? 'minmax(var(--table-country-min), 2fr)' : column.kind === 'text' ? 'minmax(var(--table-name-min), 1fr)' : `${widths[cellOf.get(column.key)!] ?? 80}px`))].join(' ');
  return <><div ref={root} className={`chart-table canvas-table${density === 'compact' ? ' is-compact' : ''}`} style={{ '--table-columns': tracks } as CSSProperties}>
    <table ref={headTable} className="canvas-table-head" aria-label={`${label} columns`}>
      <thead><tr ref={head}>
        {rank && <th scope="col" className="table-rank">#</th>}
        {shownColumns.map(column => {
          const sorted = props.sort?.column === column.key ? props.sort.direction : 'none';
          const text = <>{column.label}{props.sort && (sorted !== 'none' || column.kind === 'numeric') && <span className="table-sort" aria-hidden="true"
            style={{ visibility: sorted !== 'none' ? undefined : 'hidden' }}> {sorted === 'descending' ? '▾' : '▴'}</span>}</>;
          // Value columns size to their content, as the DOM table's natural columns do.
          const label = column.kind === 'numeric' ? <span className="table-natural" data-table-natural={column.key}>{text}</span> : text;
          const content = props.onSort ? <button type="button" aria-label={column.label} onClick={() => props.onSort!(column.key)}>{label}</button> : label;
          return <th key={column.key} scope="col" className={`${column.kind === 'numeric' ? 'is-numeric align-end' : 'align-start'}`}
            aria-sort={props.sort ? sorted : undefined} data-tooltip={column.hint}>{content}</th>;
        })}
      </tr></thead>
      <tbody ref={probe} className="canvas-table-probe" dangerouslySetInnerHTML={{ __html: PROBE_HTML }} />
    </table>
    <div className="canvas-table-body">
      <canvas ref={canvas} className="canvas-table-rows" tabIndex={0} role="img" aria-label={`${label} rows. Arrow keys read each row.`} />
    </div>
  </div>
    <table className="sr-only canvas-table-spoken" aria-label={label} style={{ '--spoken-columns': columns.length + (rank ? 1 : 0) } as CSSProperties}>
      <thead><tr>{rank && <th scope="col">Rank</th>}{columns.map(column => <th key={column.key} scope="col">{column.label}</th>)}</tr></thead>
      <tbody ref={spoken} />
    </table>
    <p ref={live} className="sr-only" aria-live="polite" />
    {contextMenu.menu}
  </>;
}
