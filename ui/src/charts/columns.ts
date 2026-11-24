import type { ChartLayer } from './TimeChart';
import type { PlotFrame } from './frame';
import type { ChartTokens } from './tokens';
import type { Range } from './view';
import { fillHatched } from './hatch.ts';
import { stackEdges } from './stack.ts';
import type { Edge } from './stack';

export interface ColumnPeriod {
  firstHour: number;
  lastHour: number;
  values: readonly (number | null)[];
  uncertain?: readonly boolean[];
  unknown?: boolean;
}

export interface ColumnStyle {
  spacing?: 'pixel' | 'proportional';
  thinBands?: boolean;
  fullHeightUnknown?: boolean;
}

const GAP_FROM = 6;
const CAP_TILES = 3;
export const CAP_HEADROOM = 0.1;

export function columnAt(periods: readonly ColumnPeriod[], hour: number): number {
  let low = 0, high = periods.length - 1;
  while (low <= high) {
    const mid = (low + high) >> 1;
    if (hour < periods[mid].firstHour) high = mid - 1;
    else if (hour > periods[mid].lastHour) low = mid + 1;
    else return mid;
  }
  return -1;
}

export const columnStops = (periods: readonly ColumnPeriod[]) => periods.map(period => period.firstHour);

export function columnEdges(period: ColumnPeriod): (Edge | null)[] {
  const known = period.values.map(value => value ?? 0);
  const edges = stackEdges(known);
  return edges.map((edge, band) => period.values[band] == null ? null : edge);
}

const periodUnknown = (period: ColumnPeriod) => period.unknown === true || (period.values.length > 0 && period.values.every(value => value == null));

export function columnsExtent(periods: readonly ColumnPeriod[], x: Range): Range | null {
  let low = 0, high = 0, any = false, unknown = false;
  for (const period of periods) {
    if (period.lastHour < x[0] || period.firstHour > x[1] || periodUnknown(period)) continue;
    any = true;
    unknown ||= period.values.some(value => value == null) || period.uncertain?.some(Boolean) === true;
    for (const edge of columnEdges(period)) {
      if (!edge) continue;
      low = Math.min(low, edge.low); high = Math.max(high, edge.high);
    }
  }
  if (!any) return null;
  if (!unknown) return [low, high];
  const span = high - low;
  return [low, span > 0 ? high + span * CAP_HEADROOM : high + 1];
}

export function columnBox(period: Pick<ColumnPeriod, 'firstHour' | 'lastHour'>, frame: PlotFrame, spacing: ColumnStyle['spacing'] = 'pixel'): { left: number; width: number } | null {
  if (spacing === 'proportional') {
    const left = frame.toX(Math.max(period.firstHour, frame.x[0])), right = frame.toX(Math.min(period.lastHour + 1, frame.x[1]));
    const gap = Math.min(2, (right - left) * 0.15);
    const width = right - left - 2 * gap;
    return width > 0 ? { left: left + gap, width } : null;
  }
  const left = frame.toX(period.firstHour), right = frame.toX(period.lastHour + 1);
  const inset = right - left >= GAP_FROM ? 1 : 0;
  return { left: left + inset, width: Math.max(1, right - left - 2 * inset) };
}

export interface PreparedColumn {
  firstHour: number;
  lastHour: number;
  unknown: boolean;
  edges: readonly (Edge | null)[];
  zero: readonly boolean[];
  uncertain?: readonly boolean[];
  hasNull: boolean;
  low: number;
  high: number;
  any: boolean;
  hasUnknown: boolean;
}

export function prepareColumn(period: ColumnPeriod): PreparedColumn {
  const unknown = periodUnknown(period);
  if (unknown) {
    return {
      firstHour: period.firstHour, lastHour: period.lastHour, unknown: true,
      edges: [], zero: [], uncertain: period.uncertain, hasNull: false, low: 0, high: 0, any: false, hasUnknown: false,
    };
  }
  const values = period.values;
  const edges = stackEdges(values.map(value => value ?? 0)).map((edge, band) => values[band] == null ? null : edge);
  const zero = values.map(value => value === 0);
  const hasNull = values.some(value => value == null);
  const hasUnknown = hasNull || period.uncertain?.some(Boolean) === true;
  let low = 0, high = 0;
  for (const edge of edges) {
    if (!edge) continue;
    low = Math.min(low, edge.low); high = Math.max(high, edge.high);
  }
  return {
    firstHour: period.firstHour, lastHour: period.lastHour, unknown: false,
    edges, zero, uncertain: period.uncertain, hasNull, low, high, any: true, hasUnknown,
  };
}

function drawPreparedColumn(period: PreparedColumn, colours: readonly string[], style: ColumnStyle,
  ctx: CanvasRenderingContext2D, frame: PlotFrame, tokens: ChartTokens,
  left: number, width: number, zero: number, cap: number, unknownHeight: number): void {
  const hatch = (x: number, top: number, w: number, height: number) =>
    fillHatched(ctx, x, top, w, height, tokens.muted, tokens.hatchSpacing, tokens.lineWidth.grid);
  if (period.unknown) {
    hatch(left, frame.plotTop, width, unknownHeight);
    return;
  }
  let top = zero;
  let marked = false;
  period.edges.forEach((edge, band) => {
    if (!edge) return;
    const y0 = frame.toY(edge.high), y1 = frame.toY(edge.low);
    let height = y1 - y0;
    if (height < 1) {
      if (!style.thinBands || period.zero[band]) {
        marked ||= period.uncertain?.[band] === true;
        return;
      }
      height = 1;
    }
    ctx.fillStyle = colours[band % colours.length];
    ctx.fillRect(left, y0, width, height);
    if (period.uncertain?.[band]) hatch(left, y0, width, height);
    top = Math.min(top, y0);
  });
  if (marked) {
    const markTop = Math.max(frame.plotTop, top - tokens.hatchSpacing);
    hatch(left, markTop, width, top - markTop);
    top = markTop;
  }
  if (period.hasNull) {
    const capTop = Math.max(frame.plotTop, top - cap);
    hatch(left, capTop, width, top - capTop);
  }
}

const unknownColumnHeight = (frame: PlotFrame, style: ColumnStyle) => style.fullHeightUnknown
  ? frame.plotHeight
  : frame.toY(Math.max(frame.y[0], Math.min(frame.y[1], 0))) - frame.plotTop;

export function drawPreparedColumns(periods: readonly PreparedColumn[], colours: readonly string[], style: ColumnStyle,
  ctx: CanvasRenderingContext2D, frame: PlotFrame, tokens: ChartTokens, state: { playhead: number | null }): void {
  const until = Math.min(frame.x[1], state.playhead ?? Infinity);
  const zero = frame.toY(0);
  const cap = tokens.hatchSpacing * CAP_TILES;
  const unknownHeight = unknownColumnHeight(frame, style);
  for (const period of periods) {
    if (period.lastHour < frame.x[0] || period.firstHour > until) continue;
    const box = columnBox(period, frame, style.spacing);
    if (!box) continue;
    drawPreparedColumn(period, colours, style, ctx, frame, tokens, box.left, box.width, zero, cap, unknownHeight);
  }
}

export function columnsLayer(periods: readonly ColumnPeriod[], colours: readonly string[], style: ColumnStyle = {}): ChartLayer {
  return {
    draw(ctx: CanvasRenderingContext2D, frame: PlotFrame, tokens: ChartTokens, state: { playhead: number | null }) {
      const until = Math.min(frame.x[1], state.playhead ?? Infinity);
      const zero = frame.toY(0);
      const cap = tokens.hatchSpacing * CAP_TILES;
      const unknownHeight = unknownColumnHeight(frame, style);
      for (const period of periods) {
        if (period.lastHour < frame.x[0] || period.firstHour > until) continue;
        const box = columnBox(period, frame, style.spacing);
        if (!box) continue;
        drawPreparedColumn(prepareColumn(period), colours, style, ctx, frame, tokens, box.left, box.width, zero, cap, unknownHeight);
      }
    },
    extent: x => columnsExtent(periods, x),
  };
}
