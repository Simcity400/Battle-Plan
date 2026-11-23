import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import type { PlaybackController } from '../../dashboard/playback';
import type { Live } from '../../charts/live';
import { readTokens } from '../../charts/tokens.ts';
import { measuredWidget } from '../../dashboard/widgetPerf.ts';
import type { TableBuild } from './builders';
import { bitmapSurface, holdElement } from '../bitmapSurface.ts';
import { pipelineOf } from '../../dashboard/follow/pipeline.ts';
import { useFollowedWidget } from '../../dashboard/follow/useFollowed.ts';
import { useWidgetKey } from '../../dashboard/follow/useFollow.ts';
import { pageClock } from '../../dashboard/widgetClock.ts';
import { perfLog } from '../../shell/perf.ts';
import { auditRows } from '../views/table.ts';
import { sortRowsAtHour } from './surface.ts';
import type { AuditRow } from '../notifications';

export type ExactRows = (hour: number) => Promise<{ rows: readonly TableRowModel[]; values: TableValues | null; empty?: string } | null>;
import { carriedValues, type TableReading, type TableRowModel, type TableSurfaceModel, type TableTheme, type TableValues } from './surface.ts';
import { updateTableSettings, type TableColumnDefaults } from './settings.ts';
import { layoutTable, ROW_HEIGHT, type TableColumnLayout, type TableFormat, type TableSettings } from './logic.ts';
import { columnWidthVar, DISPLAY_LABELS, TableHeader, type TableHeaderColumn } from './TableHeader.tsx';

const TABLE_AUDIT = -1;

export interface FamilyColumn extends TableColumnDefaults {
  priority: number;
  numeric?: boolean;
  format?: TableFormat;
  text?: boolean;
  widest?: readonly string[];
  hideable?: boolean;
  hint?: string;
}

export interface FamilyTableProps {
  label: string;
  className?: string;
  clock: PlaybackController;
  offset: number;
  columns: readonly FamilyColumn[];
  rows: readonly TableRowModel[];
  settings: TableSettings;
  onSettings?: (next: TableSettings) => void;
  live: Live<TableValues>;
  reply?: unknown;
  follow?: number;
  sources?: Live<Readonly<Record<string, number>>>;
  shownAt: number;
  countryColours?: ReadonlyMap<string, string>;
  rank?: { markers: boolean };
  members?: { source: string; most: number };
  empty?: string;
  fixed?: number;
  build?: TableBuild;
  tooltipKeys?: (row: string) => readonly string[];
  tooltipColumns?: readonly string[];
  onHover: (row: string | null, at?: { x: number; y: number }, drawn?: TableReading) => void;
  exactRows?: ExactRows;
  headerSlot: HTMLElement | null;
}

const OVERSCAN = 4;
const HEADER_WIDTH = 120;
const RANK_WIDTH = 36, RANK_MARKED_WIDTH = 64;

function themeOf(countryColours?: ReadonlyMap<string, string>): TableTheme | undefined {
  if (typeof document === 'undefined') return undefined;
  const tokens = readTokens();
  return { surface: tokens.surface, text: tokens.text, muted: tokens.muted, line: tokens.border, lineSoft: tokens.grid, accent: tokens.accent,
    blue: tokens.accent, red: tokens.error, amber: tokens.warning, green: tokens.positive, grey: tokens.muted, hatch: tokens.muted,
    font: tokens.font, fontSize: { small: tokens.fontSize.small, body: tokens.fontSize.body }, lineWidth: tokens.lineWidth.grid, hatchSpacing: tokens.hatchSpacing,
    country: countryColours ? Object.fromEntries(countryColours) : undefined };
}

export default function FamilyTable({ label, className, clock, offset, columns: specs, rows, settings, onSettings, live, reply, follow, sources,
  shownAt, countryColours, rank, members, empty, build, fixed, tooltipKeys, tooltipColumns, onHover, exactRows, headerSlot }: FamilyTableProps) {
  const body = useRef<HTMLDivElement>(null), widget = measuredWidget() ?? undefined;
  const [width, setWidth] = useState(0), [scrollLeft, setScrollLeft] = useState(0);
  const [resizing, setResizing] = useState<{ key: string; width: number } | null>(null);
  const theme = useMemo(() => themeOf(countryColours), [countryColours]);
  const change = useCallback((update: (next: TableSettings) => void) => onSettings?.(updateTableSettings(settings, update)), [settings, onSettings]);
  const followed = follow !== undefined || sources !== undefined;
  const rankWidth = rank ? rank.markers ? RANK_MARKED_WIDTH : RANK_WIDTH : 0;

  const view = useRef({ first: 0, count: 0 });
  const mirrorAsked = useRef(0), mirrorBody = useRef<HTMLTableSectionElement>(null);
  const mirrored = useRef<{ first: number; rows: NonNullable<TableReading['rows']> } | null>(null);
  const mirrorShape = useRef<{ columns: readonly { key: string }[]; rank: boolean }>({ columns: [], rank: false });
  const paintMirror = useCallback((held: { first: number; rows: NonNullable<TableReading['rows']> } | null) => {
    mirrored.current = held;
    const into = mirrorBody.current; if (!into) return;
    const { columns: shown, rank: ranked } = mirrorShape.current;
    into.replaceChildren(...(held?.rows ?? []).map((row, index) => {
      const tr = document.createElement('tr');
      tr.setAttribute('aria-rowindex', String(held!.first + index + 2));
      const cell = (text: string) => { const td = document.createElement('td'); td.textContent = text; tr.append(td); };
      if (ranked) cell(String(held!.first + index + 1));
      for (const column of shown) cell(row.cells[column.key] ?? 'Not recorded');
      return tr;
    }));
  }, []);
  const rebind = useRef<() => void>(() => {});
  const readMirror = useCallback(() => { ++mirrorAsked.current; rebind.current(); }, []);
  const rowHeight = ROW_HEIGHT[settings.density];
  useLayoutEffect(() => {
    const node = body.current; if (!node) return;
    const update = () => {
      const first = Math.max(0, Math.floor(node.scrollTop / rowHeight) - OVERSCAN), count = Math.ceil(node.clientHeight / rowHeight) + OVERSCAN * 2;
      if (view.current.first === first && view.current.count === count) return;
      view.current = { first, count };
      readMirror();
    };
    const observer = new ResizeObserver(update);
    observer.observe(node); node.addEventListener('scroll', update, { passive: true }); update();
    return () => { observer.disconnect(); node.removeEventListener('scroll', update); };
  }, [rowHeight, readMirror]);
  const known = useMemo(() => new Map(specs.map(column => [column.key, column])), [specs]);
  const columns = useMemo<TableColumnLayout[]>(() => settings.order.flatMap(key => {
    const spec = known.get(key);
    if (!spec) return [];
    const own = settings.columns[key];
    return [{ key, label: spec.label, priority: spec.priority, width: own?.width ?? 0, shown: own?.show ?? true, display: own?.as ?? spec.as, tone: own?.tone,
      numeric: spec.numeric, format: spec.format, widest: spec.widest }];
  }), [settings, known]);
  const drawnWidths = useRef<Record<string, number>>({}), frame = useRef<HTMLDivElement>(null);
  const [shaped, setShaped] = useState<{ widths: Record<string, number>; shape: string }>({ widths: {}, shape: '' });
  const layoutAt = useCallback((widths: Record<string, number>, available: number) => layoutTable({ columns: columns.map(column => ({ ...column,
    width: resizing?.key === column.key ? resizing.width : widths[column.key] ?? (column.width || HEADER_WIDTH) })), available: Math.max(1, available),
    rankShown: !!rank, rankWidth }, settings.overflow), [columns, resizing, rank, rankWidth, settings.overflow]);
  const layout = useMemo(() => layoutAt(shaped.widths, width), [layoutAt, width, shaped]);
  const shapeOf = (shown: ReturnType<typeof layoutAt>) => JSON.stringify([shown.columns.map(column => column.key), shown.hidden, shown.scrollable]);
  const headerColumns = useMemo<TableHeaderColumn[]>(() => layout.columns.map(column => {
    const spec = known.get(column.key)!;
    return { ...column, text: spec.text, hideable: spec.hideable, hint: spec.hint,
      displays: spec.offered.length > 1 ? spec.offered.map(display => [display, DISPLAY_LABELS[display]] as const) : [] };
  }), [layout, known]);

  const model = useCallback((id: number, _owner: number, widthPx: number, heightPx: number): TableSurfaceModel => {
    const carried = followed ? undefined : carriedValues(rows as TableRowModel[], live.read(), tooltipKeys);
    const values = carried?.values, missingValues = carried?.missing;
    return { widget, id: String(id), follow, sources: sources?.read(), offset, date: { since: shownAt, through: shownAt }, rows: rows as TableRowModel[], columns, settings,
      ...(!followed && reply !== undefined && { reply }),
      width: widthPx, height: heightPx, scrollLeft: 0, theme, headerHeight: 0, values, missingValues,
      ...(rank && { rankShown: true, rankWidth, rankMarkers: rank.markers }), ...(members && { members }), ...(empty && { empty }), ...(build && { build }), ...(fixed !== undefined && { fixed }) };
  }, [rows, reply, shownAt, follow, sources, followed, live, settings, widget, offset, columns, theme, rank, rankWidth, members, empty, build, fixed, tooltipKeys]);

  const accessible = useRef<HTMLTableElement>(null), drawnCount = useRef(0);
  useLayoutEffect(() => {
    mirrorShape.current = { columns: layout.columns, rank: !!rank };
    paintMirror(mirrored.current);
  }, [layout.columns, rank, paintMirror]);
  useEffect(() => {
    let playing = clock.getSnapshot().playing;
    return clock.subscribe(() => { const now = clock.getSnapshot().playing; if (playing && !now) readMirror(); playing = now; });
  }, [clock, readMirror]);

  const hoverAt = useRef<{ x: number; y: number } | null>(null);
  const totalWidth = layoutAt(drawnWidths.current, width).totalWidth;
  const latest = useRef({ model, onHover, tooltipKeys, tooltipColumns, exactRows, totalWidth, layoutAt, width });
  latest.current = { model, onHover, tooltipKeys, tooltipColumns, exactRows, totalWidth, layoutAt, width };
  const widgetKey = useWidgetKey();
  const pipeline = pipelineOf(pageClock(clock));
  const pipe = useFollowedWidget(pipeline, pipeline ? `${widgetKey}|${offset}` : null, { offset, pin: fixed === undefined ? null : fixed + offset });
  const surface = useRef<{ canvas: HTMLCanvasElement; extent: HTMLDivElement; surface: ReturnType<typeof bitmapSurface> } | null>(null);
  const bind = useCallback(() => {
    const node = body.current, held = surface.current;
    if (!node || !held || !pipeline || pipe === null) return;
    const w = Math.max(1, node.clientWidth), h = Math.max(1, node.clientHeight);
    const made = latest.current.model(pipe, 0, w, h);
    made.scrollLeft = node.scrollLeft; made.scrollTop = node.scrollTop; made.headerHeight = 0;
    held.extent.style.width = `${Math.max(w, latest.current.totalWidth)}px`;
    Object.assign(held.canvas.style, { width: `${w}px`, height: `${h}px` });
    const keys = latest.current.tooltipKeys;
    const tips = keys ? Object.fromEntries(made.rows.map(row => [row.key, keys(row.key)])) : undefined;
    pipeline.present(pipe, 'table');
    pipeline.bind(pipe, [held.surface], { surfaces: [{ width: w, height: h, dpr: devicePixelRatio }], ...followed ? {} : { dated: { h: shownAt, c: shownAt } } },
      {}, { model: made, ...tips ? { tooltipKeys: tips } : {}, ...latest.current.tooltipColumns ? { tooltipColumns: latest.current.tooltipColumns } : {}, window: view.current, ...perfLog ? { audit: true } : {} });
  }, [pipeline, pipe, followed, shownAt, offset]);
  useLayoutEffect(() => {
    const node = body.current;
    if (!node || !pipeline || pipe === null) return;
    const extent = document.createElement('div');
    const drawn = pipeline.held(pipe, 'table', () => {
      const canvas = document.createElement('canvas');
      canvas.className = 'tf-canvas'; canvas.setAttribute('aria-hidden', 'true'); canvas.dataset.surface = 'table'; canvas.dataset.placement = String(pipe);
      return { canvas, surface: bitmapSurface(canvas) };
    })!;
    const canvas = drawn.canvas;
    extent.className = 'tf-extent'; extent.append(canvas); node.append(extent);
    surface.current = { canvas, extent, surface: drawn.surface };
    const rowSize = () => ROW_HEIGHT[latest.current.model(pipe, 0, 1, 1).settings.density];
    const install = () => {
      const shown = pipeline.view(pipe), frameNode = frame.current;
      holdElement(frameNode, !!shown?.dim);
      const visible = shown?.mode === 'picture';
      node.toggleAttribute('data-table-visible', visible); node.toggleAttribute('data-table-pending', !visible);
      if (!visible || !shown.fields) { paintMirror(null); latest.current.onHover(null); return; }
      const rows = shown.fields.rows as { count: number; empty?: string } | undefined;
      if (rows) {
        drawnCount.current = rows.count;
        accessible.current?.setAttribute('aria-rowcount', String(rows.count + 1));
        if (rows.empty) accessible.current?.setAttribute('aria-description', rows.empty); else accessible.current?.removeAttribute('aria-description');
        extent.style.height = `${Math.max(node.clientHeight, rowSize() * rows.count)}px`;
      }
      const widths = shown.fields.widths as Record<string, number> | undefined;
      if (widths) {
        drawnWidths.current = widths;
        for (const [key, at] of Object.entries(widths)) frameNode?.style.setProperty(columnWidthVar(key), `${at}px`);
        const next = latest.current.layoutAt(widths, latest.current.width), shape = shapeOf(next);
        latest.current.totalWidth = next.totalWidth;
        setShaped(held => held.shape === shape ? held : { widths, shape });
      }
      paintMirror((shown.fields.mirror as { first: number; rows: NonNullable<TableReading['rows']> } | undefined) ?? null);
      const tip = shown.tooltip as TableReading | null;
      const at = hoverAt.current;
      if (tip && at && typeof shown.fields.tooltip === 'object') {
        const key = (shown.fields.tooltip as { key: string }).key;
        latest.current.onHover(key.split(':')[1]!, at, tip);
      } else latest.current.onHover(null);
    };
    install();
    const stop = pipeline.onInstall(pipe, install);
    const point = (event: PointerEvent) => {
      if (!event.isTrusted) return;
      hoverAt.current = { x: event.clientX, y: event.clientY };
      const box = node.getBoundingClientRect();
      pipeline.pointer(pipe, 0, [event.clientX - box.left, event.clientY - box.top]);
    };
    const leave = () => { hoverAt.current = null; latest.current.onHover(null); pipeline.pointer(pipe, 0, null); pipeline.inspect(pipe, null); };
    const scrolled = () => { setScrollLeft(node.scrollLeft); bind(); };
    node.addEventListener('pointermove', point); node.addEventListener('pointerleave', leave); node.addEventListener('scroll', scrolled, { passive: true });
    const observer = new ResizeObserver(bind); observer.observe(node);
    bind();
    return () => {
      stop(); observer.disconnect();
      node.removeEventListener('pointermove', point); node.removeEventListener('pointerleave', leave); node.removeEventListener('scroll', scrolled);
      extent.remove(); surface.current = null; holdElement(frame.current, false);
    };
  }, [pipeline, pipe, bind, paintMirror]);
  useLayoutEffect(() => { rebind.current = bind; bind(); }, [model, bind]);
  const auditable = !!exactRows;
  const shownColumns = useRef<string[]>([]);
  shownColumns.current = layout.columns.map(column => column.key);
  useLayoutEffect(() => {
    if (!perfLog || !pipeline || pipe === null || !auditable) return;
    pipeline.audit(pipe, TABLE_AUDIT, fields => fields.audit as readonly AuditRow[] | undefined);
    const stopReader = perfLog.reader(pipe, TABLE_AUDIT, async hour => {
      const exact = await latest.current.exactRows?.(hour);
      if (!exact) return null;
      const sorted = sortRowsAtHour(exact.rows, latest.current.model(pipe, 0, 1, 1).settings, exact.values, hour);
      const rows = auditRows(sorted, shownColumns.current, exact.values, hour, exact.empty);
      return { values: new Map(rows.map(row => [row.key, row.value])), ordered: true };
    }, pipeline.generation);
    return () => { stopReader(); pipeline.audit(pipe, TABLE_AUDIT, null); };
  }, [pipeline, pipe, auditable]);
  useEffect(() => followed ? undefined : live.subscribe(bind), [live, followed, bind]);
  useLayoutEffect(() => {
    const node = body.current; if (!node) return;
    const observer = new ResizeObserver(() => setWidth(node.clientWidth));
    observer.observe(node); setWidth(node.clientWidth);
    return () => observer.disconnect();
  }, []);

  const hidden = settings.overflow === 'tooltip' ? layout.hidden : [];
  const pill = headerSlot && hidden.length > 0 && createPortal(<span className="pill" title={`Hover a row to see ${hidden.map(key => known.get(key)?.label ?? key).join(', ')}`}>
    +{hidden.length} in tooltip</span>, headerSlot);
  return <div ref={frame} className={`tf-frame${className ? ` ${className}` : ''}`} data-density={settings.density} data-overflow={settings.overflow}
    onFocus={event => { if (!event.currentTarget.contains(event.relatedTarget as Node | null)) readMirror(); }}>
    {pill}
    <div className="tf-head-clip">
      <TableHeader columns={headerColumns} settings={settings} rankWidth={rankWidth} pinned={layout.scrollable} scrollLeft={settings.overflow === 'scroll' ? scrollLeft : 0}
        onChange={change} onResizing={(key, next) => setResizing({ key, width: next })}
        onResize={(key, next) => {
          setResizing(null);
          change(draft => { draft.columns[key]!.width = next; });
        }} />
    </div>
    <div ref={body} className="tf-body" data-overflow={settings.overflow} tabIndex={0} aria-label={`${label} rows`} />
    <table ref={accessible} className="table-accessibility" aria-label={label} aria-rowcount={1}>
      <thead><tr>{rank && <th scope="col">Rank</th>}{layout.columns.map(column => <th key={column.key} scope="col">{column.label}</th>)}</tr></thead>
      <tbody ref={mirrorBody} />
    </table>
  </div>;
}
