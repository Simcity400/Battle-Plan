import { useEffect, useId, useLayoutEffect, useRef, useState } from 'react';
import type { MenuItem } from '../../components/ContextMenu';
import { useContextMenu } from '../../components/ContextMenu';
import Legend from '../Legend';
import { drawCountryFlag, countryFlagContent } from '../countryFlag';
import { font, LABEL_WEIGHT } from '../draw';
import { drawUnknownHatch } from '../hatch';
import { fitLabels } from '../labels';
import { FLAG, NAME_PAD } from '../race/draw';
import { liveKey } from '../live';
import type { Live, LiveValues } from '../live';
import { scheduler } from '../scheduler';
import type { FrameTask } from '../scheduler';
import { drawingSurface, sizeCanvas, toLocal, watchCanvasSize } from '../surface';
import { fitTooltip, hideTooltip, showTooltip, tooltipStore, tooltipText } from '../tooltip';
import type { TooltipContent } from '../tooltip';
import { useChartTokens, useViewportHeight } from '../useChartTokens';
import { MATRIX_HEADER, MATRIX_NAMES_WIDTH, matrixCell, matrixCellAt, matrixGutter, matrixKey, matrixKeyed, matrixWindow, revealItem } from './matrix';
import type { MatrixAxisItem, MatrixCell, MatrixKeyed } from './matrix';
import '../charts.css';
import './heat.css';

export type { MatrixAxisItem, MatrixCell } from './matrix';

export interface HeatMatrixProps {
  label: string;
  rows: readonly MatrixAxisItem[]; columns: readonly MatrixAxisItem[];
  cell: (row: number, column: number) => MatrixCell;
  revision: unknown;
  live?: Live<LiveValues<MatrixCell>>;
  tooltip: (row: number, column: number) => TooltipContent | null;
  axisTooltip?: (item: MatrixAxisItem, axis: 'row' | 'column') => TooltipContent | null;
  onSelect?: (row: number, column: number) => void;
  menu?: (row: number, column: number) => readonly MenuItem[];
  onVisible?: (rows: readonly string[], columns: readonly string[]) => void;
  legend?: readonly { id?: string; label: string; color?: string }[];
  active: boolean;
}

interface Geometry { gutter: number; header: number; cell: number; scrollLeft: number; scrollTop: number; width: number; height: number }
const UNKNOWN: MatrixCell = { unknown: true };

export default function HeatMatrix(props: HeatMatrixProps) {
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const owner = useId();
  const surface = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const task = useRef<FrameTask | null>(null);
  const latest = useRef({ props, tokens, viewport });
  latest.current = { props, tokens, viewport };
  const geometry = useRef<Geometry>({ gutter: 0, header: MATRIX_HEADER, cell: 0, scrollLeft: 0, scrollTop: 0, width: 0, height: 0 });
  const pointer = useRef<{ x: number; y: number; clientX: number; clientY: number } | null>(null);
  const keyed = useRef<MatrixKeyed | null>(null);
  const keyIntent = useRef(false);
  const [extent, setExtent] = useState({ width: 0, height: 0 });
  const [client, setClient] = useState({ width: 0, height: 0 });
  useLayoutEffect(() => {
    const scroller = surface.current!;
    const measure = () => setClient(before => before.width === scroller.clientWidth && before.height === scroller.clientHeight
      ? before : { width: scroller.clientWidth, height: scroller.clientHeight });
    const observer = new ResizeObserver(measure);
    observer.observe(scroller); measure();
    return () => observer.disconnect();
  }, []);
  const [spoken, setSpoken] = useState('');
  const menu = useContextMenu(`${props.label} actions`);
  const menuKey = useRef(false);
  const reported = useRef('');

  useEffect(() => {
    const element = canvas.current!, scroller = surface.current!;
    const paper = drawingSurface(element), ctx = paper.context;
    if (!ctx) return;
    const loaded = () => { if (task.current) scheduler.wake(task.current); };
    const watched = watchCanvasSize(element, loaded);
    const widths = new Map<string, number>();
    let widthFont = '';
    const frameTask: FrameTask = { frame: () => {
      const { props: p, tokens: t, viewport: room } = latest.current;
      const size = watched.read();
      const width = size.layoutWidth, height = size.layoutHeight;
      sizeCanvas(paper.canvas, size.backing.width, size.backing.height);
      ctx.setTransform(size.backing.scaleX, 0, 0, size.backing.scaleY, 0, 0);
      ctx.clearRect(0, 0, width, height);
      const labelFont = font(t, t.fontSize.small, LABEL_WEIGHT);
      ctx.font = labelFont;
      if (widthFont !== labelFont) { widths.clear(); widthFont = labelFont; }
      const names = scroller.clientWidth >= MATRIX_NAMES_WIDTH;
      let widest = 0;
      if (names) for (const row of p.rows) {
        let measured = widths.get(row.label);
        if (measured === undefined) { measured = ctx.measureText(row.label).width; widths.set(row.label, measured); }
        widest = Math.max(widest, measured);
      }
      const flags = p.rows.some(row => row.countryFlag) || p.columns.some(column => column.countryFlag);
      const gutter = matrixGutter(scroller.clientWidth, widest, names, flags);
      const cell = matrixCell(scroller.clientWidth, scroller.clientHeight, gutter, p.rows.length, p.columns.length);
      const totalWidth = gutter + cell * p.columns.length, totalHeight = MATRIX_HEADER + cell * p.rows.length;
      setExtent(before => before.width === totalWidth && before.height === totalHeight ? before : { width: totalWidth, height: totalHeight });
      const scrollLeft = scroller.scrollLeft, scrollTop = scroller.scrollTop;
      const g: Geometry = { gutter, header: MATRIX_HEADER, cell, scrollLeft, scrollTop, width, height };
      geometry.current = g;
      const rows = matrixWindow(p.rows.length, scrollTop, height - MATRIX_HEADER, cell);
      const columns = matrixWindow(p.columns.length, scrollLeft, width - gutter, cell);
      const rowIds = p.rows.slice(rows.first, rows.end).map(row => row.id), columnIds = p.columns.slice(columns.first, columns.end).map(column => column.id);
      const visibleKey = JSON.stringify([rowIds, columnIds]);
      if (reported.current !== visibleKey) {
        reported.current = visibleKey;
        queueMicrotask(() => latest.current.props.onVisible?.(rowIds, columnIds));
      }
      ctx.save(); ctx.beginPath(); ctx.rect(gutter, MATRIX_HEADER, width - gutter, height - MATRIX_HEADER); ctx.clip();
      const values = p.live?.read();
      for (let row = rows.first; row < rows.end; row++) for (let column = columns.first; column < columns.end; column++) {
        const held = values?.get(liveKey(p.rows[row].id, p.columns[column].id));
        const look = !values ? p.cell(row, column) : held === undefined ? UNKNOWN : held;
        if (!look) continue;
        const rect = { left: gutter + column * cell - scrollLeft + 1, top: MATRIX_HEADER + row * cell - scrollTop + 1, width: cell - 2, height: cell - 2 };
        if ('unknown' in look) drawUnknownHatch(ctx, t, rect);
        else { ctx.fillStyle = look.color; ctx.fillRect(rect.left, rect.top, rect.width, rect.height); }
      }
      if (keyed.current) {
        keyed.current = matrixKeyed(keyed.current, p.rows, p.columns);
        if (!keyed.current) setSpoken('');
      }
      const focus = keyed.current ?? (pointer.current ? matrixCellAt(pointer.current.x, pointer.current.y, g, p.rows.length, p.columns.length) : null);
      if (focus) {
        ctx.strokeStyle = t.accent; ctx.lineWidth = 2;
        ctx.strokeRect(gutter + focus.column * cell - scrollLeft + 1, MATRIX_HEADER + focus.row * cell - scrollTop + 1, cell - 2, cell - 2);
      }
      ctx.restore();
      ctx.save(); ctx.beginPath(); ctx.rect(0, MATRIX_HEADER, gutter, height - MATRIX_HEADER); ctx.clip();
      ctx.textAlign = 'left'; ctx.textBaseline = 'middle'; ctx.fillStyle = t.text;
      const rowNameX = (flags ? FLAG.x + FLAG.width : FLAG.x) + NAME_PAD / 2;
      const fittedRows = names ? fitLabels(p.rows.slice(rows.first, rows.end).map(item => item.label), gutter - rowNameX - NAME_PAD / 2, text => ctx.measureText(text).width) : null;
      for (let row = rows.first; row < rows.end; row++) {
        const item = p.rows[row], y = MATRIX_HEADER + (row + 0.5) * cell - scrollTop;
        if (focus?.row === row) { ctx.globalAlpha = 0.05; ctx.fillRect(0, y - cell / 2, gutter, cell); ctx.globalAlpha = 1; }
        if (item.countryFlag) drawCountryFlag(ctx, item.countryFlag, FLAG.x, y - FLAG.height / 2, FLAG.width, FLAG.height, t, loaded);
        if (fittedRows) ctx.fillText(fittedRows.get(item.label) ?? '', rowNameX, y);
      }
      ctx.restore();
      ctx.save(); ctx.beginPath(); ctx.rect(gutter, 0, width - gutter, MATRIX_HEADER); ctx.clip();
      ctx.textAlign = 'center'; ctx.textBaseline = 'middle'; ctx.fillStyle = t.muted;
      const fittedCols = fitLabels(p.columns.slice(columns.first, columns.end).filter(item => !item.countryFlag).map(item => item.label), cell - 2, text => ctx.measureText(text).width);
      for (let column = columns.first; column < columns.end; column++) {
        const item = p.columns[column], x = gutter + (column + 0.5) * cell - scrollLeft;
        if (item.countryFlag) drawCountryFlag(ctx, item.countryFlag, x - FLAG.width / 2, (MATRIX_HEADER - FLAG.height) / 2, FLAG.width, FLAG.height, t, loaded);
        else ctx.fillText(fittedCols.get(item.label) ?? '', x, MATRIX_HEADER / 2);
      }
      ctx.restore();
      ctx.strokeStyle = t.grid; ctx.lineWidth = t.lineWidth.grid;
      ctx.beginPath(); ctx.moveTo(gutter, MATRIX_HEADER + 0.5); ctx.lineTo(width, MATRIX_HEADER + 0.5); ctx.stroke();
      let content: TooltipContent | null = null;
      if (focus) content = p.tooltip(focus.row, focus.column);
      else if (pointer.current) {
        const { x, y } = pointer.current;
        const axis = x < gutter && y >= MATRIX_HEADER ? { item: p.rows[Math.floor((y - MATRIX_HEADER + scrollTop) / cell)], axis: 'row' as const }
          : y < MATRIX_HEADER && x >= gutter ? { item: p.columns[Math.floor((x - gutter + scrollLeft) / cell)], axis: 'column' as const } : null;
        if (axis?.item) content = p.axisTooltip ? p.axisTooltip(axis.item, axis.axis) : { title: axis.item.label, rows: [], countryFlag: axis.item.countryFlag };
      }
      const decoded = content ? countryFlagContent(fitTooltip([content], room, { keep: 1 }), undefined, loaded) : null;
      const fitted = decoded ? fitTooltip([decoded], room, { keep: 1 }) : null;
      const held = tooltipStore.get();
      if (fitted && (!keyed.current || keyIntent.current || held == null || held.owner === owner)) {
        const rect = element.getBoundingClientRect();
        const anchor = pointer.current ?? (focus ? {
          clientX: rect.left + (gutter + (focus.column + 0.5) * cell - scrollLeft) * rect.width / Math.max(1, width),
          clientY: rect.top + (MATRIX_HEADER + (focus.row + 0.5) * cell - scrollTop) * rect.height / Math.max(1, height) } : null);
        if (anchor) showTooltip(owner, fitted, anchor.clientX, anchor.clientY);
        if (keyed.current) setSpoken(tooltipText(fitted));
      } else if (!fitted) hideTooltip(owner);
      keyIntent.current = false;
      return false;
    } };
    task.current = frameTask;
    const remove = scheduler.add(frameTask, element);
    scheduler.setVisible(frameTask, latest.current.props.active);
    const observer = new ResizeObserver(loaded);
    observer.observe(scroller);
    return () => { remove(); watched.disconnect(); observer.disconnect(); task.current = null; hideTooltip(owner); };
  }, [owner]);
  useEffect(() => {
    if (task.current) { scheduler.setVisible(task.current, props.active); scheduler.wake(task.current); }
    if (!props.active) hideTooltip(owner);
  }, [props.rows, props.columns, props.revision, props.live, props.active, tokens, viewport, owner]);
  const wake = () => { if (task.current) scheduler.wake(task.current); };
  useEffect(() => props.live?.subscribe(() => { if (task.current) scheduler.wake(task.current); }), [props.live]);
  const at = (clientX: number, clientY: number) => {
    const element = canvas.current!, rect = element.getBoundingClientRect();
    return toLocal(clientX, clientY, rect, element.clientWidth, element.clientHeight);
  };
  const cellAt = (clientX: number, clientY: number) => {
    const point = at(clientX, clientY);
    return matrixCellAt(point.x, point.y, geometry.current, props.rows.length, props.columns.length);
  };
  const leave = () => { pointer.current = null; keyed.current = null; setSpoken(''); hideTooltip(owner); wake(); };
  const keyedCell = () => keyed.current && (keyed.current = matrixKeyed(keyed.current, props.rows, props.columns));
  const openMenu = (target: { row: number; column: number }, x: number, y: number) => {
    const items = props.menu?.(target.row, target.column);
    if (items?.length) { hideTooltip(owner); menu.open(x, y, items); return true; }
    return false;
  };
  return <div className="heat-matrix"><div ref={surface} className="heat-matrix-surface themed-scrollbar" onScroll={wake}>
    <div className="heat-matrix-spacer" style={{ width: Math.max(extent.width, client.width), height: Math.max(extent.height, client.height) }}>
      <div className="heat-matrix-viewport" style={{ width: client.width, height: client.height }}>
        <canvas ref={canvas} tabIndex={0} role="img" aria-label={props.label}
          onPointerDown={() => { menuKey.current = false; }}
          onPointerMove={event => { pointer.current = { ...at(event.clientX, event.clientY), clientX: event.clientX, clientY: event.clientY }; keyed.current = null; setSpoken(''); wake(); }}
          onPointerLeave={() => { pointer.current = null; if (!keyed.current) hideTooltip(owner); wake(); }} onBlur={leave}
          onClick={event => { const hit = cellAt(event.clientX, event.clientY); if (hit) props.onSelect?.(hit.row, hit.column); }}
          onContextMenu={event => {
            event.preventDefault();
            if (menuKey.current) { menuKey.current = false; return; }
            const hit = keyedCell() ?? cellAt(event.clientX, event.clientY);
            if (hit) openMenu(hit, event.clientX, event.clientY);
          }}
          onKeyDown={event => {
            if (event.key === 'Escape') { event.preventDefault(); leave(); return; }
            const g = geometry.current, scroller = surface.current!;
            const current = keyedCell();
            if ((event.key === 'Enter' || event.key === ' ') && current) { event.preventDefault(); if (!event.repeat) props.onSelect?.(current.row, current.column); return; }
            if ((event.key === 'ContextMenu' || event.key === 'F10' && event.shiftKey) && current) {
              const rect = canvas.current!.getBoundingClientRect();
              if (openMenu(current, rect.left + (g.gutter + (current.column + 0.5) * g.cell - g.scrollLeft) * rect.width / Math.max(1, g.width),
                rect.top + (g.header + (current.row + 0.5) * g.cell - g.scrollTop) * rect.height / Math.max(1, g.height))) { event.preventDefault(); menuKey.current = true; }
              return;
            }
            const page = Math.max(1, Math.floor((g.height - g.header) / Math.max(1, g.cell)));
            const next = matrixKey(event.key, current ?? { row: 0, column: 0 }, props.rows.length, props.columns.length, current ? page : 0);
            if (!next) return;
            event.preventDefault();
            keyed.current = { ...next, rowId: props.rows[next.row].id, columnId: props.columns[next.column].id }; pointer.current = null; keyIntent.current = true;
            scroller.scrollTop = revealItem(next.row, g.cell, scroller.scrollTop, g.height - g.header);
            scroller.scrollLeft = revealItem(next.column, g.cell, scroller.scrollLeft, g.width - g.gutter);
            wake();
          }} />
      </div>
    </div>
  </div>
    {props.legend?.length ? <Legend label={`${props.label} legend`} items={props.legend.map((item, index) => ({ ...item, id: item.id ?? String(index) }))} always /> : null}
    <p className="sr-only" aria-live="polite">{spoken}</p>
    {menu.menu}
  </div>;
}
