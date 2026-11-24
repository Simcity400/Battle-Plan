import { useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { KeyboardEvent, PointerEvent } from 'react';
import { mix } from '../../shell/appearance/color';
import TimeChart from '../TimeChart';
import Legend from '../Legend';
import type { ChartLayer, LiveHead, TimeChartProps } from '../TimeChart';
import type { Live } from '../live';
import type { TooltipContent } from '../tooltip';
import { fitLabels, fitLabelWidth } from '../labels';
import { font, VALUE_LABEL_GAP } from '../draw';
import { toLocal } from '../surface';
import { keyTarget } from '../rowNavigation';
import type { Range } from '../view';
import { cellRect, cellRuns, heatNavigation, heatWindow, heatRowAt, joinedCellPeriod, marginColor, periodAt, splitStripes } from './layout';
import { drawUnknownHatch } from '../hatch';
import { drawRecordedGap } from '../draw';
import type { HeatLook, HeatPart, HeatPeriod, HeatRow, HeatTail } from './layout';
import { cancelGeometry, invalidateGeometry } from '../geometry';
import { fontFamilies, loadedFontFamily } from '../fonts';
import { useChartTokens } from '../useChartTokens';
import { fillPendingBand, markApproximate, spanRanges } from '../prepared';
import type { Span } from '../prepared';
import { overlayRuns } from '../series.ts';
import './heat.css';

export interface HeatChartProps {
  label: string; rows: readonly HeatRow[]; periods: readonly HeatPeriod[];
  tail?: HeatTail;
  liveTail?: Live<HeatTail | null>;
  labelCatalogue?: readonly string[];
  extent: Range; playhead: number | null; branchStart: number;
  liveHead?: LiveHead;
  afterPlayhead?: boolean; legend?: readonly { id?: string; label: string; color?: string }[];
  look: HeatLook; active: boolean; revision: unknown; hoursPerSecond?: number;
  loading?: readonly Span[];
  tooltip: (row: HeatRow, period: HeatPeriod | null, hour: number) => TooltipContent | null;
  onContextMenu?: (hour: number, x: number, y: number) => void;
  onPick?: (row: HeatRow, period: HeatPeriod | null, hour: number) => void;
  pickOwner?: unknown;
  onSummaryView?: TimeChartProps['onSummaryView'];
}

export default function HeatChart(props: HeatChartProps) {
  const tokens = useChartTokens();
  const surface = useRef<HTMLDivElement>(null);
  const top = tokens.fontSize.small * 2, bottom = 8;
  const minimumRow = tokens.fontSize.small + 2;
  const [viewport, setViewport] = useState({ width: 0, height: top + bottom + minimumRow * 10, scroll: 0 });
  const [labelWidth, setLabelWidth] = useState(0);
  const [hoverRow, setHoverRow] = useState<number | null>(null);
  const labels = useMemo(() => props.labelCatalogue ?? props.rows.map(row => row.label), [props.labelCatalogue, props.rows]);
  const labelCache = useRef<{ font: string; room: number; key: string; labels: Map<string, number>; fitted: Map<string, string> }>(
    { font: '', room: -1, key: '', labels: new Map(), fitted: new Map() });
  const window = heatWindow(props.rows.length, viewport.scroll, viewport.height, minimumRow, top, bottom);
  const visible = useMemo(() => props.rows.slice(window.first, window.end), [props.rows, window.first, window.end]);
  const history = props.periods;
  const periods = useMemo(() => props.tail ? [...history, props.tail.period] : props.periods, [history, props.periods, props.tail]);
  const historyRuns = useRef<{ rows: readonly HeatRow[]; periods: readonly HeatPeriod[]; read: number; runs: ReturnType<typeof cellRuns>[] } | null>(null);
  const selected = useRef(0);
  const keyboard = useRef(false);
  const pointerY = useRef(top);
  useLayoutEffect(() => {
    const element = surface.current!;
    const measure = () => {
      const width = element.clientWidth, measured = element.clientHeight, scrolled = element.scrollTop;
      setViewport(previous => {
        const height = measured || previous.height;
        return previous.width === width && previous.height === height && previous.scroll === scrolled ? previous : { width, height, scroll: scrolled };
      });
    };
    const observer = new ResizeObserver(measure);
    observer.observe(element); measure();
    return () => observer.disconnect();
  }, []);
  useLayoutEffect(() => {
    const context = document.createElement('canvas').getContext('2d');
    if (!context) return;
    context.font = font(tokens, tokens.fontSize.small, 500);
    if (labelCache.current.font !== context.font) labelCache.current = { font: context.font, room: -1, key: '', labels: new Map(), fitted: new Map() };
    const measure = () => {
      if (labelCache.current.font !== context.font) labelCache.current = { font: context.font, room: -1, key: '', labels: new Map(), fitted: new Map() };
      const widths = labelCache.current.labels;
      const width = fitLabelWidth(labels, label => {
        let width = widths.get(label);
        if (width === undefined) { width = context.measureText(label).width; widths.set(label, width); }
        return width;
      }, 0, viewport.width * 0.45, VALUE_LABEL_GAP * 2);
      return () => setLabelWidth(width);
    };
    const families = new Set(fontFamilies(tokens.font));
    const loaded = (event: FontFaceSetLoadEvent) => {
      if (!loadedFontFamily(event, families)) return;
      labelCache.current.labels.clear(); labelCache.current.fitted.clear(); labelCache.current.room = -1; labelCache.current.key = ''; invalidateGeometry(measure);
      setViewport(previous => ({ ...previous }));
    };
    document.fonts?.addEventListener('loadingdone', loaded);
    invalidateGeometry(measure);
    return () => { cancelGeometry(measure); document.fonts?.removeEventListener('loadingdone', loaded); };
  }, [labels, tokens.font, tokens.fontSize.small, viewport.width]);
  const scroll = () => {
    const scrolled = surface.current!.scrollTop;
    setViewport(previous => previous.scroll === scrolled ? previous : { ...previous, scroll: scrolled });
  };
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
  const navigation = useMemo(() => heatNavigation(periods), [periods]);
  const historyLast = useMemo(() => heatNavigation(history).keyHours.at(-1) ?? -Infinity, [history]);
  const colours = useRef(new WeakMap<readonly HeatPart[], { grey: string; color: string | null }>());
  const layers = useMemo<ChartLayer[]>(() => {
    const gaps = overlayRuns(periods.filter(period => period.readHour == null), [], () => false, []);
    return [{ draw(ctx, frame, tokens, state) {
      const grey = mix(tokens.surface, tokens.text, 0.2);
      let lo = 0, hi = navigation.keyHours.length;
      while (lo < hi) { const mid = (lo + hi) >>> 1; if (navigation.keyHours[mid] <= (state.playhead ?? Infinity)) lo = mid + 1; else hi = mid; }
      const read = state.playhead == null ? Infinity : navigation.keyHours[lo - 1] ?? -Infinity;
      const historyRead = Math.min(read, historyLast);
      let held = historyRuns.current;
      if (!held || held.rows !== visible || held.periods !== history || held.read !== historyRead)
        historyRuns.current = held = { rows: visible, periods: history, read: historyRead, runs: visible.map(row => cellRuns(row, history, historyRead)) };
      const drawnRows = held.runs;
      const cellsFrame = { ...frame, plotTop: frame.plotTop - window.offset, plotHeight: visible.length * window.rowHeight };
      for (const period of gaps) {
        drawRecordedGap(ctx, frame, tokens, period.firstHour, period.lastHour, state.playhead);
      }
      for (const [from, to] of spanRanges(props.loading, frame.x[0], frame.x[1], state.playhead))
        fillPendingBand(ctx, tokens, frame.toX(from), frame.toX(to), frame.plotTop, frame.plotHeight);
      const live = props.liveTail?.read() ?? null;
      drawnRows.forEach((runs, index) => {
        const drawCell = ({ period, cell }: ReturnType<typeof cellRuns>[number]) => {
          const rect = cellRect(period, index, visible.length, cellsFrame, frame.x[0], frame.x[1], state.playhead);
          if (!rect) return;
          if (cell.filtered) {
            ctx.strokeStyle = tokens.border; ctx.lineWidth = tokens.lineWidth.grid;
            ctx.strokeRect(rect.left, rect.top, rect.width, rect.height);
          } else if (cell.unknown) {
            drawUnknownHatch(ctx, tokens, rect);
          } else if (props.look === 'split') for (const stripe of splitStripes(cell.parts, rect.top, rect.height)) {
            ctx.fillStyle = stripe.color; ctx.fillRect(rect.left, stripe.top, rect.width, stripe.height);
          } else {
            let cached = colours.current.get(cell.parts);
            if (!cached || cached.grey !== grey) { cached = { grey, color: marginColor(cell.parts, grey) }; colours.current.set(cell.parts, cached); }
            if (cached.color) { ctx.fillStyle = cached.color; ctx.fillRect(rect.left, rect.top, rect.width, rect.height); }
          }
          if (cell.approximate) markApproximate(ctx, tokens, rect.left, rect.top, rect.width, rect.height);
        };
        const tail = props.tail, cell = tail?.cells[window.first + index];
        const shown = tail && cell && tail.period.readHour != null && tail.period.readHour <= read;
        const last = runs.at(-1);
        const joined = shown && last && joinedCellPeriod(last.period, tail.period, last.cell, cell);
        runs.forEach((run, at) => { if (!joined || at < runs.length - 1) drawCell(run); });
        if (shown) drawCell({ period: joined || tail.period, cell });
        const liveCell = live?.cells[window.first + index];
        if (live && liveCell) {
          const rect = cellRect(live.period, index, visible.length, cellsFrame, frame.x[0], frame.x[1], state.playhead);
          if (rect) { ctx.fillStyle = tokens.surface; ctx.fillRect(rect.left, rect.top, rect.width, rect.height); }
          drawCell({ period: live.period, cell: liveCell });
        }
      });
    } }];
  }, [visible, periods, history, historyLast, props.tail, props.liveTail, props.look, props.loading, navigation.keyHours, window.offset, window.rowHeight]);
  const liveHead = useMemo<LiveHead | undefined>(() => {
    const head = props.liveHead, tail = props.liveTail;
    if (!head || !tail) return head;
    return { read: head.read, subscribe: changed => { const stops = [head.subscribe(changed), tail.subscribe(changed)]; return () => { for (const stop of stops) stop(); }; } };
  }, [props.liveHead, props.liveTail]);
  const rows = useMemo(() => ({ labels: visible.map(row => row.label), labelWidth,
    tooltip: (_row: number, hour: number) => {
      const index = keyboard.current ? selected.current : heatRowAt(pointerY.current, window, top, props.rows.length);
      const column = periodAt(periods, hour);
      return props.rows[index] ? props.tooltip(props.rows[index], column == null ? null : periods[column], hour) : null;
    } }), [visible, labelWidth, props.rows, periods, props.tooltip, window.first, window.offset, window.rowHeight, top, hoverRow]);
  const overlay = (ctx: CanvasRenderingContext2D, frame: Parameters<ChartLayer['draw']>[1], tokens: Parameters<ChartLayer['draw']>[2]) => {
    ctx.save(); ctx.beginPath(); ctx.rect(0, frame.plotTop, frame.plotLeft - VALUE_LABEL_GAP, frame.plotHeight); ctx.clip();
    ctx.clearRect(0, frame.plotTop, frame.plotLeft - VALUE_LABEL_GAP, frame.plotHeight);
    ctx.font = font(tokens, tokens.fontSize.small, 500); ctx.textAlign = 'right'; ctx.textBaseline = 'middle'; ctx.fillStyle = tokens.text;
    if (labelCache.current.font !== ctx.font) labelCache.current = { font: ctx.font, room: -1, key: '', labels: new Map(), fitted: new Map() };
    const cache = labelCache.current;
    const room = frame.plotLeft - VALUE_LABEL_GAP * 2;
    const visibleLabels = visible.map(row => row.label);
    const key = visibleLabels.join('\u0000');
    if (cache.room !== room || cache.key !== key) {
      cache.room = room; cache.key = key;
      cache.fitted = fitLabels(visibleLabels, room, text => {
        let width = cache.labels.get(text);
        if (width === undefined) { width = ctx.measureText(text).width; cache.labels.set(text, width); }
        return width;
      });
    }
    visible.forEach((row, index) => {
      ctx.fillText(cache.fitted.get(row.label) ?? '', frame.plotLeft - VALUE_LABEL_GAP,
        frame.plotTop + (index + 0.5) * window.rowHeight - window.offset);
    });
    ctx.restore();
  };
  return <div className="heat-chart"><div ref={surface} className="heat-chart-surface" onScroll={scroll}
    onKeyDownCapture={keyDown} onPointerMoveCapture={point} onPointerDownCapture={point}>
    <div className="heat-chart-spacer" style={{ height: window.total }}>
      <div className="heat-chart-viewport" style={{ height: viewport.height }}>
        <div className="heat-chart-plot" style={{ height: window.plotHeight }}><TimeChart label={props.label} extent={props.extent} playhead={props.playhead} liveHead={liveHead} playback="cursor" afterPlayhead={props.afterPlayhead}
          branchStart={props.branchStart} hoursPerSecond={props.hoursPerSecond} axes={{ xFrom: null, xTo: null, yMin: 0, yMax: Math.max(1, visible.length), log: false }}
          padding={{ top, bottom }} rows={rows} layers={layers} overlay={overlay} {...navigation}
          active={props.active} revision={props.revision} onContextMenu={props.onContextMenu} onSummaryView={props.onSummaryView}
          pickOwner={props.pickOwner} onPick={props.onPick ? hour => {
            const row = props.rows[keyboard.current ? selected.current : heatRowAt(pointerY.current, window, top, props.rows.length)];
            const column = periodAt(periods, hour);
            if (row) props.onPick!(row, column == null ? null : periods[column], hour);
          } : undefined} /></div>
      </div>
    </div>
  </div><Legend label={`${props.label} legend`} items={legend} always windowed /></div>;
}
