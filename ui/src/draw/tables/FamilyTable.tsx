import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import type { PlaybackController } from '../../dashboard/playback';
import type { Live } from '../../charts/live';
import { readTokens } from '../../charts/tokens.ts';
import { measuredWidget } from '../../dashboard/widgetPerf.ts';
import type { TableBuild } from './builders';
import { attachTable, type ExactRows, type TableHostHandle } from './host.ts';
import { carriedValues, type TableReading, type TableRowModel, type TableSurfaceModel, type TableTheme, type TableValues } from './surface.ts';
import { updateTableSettings, type TableColumnDefaults } from './settings.ts';
import { layoutTable, ROW_HEIGHT, type TableColumnLayout, type TableFormat, type TableSettings } from './logic.ts';
import { columnWidthVar, DISPLAY_LABELS, TableHeader, type TableHeaderColumn } from './TableHeader.tsx';

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
  shownAt, countryColours, rank, members, empty, build, fixed, tooltipKeys, onHover, exactRows, headerSlot }: FamilyTableProps) {
  const body = useRef<HTMLDivElement>(null), host = useRef<TableHostHandle | null>(null), widget = measuredWidget() ?? undefined;
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
  const readMirror = useCallback(() => {
    const handle = host.current, range = view.current, asked = ++mirrorAsked.current;
    if (!handle || !range.count) return;
    void handle.read({ rows: range }).then(reading => { if (asked === mirrorAsked.current) paintMirror(reading && { first: range.first, rows: reading.rows ?? [] }); });
  }, [paintMirror]);
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

  const hoverAsked = useRef(0);
  const totalWidth = layoutAt(drawnWidths.current, width).totalWidth;
  const latest = useRef({ model, onHover, tooltipKeys, exactRows, totalWidth, layoutAt, width });
  latest.current = { model, onHover, tooltipKeys, exactRows, totalWidth, layoutAt, width };
  useEffect(() => {
    const node = body.current; if (!node) return;
    const handle = attachTable(node, clock, (...args) => latest.current.model(...args),
      (visible, pending) => {
        node.toggleAttribute('data-table-visible', visible); node.toggleAttribute('data-table-pending', !!pending);
        if (visible) { accessible.current?.setAttribute('aria-rowcount', String(drawnCount.current + 1)); readMirror(); }
        else { ++mirrorAsked.current; paintMirror(null); accessible.current?.setAttribute('aria-rowcount', '1'); }
      },
      (key, at) => {
        const row = key?.startsWith('row:') && at ? key.split(':')[1]! : null, asked = ++hoverAsked.current;
        if (row === null || !handle) { latest.current.onHover(null); return; }
        void handle.read({ keys: latest.current.tooltipKeys?.(row) ?? [], detail: row }).then(reading => {
          if (asked !== hoverAsked.current) return;
          if (reading) latest.current.onHover(row, at, reading); else latest.current.onHover(null);
        });
      },
      setScrollLeft, () => latest.current.totalWidth, () => latest.current.exactRows,
      (count, empty) => {
        const before = drawnCount.current;
        drawnCount.current = count;
        const table = accessible.current;
        table?.setAttribute('aria-rowcount', String(count + 1));
        if (empty) table?.setAttribute('aria-description', empty); else table?.removeAttribute('aria-description');
        if (count === 0) { ++mirrorAsked.current; if (mirrored.current?.rows.length) paintMirror(null); }
        else if (before === 0) readMirror();
      },
      widths => {
        drawnWidths.current = widths;
        for (const [key, at] of Object.entries(widths)) frame.current?.style.setProperty(columnWidthVar(key), `${at}px`);
        const next = latest.current.layoutAt(widths, latest.current.width), shape = shapeOf(next);
        latest.current.totalWidth = next.totalWidth;
        setShaped(held => held.shape === shape ? held : { widths, shape });
      });
    host.current = handle;
    return () => { handle?.close(); host.current = null; };
  }, [clock, readMirror, paintMirror]);
  useEffect(() => { host.current?.refresh(); }, [model]);
  useEffect(() => followed ? undefined : live.subscribe(() => host.current?.refresh()), [live, followed]);
  useEffect(() => sources?.subscribe(() => host.current?.refresh()), [sources]);
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
