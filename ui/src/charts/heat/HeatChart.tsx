import { useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { KeyboardEvent, PointerEvent } from 'react';
import { mix } from '../../shell/appearance/color';
import TimeChart from '../TimeChart';
import Legend from '../Legend';
import type { ChartLayer } from '../TimeChart';
import type { TooltipContent } from '../tooltip';
import { fitLabelWidth, ellipsise } from '../labels';
import { font, VALUE_LABEL_GAP } from '../draw';
import { toLocal } from '../surface';
import { keyTarget } from '../rowNavigation';
import type { Range } from '../view';
import { cellRect, cellRuns, heatNavigation, heatWindow, heatRowAt, marginColor, periodAt, splitStripes } from './layout';
import { drawUnknownHatch } from '../hatch';
import { drawRecordedGap } from '../draw';
import type { HeatLook, HeatPart, HeatPeriod, HeatRow } from './layout';
import { useChartTokens } from '../useChartTokens';
import './heat.css';

export interface HeatChartProps {
  label: string; rows: readonly HeatRow[]; periods: readonly HeatPeriod[];
  /** Full campaign labels, when playback changes the row inventory. */
  labelCatalogue?: readonly string[];
  extent: Range; playhead: number | null; branchStart: number;
  afterPlayhead?: boolean; legend?: readonly { id?: string; label: string; color?: string }[];
  look: HeatLook; active: boolean; revision: unknown; hoursPerSecond?: number;
  tooltip: (row: HeatRow, period: HeatPeriod | null, hour: number) => TooltipContent | null;
  onContextMenu?: (hour: number, x: number, y: number) => void;
}

/** Content-neutral heat strip on the kit's time surface. Labels, axes, zoom,
 * pan, backing pixels, keyboard hover and tooltip ownership are shared. */
export default function HeatChart(props: HeatChartProps) {
  const tokens = useChartTokens();
  const surface = useRef<HTMLDivElement>(null);
  const top = tokens.fontSize.small * 2, bottom = 8;
  const minimumRow = tokens.fontSize.small + 2;
  const [viewport, setViewport] = useState({ width: 0, height: top + bottom + minimumRow * 10, scroll: 0 });
  const [labelWidth, setLabelWidth] = useState(0);
  const [hoverRow, setHoverRow] = useState<number | null>(null);
  const labels = useMemo(() => props.labelCatalogue ?? props.rows.map(row => row.label), [props.labelCatalogue, props.rows]);
  const labelCache = useRef<{ font: string; labels: Map<string, number> }>({ font: '', labels: new Map() });
  const window = heatWindow(props.rows.length, viewport.scroll, viewport.height, minimumRow, top, bottom);
  const visible = useMemo(() => props.rows.slice(window.first, window.end), [props.rows, window.first, window.end]);
  const selected = useRef(0);
  const keyboard = useRef(false);
  const pointerY = useRef(top);
  useLayoutEffect(() => {
    const element = surface.current!;
    const measure = () => setViewport(previous => ({ ...previous, width: element.clientWidth,
      height: element.clientHeight || previous.height, scroll: element.scrollTop }));
    const observer = new ResizeObserver(measure);
    observer.observe(element); measure();
    return () => observer.disconnect();
  }, []);
  useLayoutEffect(() => {
    const context = document.createElement('canvas').getContext('2d');
    if (!context) return;
    context.font = font(tokens, tokens.fontSize.small, 500);
    if (labelCache.current.font !== context.font) labelCache.current = { font: context.font, labels: new Map() };
    const widths = labelCache.current.labels;
    for (const label of labels) if (!widths.has(label)) widths.set(label, context.measureText(label).width);
    setLabelWidth(fitLabelWidth(labels, label => widths.get(label)!, 0,
      viewport.width * 0.45, VALUE_LABEL_GAP * 2));
  }, [labels, tokens.font, tokens.fontSize.small, viewport.width]);
  const scroll = () => setViewport(previous => ({ ...previous, scroll: surface.current!.scrollTop }));
  const keyDown = (event: KeyboardEvent) => {
    if (!['ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
    if (!keyboard.current) selected.current = heatRowAt(pointerY.current, window, top, props.rows.length);
    keyboard.current = true;
    selected.current = keyTarget(event.key === 'ArrowUp' || event.key === 'ArrowDown' ? event.key : '',
      selected.current, props.rows.length, 1) ?? Math.max(0, Math.min(props.rows.length - 1, selected.current));
    setHoverRow(selected.current);
    const element = surface.current!;
    const start = selected.current * window.rowHeight, end = start + window.rowHeight;
    const available = viewport.height - top - bottom;
    if (start < element.scrollTop) element.scrollTop = start;
    else if (end > element.scrollTop + available) element.scrollTop = end - available;
    scroll();
  };
  const point = (event: PointerEvent) => {
    if (!(event.target instanceof HTMLCanvasElement)) return;
    keyboard.current = false;
    const element = event.target;
    pointerY.current = toLocal(event.clientX, event.clientY, element.getBoundingClientRect(), element.clientWidth, element.clientHeight).y;
    setHoverRow(heatRowAt(pointerY.current, window, top, props.rows.length));
  };
  const legend = useMemo(() => props.legend?.map((item, index) => ({ ...item, id: item.id ?? String(index) })) ?? [], [props.legend]);
  const navigation = useMemo(() => heatNavigation(props.periods), [props.periods]);
  const layers = useMemo<ChartLayer[]>(() => {
    const colours = new WeakMap<readonly HeatPart[], { grey: string; color: string | null }>();
    const gaps = props.periods.filter(period => period.readHour == null);
    let builtUntil = -Infinity;
    let drawnRows: ReturnType<typeof cellRuns>[] = [];
    return [{ draw(ctx, frame, tokens, state) {
      const grey = mix(tokens.surface, tokens.text, 0.2);
      let lo = 0, hi = navigation.keyHours.length;
      while (lo < hi) { const mid = (lo + hi) >>> 1; if (navigation.keyHours[mid] <= (state.playhead ?? Infinity)) lo = mid + 1; else hi = mid; }
      const read = state.playhead == null ? Infinity : navigation.keyHours[lo - 1] ?? -Infinity;
      if (read !== builtUntil) {
        drawnRows = visible.map(row => cellRuns(row, props.periods, read)); builtUntil = read;
      }
      for (const period of gaps) {
        drawRecordedGap(ctx, frame, tokens, period.firstHour, period.lastHour, state.playhead);
      }
      drawnRows.forEach((runs, index) => runs.forEach(({ period, cell }) => {
        const rect = cellRect(period, index, visible.length, { ...frame, plotTop: frame.plotTop - window.offset,
          plotHeight: visible.length * window.rowHeight }, frame.x[0], frame.x[1], state.playhead);
        if (!rect) return;
        if (cell.filtered) {
          ctx.strokeStyle = tokens.border; ctx.lineWidth = tokens.lineWidth.grid;
          ctx.strokeRect(rect.left, rect.top, rect.width, rect.height); return;
        }
        if (cell.unknown) {
          drawUnknownHatch(ctx, tokens, rect); return;
        }
        if (props.look === 'split') for (const stripe of splitStripes(cell.parts, rect.top, rect.height)) {
          ctx.fillStyle = stripe.color; ctx.fillRect(rect.left, stripe.top, rect.width, stripe.height);
        } else {
          let cached = colours.get(cell.parts);
          if (!cached || cached.grey !== grey) { cached = { grey, color: marginColor(cell.parts, grey) }; colours.set(cell.parts, cached); }
          if (cached.color) { ctx.fillStyle = cached.color; ctx.fillRect(rect.left, rect.top, rect.width, rect.height); }
        }
      }));
    } }];
  }, [visible, props.periods, props.look, navigation.keyHours, window.offset, window.rowHeight]);
  const rows = useMemo(() => ({ labels: visible.map(row => row.label), labelWidth,
    tooltip: (_row: number, hour: number) => {
      const index = keyboard.current ? selected.current : heatRowAt(pointerY.current, window, top, props.rows.length);
      const column = periodAt(props.periods, hour);
      return props.rows[index] ? props.tooltip(props.rows[index], column == null ? null : props.periods[column], hour) : null;
    } }), [visible, labelWidth, props.rows, props.periods, props.tooltip, window.first, window.offset, window.rowHeight, top, hoverRow]);
  // TimeChart owns the fixed date axis; align its row labels with the scrolled cells.
  const overlay = (ctx: CanvasRenderingContext2D, frame: Parameters<ChartLayer['draw']>[1], tokens: Parameters<ChartLayer['draw']>[2]) => {
    ctx.save(); ctx.beginPath(); ctx.rect(0, frame.plotTop, frame.plotLeft - VALUE_LABEL_GAP, frame.plotHeight); ctx.clip();
    ctx.clearRect(0, frame.plotTop, frame.plotLeft - VALUE_LABEL_GAP, frame.plotHeight);
    ctx.font = font(tokens, tokens.fontSize.small, 500); ctx.textAlign = 'right'; ctx.textBaseline = 'middle'; ctx.fillStyle = tokens.text;
    visible.forEach((row, index) => ctx.fillText(ellipsise(row.label, frame.plotLeft - VALUE_LABEL_GAP * 2,
      text => ctx.measureText(text).width), frame.plotLeft - VALUE_LABEL_GAP,
      frame.plotTop + (index + 0.5) * window.rowHeight - window.offset));
    ctx.restore();
  };
  return <div className="heat-chart"><div ref={surface} className="heat-chart-surface" onScroll={scroll}
    onKeyDownCapture={keyDown} onPointerMoveCapture={point} onPointerDownCapture={point}>
    <div className="heat-chart-spacer" style={{ height: window.total }}>
      <div className="heat-chart-viewport" style={{ height: viewport.height }}>
        <div className="heat-chart-plot" style={{ height: window.plotHeight }}><TimeChart label={props.label} extent={props.extent} playhead={props.playhead} playback="cursor" afterPlayhead={props.afterPlayhead}
          branchStart={props.branchStart} hoursPerSecond={props.hoursPerSecond} axes={{ xFrom: null, xTo: null, yMin: 0, yMax: Math.max(1, visible.length), log: false }}
          padding={{ top, bottom }} rows={rows} layers={layers} overlay={overlay} {...navigation}
          active={props.active} revision={props.revision} onContextMenu={props.onContextMenu} /></div>
      </div>
    </div>
  </div><Legend label={`${props.label} legend`} items={legend} always windowed /></div>;
}
